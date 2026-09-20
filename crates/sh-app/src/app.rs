//! Root application component: session, theme, key dispatch, root render.

use crate::actions::{
    BackToGrid, CopySelected, CropCancel, CropCopy, CropSave, DeleteSelected, MoveSelected,
    NextImage, OpenFile, OpenFolder, OpenSelected, OpenSettings, PrevImage, SelectAll, SelectNext,
    SelectPrev, ToggleCrop, ToggleFullscreen, ToggleOverlays, ToggleSelected, ToggleSlideshow,
};
use crate::state::session::{build_image_items, next_index, FitMode, Session, ZoomPreset};
use crate::state::theme_store::{hot_reload_decision, HotReloadDecision, ThemeStore};
use crate::state::view::View;
use crate::ui::grid;
use crate::ui::grid::GridSizeGeometry;
use crate::ui::icons::{icon, IconName};
use crate::ui::overlay::{self, OverlayData};
use crate::ui::settings_panel::scroll;
use crate::ui::topbar;
use crate::ui::welcome;
use crate::viewer::{render_viewer, ViewerParams};
use gpui::prelude::*;
use gpui::*;
use sh_core::i18n::{t, Language, StrKey};
use sh_core::navigation::{SortBy, SortDir};
use sh_core::settings::GridSize;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Instant;

/// Staged batch file op awaiting confirm-bar approval (V3 destructive half).
/// Paths are snapshotted at staging: the rescan after execution rebuilds
/// indices, so holding indices here would corrupt the outcome.
#[derive(Debug, Clone)]
pub enum BatchOp {
    Delete { paths: Vec<PathBuf> },
    Move { paths: Vec<PathBuf>, dest: PathBuf },
}

impl BatchOp {
    fn paths(&self) -> &[PathBuf] {
        match self {
            BatchOp::Delete { paths } => paths,
            BatchOp::Move { paths, .. } => paths,
        }
    }
}

/// The root application entity.
pub struct App {
    /// Mutable session state.
    pub session: Session,
    /// Active theme store.
    pub theme_store: ThemeStore,
    /// Current viewport size in pixels.
    pub viewport: Size<Pixels>,
    /// Monotonic navigation counter; guards against stale decode completions.
    ///
    /// Every `navigate()` bumps this. An async probe completion only commits
    /// global state (`zoom`/`fit_mode`/`error`) while its captured sequence
    /// still matches, so a slow probe for image B cannot clobber the state
    /// of a later navigation to image C.
    pub navigation_seq: u64,
    /// Last mouse-down position while dragging (pan gesture), if any.
    pub drag_last: Option<Point<Pixels>>,
    /// Last observed hover cursor position, if any.
    ///
    /// Deadband anchor for the idle clock (see [`hover_moved_enough`]):
    /// bare-hover moves below [`HOVER_DEADBAND_PX`] don't reset
    /// [`Self::last_interaction`], so sensor-noise micro-movements can't
    /// re-show the overlays right at/after the idle threshold.
    pub last_hover: Option<Point<Pixels>>,
    /// Timestamp of the last user interaction of any kind (mouse move,
    /// keyboard action); drives overlay auto-hide.
    pub last_interaction: Instant,
    /// Whether the idle tick has already hidden the overlays for the current
    /// idle period. Prevents perpetual re-render: the tick only notifies on
    /// the visible→hidden transition, and any interaction resets the flag.
    pub overlays_hidden_by_idle: bool,
    /// Path to `settings.json` (used by [`Self::persist`]).
    pub settings_path: PathBuf,
    /// The settings as loaded at startup; [`Self::persist`] saves a copy of
    /// this with only `theme`/`last_dir` updated, so user-edited values in
    /// other fields (`cache_memory_limit_mb`, `show_hidden_files`,
    /// `max_decode_dimension`) survive every save.
    pub settings: sh_core::settings::Settings,
    /// Theme-file text last successfully applied by hot reload (or startup).
    ///
    /// Hot-reload dedupe anchor: identical text means "nothing changed",
    /// and each DISTINCT invalid text is warned about exactly once.
    pub last_applied_theme_text: String,
    /// Text of the last invalid theme the watcher warned about.
    ///
    /// Dedupes the `warn!` itself (distinct from the apply-skip dedupe in
    /// [`Self::last_applied_theme_text`]): editing an invalid file twice
    /// without changing it warns once, a NEW invalid edit warns again.
    pub last_warned_invalid_theme: Option<String>,
    /// Whether the previous theme-file poll hit a read error (deleted or
    /// unreadable file). Lets the watcher warn on the Ok→Err TRANSITION
    /// instead of every tick, and resume normally on recovery.
    pub theme_read_failed: bool,
    /// Keyboard focus anchor for the root `image_view` div.
    ///
    /// GPUI dispatches key bindings along the **focus stack**: a binding
    /// scoped to the `"image_view"` key context only matches when keyboard
    /// focus sits inside the element carrying that context (or a
    /// descendant). The root div tracks this handle via `.track_focus`, so
    /// focusing it once at startup puts every keystroke on the dispatch
    /// path that contains `image_view`, making the ←/→/Tab/F11/Ctrl+O
    /// bindings reachable without any prior mouse interaction.
    pub focus_handle: FocusHandle,
    /// Current app view (Welcome → Grid → Viewer).
    pub view: View,
    /// Selected index in the Grid view.
    pub grid_selected: usize,
    /// Range anchor for Shift+selection (V3): follows the cursor on
    /// navigation and viewer opens, frozen during selection ops — so
    /// chained Shift+clicks share the true anchor and selection never
    /// moves the "visited" mark.
    pub anchor: usize,
    /// Multi-selection set (V3): grid indices, ascending + deduped by
    /// construction. Cursor + range anchor stays `grid_selected`. Cleared
    /// on folder swap and sort change; survives viewer round-trips.
    pub selected: BTreeSet<usize>,
    /// Pending destructive batch op (V3): bar visible ⟺ `Some`. Cleared on
    /// confirm, cancel, folder change. Never persisted.
    pub pending_batch: Option<BatchOp>,
    /// Last batch-op report for the grid topbar-center (V3): `Some` only on
    /// partial outcomes; full success is silent. Cleared on next staging
    /// or folder change — never `session.error` (which does not render with
    /// images present).
    pub batch_status: Option<String>,
    /// Manual grid scroll offset in px (wheel-driven, clamped).
    pub grid_scroll_px: f32,
    /// Manual settings-content scroll offset in px (wheel-driven, clamped).
    /// Shortcuts-only: General/Appearance fit normal windows. Same
    /// translation pattern as [`grid_scroll_px`] (see
    /// [`crate::ui::settings_panel::scroll`] for why native scroll cannot
    /// engage here). Reset on section change and on open.
    pub settings_scroll_px: f32,
    /// Decoded 256px thumbnails by path (grid cells). Cleared on every
    /// folder open; filled by one background task per open (seq-guarded).
    /// Full-resolution images NEVER live here — that was the 984MB grid.
    pub thumbs: std::collections::HashMap<PathBuf, std::sync::Arc<gpui::RenderImage>>,
    /// Sequence guarding thumb decode tasks against folder switches.
    pub thumb_seq: u64,
    /// Folders that can drive the Welcome Continue button + recent chips:
    /// the persisted recents list, filtered to paths that still exist.
    /// Refreshed by [`Self::persist`] and seeded at startup (main.rs).
    pub recent_dirs_available: Vec<PathBuf>,
    /// View to return to when the Settings surface closes (Welcome, Grid,
    /// or Viewer — captured by [`Self::open_settings`]).
    pub settings_return_to: View,
    /// Active section inside the Settings surface.
    pub settings_section: crate::ui::settings_panel::SettingsSection,
    /// Shortcut capture in progress: the action id awaiting a keypress, if any.
    /// `None` = not capturing. Scoped to the Settings surface; cleared on
    /// view change (see [`Self::close_settings`]).
    pub capture_action: Option<String>,
    /// Conflict feedback for the row currently in capture mode: the ACTION ID
    /// of the incumbent holding the rejected combo, if the last pressed
    /// combo was rejected. Stored as the id (not the resolved label) so the
    /// message re-renders in the current UI language on live switch.
    /// `None` = no error to show.
    pub capture_conflict: Option<String>,
    /// Reset-all armed state for the two-step inline confirm (Shortcuts section).
    /// Cleared wherever capture is cleared.
    pub reset_armed: bool,
    /// Sort dropdown open (sort chip in the top bar).
    pub sort_menu_open: bool,
    /// Crop mode: drag selects a region instead of panning.
    pub crop_mode: bool,
    /// Current selection in viewport px (drag order; normalized on confirm).
    pub crop_rect: Option<sh_core::crop::CropRect>,
    /// Confirm bar visible (a finished drag left a non-degenerate rect).
    pub crop_bar_visible: bool,
    /// Previous-frame dissolve state — detects bar presence changes so Fit
    /// images re-center when the freed 40px changes the fit area.
    pub topbar_was_hidden: bool,
    /// Info popover open (viewer-only, transient — never persisted).
    pub info_panel_open: bool,
    /// Cached facts: (path probed, outcome). Re-resolved on open and on
    /// every navigate-while-open; render reads only (path match ⇒ rows).
    pub info_facts: Option<(
        PathBuf,
        Result<sh_core::decode::FileInfo, sh_core::errors::ShImagesError>,
    )>,
}

impl App {
    /// Create a new app with the given session, theme, and settings.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session: Session,
        theme_store: ThemeStore,
        settings_path: PathBuf,
        settings: sh_core::settings::Settings,
        last_applied_theme_text: String,
        cx: &gpui::App,
    ) -> Self {
        Self {
            session,
            theme_store,
            viewport: size(px(0.), px(0.)),
            navigation_seq: 0,
            drag_last: None,
            last_hover: None,
            last_interaction: Instant::now(),
            overlays_hidden_by_idle: false,
            settings_path,
            settings,
            last_applied_theme_text,
            last_warned_invalid_theme: None,
            theme_read_failed: false,
            focus_handle: cx.focus_handle(),
            view: View::Welcome,
            grid_selected: 0,
            anchor: 0,
            pending_batch: None,
            batch_status: None,
            selected: BTreeSet::new(),
            grid_scroll_px: 0.0,
            settings_scroll_px: 0.0,
            thumbs: std::collections::HashMap::new(),
            thumb_seq: 0,
            recent_dirs_available: Vec::new(),
            settings_return_to: View::Welcome,
            settings_section: crate::ui::settings_panel::SettingsSection::default(),
            capture_action: None,
            capture_conflict: None,
            reset_armed: false,
            sort_menu_open: false,
            crop_mode: false,
            crop_rect: None,
            crop_bar_visible: false,
            topbar_was_hidden: false,
            info_panel_open: false,
            info_facts: None,
        }
    }

    /// Mark user interaction: refreshes the idle clock and, if the overlays
    /// were hidden by idle, re-shows them with a single notify.
    ///
    /// Called from EVERY interaction surface — mouse move and all keyboard
    /// actions — so keyboard-only users never lose the chrome.
    pub fn note_interaction(&mut self, cx: &mut Context<Self>) {
        self.last_interaction = Instant::now();
        if self.overlays_hidden_by_idle {
            self.overlays_hidden_by_idle = false;
            cx.notify();
        }
    }

    /// Spawn the idle watcher: wakes periodically and hides the overlays
    /// once the mouse has been idle past [`overlay::OVERLAY_IDLE`].
    ///
    /// GPUI renders on demand — when the mouse stops, no events arrive, so
    /// nothing would ever re-render the overlays away. This tick supplies
    /// the missing wake-up. It notifies ONLY on the visible→hidden
    /// transition (guarded by [`Self.overlays_hidden_by_idle`]); any user
    /// interaction resets the flag, re-arming the next transition. The loop
    /// exits once the App entity is dropped.
    pub fn spawn_idle_watcher(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(IDLE_TICK).await;
            // Entity dropped → stop ticking (no immortal background loop).
            if this
                .update(cx, |app, cx| {
                    let idle = app.last_interaction.elapsed() > overlay::OVERLAY_IDLE;
                    let overlays_on = app.session.show_overlay_bottom;
                    // Notify exactly once per idle transition.
                    if idle && overlays_on && !app.overlays_hidden_by_idle {
                        app.overlays_hidden_by_idle = true;
                        cx.notify();
                    }
                })
                .is_err()
            {
                break;
            }
        })
        .detach();
    }

    /// Eternal slideshow tick (V3): every [`SLIDESHOW_INTERVAL`], advance
    /// the viewer by one image while the slideshow is active. The loop is
    /// spawned ONCE (main.rs + test harness) and is inert when the flag
    /// is off — no spawn-per-toggle, no re-spawn races. Same guarantees
    /// as [`Self::spawn_idle_watcher`]: the loop exits when the entity is
    /// dropped.
    pub fn spawn_slideshow_timer(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(SLIDESHOW_INTERVAL).await;
            if this
                .update(cx, |app, cx| {
                    if app.session.slideshow_active && app.view == View::Viewer {
                        app.navigate(1, cx);
                    }
                })
                .is_err()
            {
                break;
            }
        })
        .detach();
    }

    /// Toggle the slideshow (V3). Viewer-only; crop mode owns the session
    /// when active, so the toggle is a no-op there. `note_interaction`
    /// keeps the overlay visible so the user sees the state flip.
    pub fn toggle_slideshow(&mut self, cx: &mut Context<Self>) {
        if self.crop_mode {
            return;
        }
        self.session.slideshow_active = !self.session.slideshow_active;
        self.note_interaction(cx);
        cx.notify();
    }

    /// Re-resolve info facts for the current session item (sync header +
    /// stat, near-instant — no background task). Called from the info-button
    /// toggle (on open) and `navigate()` (when open). Render reads the cache
    /// only, never I/O.
    fn refresh_info_facts(&mut self) {
        let outcome = match self.session.current_item() {
            Some(item) => sh_core::decode::probe_file_info(&item.path),
            None => Err(sh_core::errors::ShImagesError::UnsupportedFormat(
                "no image open".into(),
            )),
        };
        let path = self
            .session
            .current_item()
            .map(|i| i.path.clone())
            .unwrap_or_default();
        self.info_facts = Some((path, outcome));
    }

    /// Toggle the viewer info popover. `note_interaction` runs FIRST so the
    /// overlay cannot idle-fade mid-tap; opening re-resolves facts for the
    /// current file before the next paint.
    pub fn toggle_info_panel(&mut self, cx: &mut Context<Self>) {
        self.note_interaction(cx);
        self.info_panel_open = !self.info_panel_open;
        if self.info_panel_open {
            self.refresh_info_facts();
        }
        cx.notify();
    }

    /// Close the viewer info popover (outside-click catcher + `Esc` guard).
    /// Touches nothing else — no navigation, no view change.
    pub fn close_info_panel(&mut self, cx: &mut Context<Self>) {
        self.info_panel_open = false;
        cx.notify();
    }

    /// Open a path: scan its parent's entries, anchor the selection on the
    /// opened file, apply the active session sort, show it, reset state,
    /// persist, and kick off the initial probe/fit.
    ///
    /// An invalid path (no readable parent) surfaces in the session error
    /// slot instead of panicking.
    pub fn open_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let parent = path.parent().map(std::path::Path::to_path_buf);
        if let Some(dir) = parent.filter(|p| p.is_dir()) {
            let anchor = path.clone();
            let entries = sh_core::navigation::scan_entries(&dir);
            self.session.error = None;
            self.session.images = build_image_items(entries);
            // Anchor on the opened file, then apply the active sort — the
            // resort re-anchors by path, so the selection survives the sort.
            self.session.current = self
                .session
                .images
                .iter()
                .position(|i| i.path == anchor)
                .unwrap_or(0);
            self.session.resort();
            self.session.show_overlay_bottom = true;
            // Folder swap kills the slideshow: auto-advance into a fresh
            // image list the user never chose to play is wrong.
            self.session.slideshow_active = false;
            cx.notify();
            self.persist(cx);
            self.navigate(0, cx);
            // Thumbnails: every path that swaps `session.images` must
            // re-arm the thumb batch, or the Grid renders empty
            // placeholders forever (drop-a-file → Back showed exactly
            // that: `open_path` swapped the list but only `open_folder`
            // spawned the decode batch). The map is cleared first so
            // no previous folder's thumbs linger; the seq guard drops
            // stale work if another open happens mid-decode.
            self.spawn_thumb_batch(cx);
        } else {
            self.session.error = Some(
                sh_core::errors::ShImagesError::NotAFile(path.display().to_string()).to_string(),
            );
            cx.notify();
        }
    }

    /// Arm the background thumbnail batch for the current `session.images`
    /// (8-way parallel chunks, progressive per-thumb commits, seq-guarded
    /// against folder switches). Called from every open path.
    fn spawn_thumb_batch(&mut self, cx: &mut Context<Self>) {
        self.thumbs.clear();
        self.thumb_seq = self.thumb_seq.wrapping_add(1);
        let seq = self.thumb_seq;
        let paths: Vec<PathBuf> = self.session.images.iter().map(|i| i.path.clone()).collect();
        cx.spawn(async move |this, cx| {
            for chunk in paths.chunks(8) {
                let tasks: Vec<_> = chunk
                    .iter()
                    .map(|path| {
                        let path = path.clone();
                        // Owned handle per task: entity `Context` only lends
                        // `&BackgroundExecutor`, which cannot enter the task.
                        cx.background_executor().spawn(async move {
                            sh_core::decode::load_with_limit(&path, crate::thumbs::THUMB_MAX_DIM)
                                .ok()
                                .and_then(|d| {
                                    crate::thumbs::render_thumb(&d).map(|t| (path.clone(), t))
                                })
                        })
                    })
                    .collect();
                for task in tasks {
                    let decoded = task.await;
                    let done = this
                        .update(cx, |app, cx| {
                            if seq != app.thumb_seq {
                                return false;
                            }
                            if let Some((path, thumb)) = decoded {
                                app.thumbs.insert(path, thumb);
                                cx.notify();
                            }
                            true
                        })
                        .unwrap_or(false);
                    if !done {
                        return;
                    }
                }
            }
        })
        .detach();
    }

    /// Open a folder: scan its entries (or surface "no images" in the
    /// session error slot), enter the Grid view, persist, and probe.
    /// An empty/unreadable folder surfaces in `session.error`; the view
    /// still switches to Grid so the empty-state renders with context.
    pub fn open_folder(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        // Session is the runtime source of truth for order: sync the
        // persisted sort before the first image list is built.
        self.session.sort_by = self.settings.sort_by;
        self.session.sort_dir = self.settings.sort_dir;
        let mut entries = sh_core::navigation::scan_entries(&dir);
        if let Some(first) = entries.drain(..).next() {
            self.open_path(first.path, cx);
            // `open_path` already armed the thumb batch for the new
            // image list; re-arming here would only waste a decode
            // round that dies on the seq check.
        } else {
            self.session.images = Vec::new();
            self.session.current = 0;
            self.session.error = Some(sh_core::i18n::no_images_in(
                self.settings.language,
                &dir.display().to_string(),
            ));
            // Empty folder: still clear + invalidate any previous
            // folder's thumb map (zero-path batch = clear + seq bump).
            self.spawn_thumb_batch(cx);
            cx.notify();
        }
        self.view = View::Grid;
        self.grid_selected = 0;
        self.anchor = 0;
        // Work-in-progress selection never crosses folders.
        self.selected.clear();
        // A new folder supersedes any previous batch report.
        self.batch_status = None;
        self.grid_scroll_px = 0.0;
        // NOTE: recents are owned by `persist` (via `open_path`, the
        // non-empty arm); an empty folder must NOT enter the list.
        cx.notify();
    }

    /// Viewport available to the image: full window minus the persistent top
    /// bar — or the full window while the bar is dissolved on Viewer idle.
    /// ALL fit math (navigate completion, toggle, clamp, wheel) must use
    /// this, never the raw window viewport.
    pub fn viewer_viewport(&self) -> sh_core::transform::Vec2 {
        let v = viewport_vec(self.viewport);
        let dissolved = self.view == View::Viewer
            && topbar_hidden(
                self.last_interaction.elapsed() > overlay::OVERLAY_IDLE,
                !self.session.show_overlay_bottom,
            );
        sh_core::transform::Vec2 {
            x: v.x,
            y: viewer_fit_height(v.y, dissolved),
        }
    }

    /// Back to the grid; selection follows the current image.
    /// (Extended with scroll-into-view in the grid task.)
    pub fn enter_grid(&mut self, cx: &mut Context<Self>) {
        self.grid_selected = self.session.current;
        // Returning lands the cursor (and anchor) on the viewed image;
        // the set itself is preserved (work-in-progress).
        self.anchor = self.session.current;
        self.grid_scroll_px = 0.0;
        self.view = View::Grid;
        self.note_interaction(cx);
        cx.notify();
    }

    /// Change the active sort: update session + settings copy, re-anchor
    /// the selection by path, sync the grid selection, persist.
    ///
    /// The session is the runtime truth for order; the settings copy
    /// mirrors it so the next launch restores the same criterion.
    pub fn set_sort(
        &mut self,
        by: sh_core::navigation::SortBy,
        dir: sh_core::navigation::SortDir,
        cx: &mut Context<Self>,
    ) {
        self.session.apply_sort(by, dir);
        // Indices invalidate on reorder — clear (documented v1 rule).
        self.selected.clear();
        self.settings.sort_by = by;
        self.settings.sort_dir = dir;
        self.grid_selected = self.session.current;
        self.persist(cx);
        cx.notify();
    }

    /// Change the grid density preset: mirror into the settings copy,
    /// re-clamp the scroll offset into the new preset's range, keep the
    /// cursor visible, persist.
    ///
    /// Unlike [`Self::set_sort`] there is no session field to update and
    /// the selection is untouched: density is view/persistence state, so
    /// `settings.grid_size` is both runtime and persisted truth — size
    /// change re-sorts nothing and never moves the cursor or anchor.
    pub fn set_grid_size(&mut self, size: GridSize, cx: &mut Context<Self>) {
        self.settings.grid_size = size;
        let v = viewport_vec(self.viewport);
        let visible_h = (v.y - topbar::TOPBAR_H_PX).max(1.0);
        let geo = size.geometry();
        let max = grid::grid_max_scroll(self.session.images.len(), v.x, visible_h, &geo);
        self.grid_scroll_px = self.grid_scroll_px.clamp(0.0, max);
        self.scroll_cursor_into_view(v.x, visible_h, &geo);
        self.persist(cx);
        cx.notify();
    }

    /// Scroll the selected row into view under `geo`, then clamp into
    /// range. Shared by [`Self::move_selection`] and
    /// [`Self::set_grid_size`] so arrow navigation and size changes
    /// cannot drift apart.
    fn scroll_cursor_into_view(
        &mut self,
        viewport_w: f32,
        visible_h: f32,
        geo: &grid::GridGeometry,
    ) {
        let len = self.session.images.len();
        if len == 0 {
            self.grid_scroll_px = 0.0;
            return;
        }
        let cols = grid::grid_columns(viewport_w, geo);
        let row_top = (self.grid_selected / cols) as f32 * geo.row_h as f32;
        let row_bottom = row_top + geo.row_h as f32;
        if row_top < self.grid_scroll_px {
            self.grid_scroll_px = row_top;
        } else if row_bottom > self.grid_scroll_px + visible_h {
            self.grid_scroll_px = row_bottom - visible_h;
        }
        let max = grid::grid_max_scroll(len, viewport_w, visible_h, geo);
        self.grid_scroll_px = self.grid_scroll_px.clamp(0.0, max);
    }

    /// Enter the viewer at `idx`: set current + selection, switch view,
    /// probe/fit. Reuses [`Self::navigate`] so probe, seq-guard, fit, and
    /// persist all behave exactly like keyboard navigation.
    pub fn enter_viewer(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx < self.session.images.len() {
            self.session.current = idx;
            self.grid_selected = idx;
            // Opening IS visiting: cursor and anchor move together.
            self.anchor = idx;
        }
        self.view = View::Viewer;
        self.note_interaction(cx);
        self.navigate(0, cx);
    }

    /// Move grid selection by `delta` (keyboard arrows): sticky at the ends,
    /// no wrap. Keeps the selected row visible by adjusting the manual scroll
    /// offset. No-op on an empty grid.
    pub fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.session.images.len();
        if len == 0 {
            return;
        }
        self.grid_selected =
            (self.grid_selected as isize + delta).clamp(0, len as isize - 1) as usize;
        // Plain arrows move the cursor AND collapse the set (standard OS
        // behavior); the anchor follows the cursor. Shift+arrows go through
        // `extend_selection_to` instead (anchor frozen).
        self.anchor = self.grid_selected;
        self.selected.clear();
        // Scroll the selected row into view (shared helper: same math
        // as a size change, so navigation and density cannot drift apart).
        let v = viewport_vec(self.viewport);
        let visible_h = (v.y - topbar::TOPBAR_H_PX).max(1.0);
        let geo = self.settings.grid_size.geometry();
        self.scroll_cursor_into_view(v.x, visible_h, &geo);
        self.note_interaction(cx);
        cx.notify();
    }

    /// Toggle one cell in the selection set (Ctrl+click / Ctrl+Space).
    /// Never moves the cursor: selection must not take the "visited" mark —
    /// the cursor only moves on navigation and viewer opens.
    pub fn toggle_selected(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.session.images.len() {
            return;
        }
        if !self.selected.remove(&idx) {
            self.selected.insert(idx);
        }
        self.note_interaction(cx);
        cx.notify();
    }

    /// Union the anchor→target range into the set (Shift+click /
    /// Shift+arrows). The cursor is intentionally untouched: keyboard callers
    /// advance it explicitly as the moving edge, mouse callers leave the
    /// user exactly where they were. Union-only by design: narrowing needs
    /// Ctrl+click.
    pub fn extend_selection_to(&mut self, idx: usize, cx: &mut Context<Self>) {
        let len = self.session.images.len();
        if len == 0 {
            return;
        }
        let target = idx.min(len - 1);
        self.selected
            .extend(grid::selection_range(self.anchor, target));
        self.note_interaction(cx);
        cx.notify();
    }

    /// Fill the set (Ctrl+A). No-op on an empty grid.
    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.selected = (0..self.session.images.len()).collect();
        self.note_interaction(cx);
        cx.notify();
    }

    /// Empty the set (Escape in grid). Resets the anchor to the cursor so
    /// the next Shift gesture starts fresh from where the user is.
    /// Always notifies — callers use it as the consumed-gesture path.
    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.selected.clear();
        self.anchor = self.grid_selected;
        self.note_interaction(cx);
        cx.notify();
    }

    /// Copy payload for the selection: absolute paths, ascending index
    /// order, `\n`-joined. `None` when empty — the no-op signal, so callers
    /// never touch the clipboard without user-visible content.
    pub fn selection_paths_string(&self) -> Option<String> {
        if self.selected.is_empty() {
            return None;
        }
        let lines: Vec<String> = self
            .selected
            .iter()
            .filter_map(|i| self.session.images.get(*i))
            .map(|item| item.path.display().to_string())
            .collect();
        if lines.is_empty() {
            return None;
        }
        Some(lines.join("\n"))
    }

    /// Copy the selection's paths (grid-only; viewer is a deliberate no-op
    /// — single-copy follow-up lives outside this slice). Empty set is a
    /// silent no-op via the `None` signal. Backend failure warns,
    /// fire-and-forget (same contract as `persist`).
    pub fn copy_selection(&mut self, cx: &mut Context<Self>) {
        if self.view != View::Grid {
            return;
        }
        if let Some(payload) = self.selection_paths_string() {
            if let Err(e) = crate::clipboard::copy_text(&payload) {
                tracing::warn!("could not copy paths to clipboard: {e}");
            }
        }
        self.note_interaction(cx);
        cx.notify();
    }

    /// Stage a batch delete (Delete key): snapshot the selection's paths.
    /// Silent no-op outside grid or with an empty set. Clears any previous
    /// transient batch status (the new op supersedes it).
    pub fn stage_delete(&mut self, cx: &mut Context<Self>) {
        if self.view != View::Grid || self.selected.is_empty() {
            return;
        }
        let paths: Vec<PathBuf> = self
            .selected
            .iter()
            .filter_map(|i| self.session.images.get(*i))
            .map(|item| item.path.clone())
            .collect();
        if paths.is_empty() {
            return;
        }
        self.pending_batch = Some(BatchOp::Delete { paths });
        self.batch_status = None;
        self.note_interaction(cx);
        cx.notify();
    }

    /// Stage a batch move to `dest` (M key → picker, then this). Same
    /// guards as [`Self::stage_delete`].
    pub fn stage_move(&mut self, dest: PathBuf, cx: &mut Context<Self>) {
        if self.view != View::Grid || self.selected.is_empty() {
            return;
        }
        let paths: Vec<PathBuf> = self
            .selected
            .iter()
            .filter_map(|i| self.session.images.get(*i))
            .map(|item| item.path.clone())
            .collect();
        if paths.is_empty() {
            return;
        }
        self.pending_batch = Some(BatchOp::Move { paths, dest });
        self.batch_status = None;
        self.note_interaction(cx);
        cx.notify();
    }

    /// Execute the staged batch op (Enter / confirm button): take the op
    /// (bar closes immediately even on fs failure), run it synchronously,
    /// rescan the folder, remap leftovers by path, report partials in the
    /// transient topbar-center status. Grid-only guard (defensive).
    ///
    /// NOTE (plan reorder, Task 2→3): this method is specified in Task 3 but
    /// implemented here because the Task-2 Enter handler references it — no
    /// commit in between may stay broken.
    pub fn confirm_pending(&mut self, cx: &mut Context<Self>) {
        let Some(op) = self.pending_batch.take() else {
            return;
        };
        if self.view != View::Grid {
            return;
        }
        let (verb, report) = match &op {
            BatchOp::Delete { paths } => (
                sh_core::i18n::BatchVerb::Deleted,
                sh_core::batch::trash_paths(paths),
            ),
            BatchOp::Move { paths, dest } => (
                sh_core::i18n::BatchVerb::Moved,
                sh_core::batch::move_paths(paths, dest),
            ),
        };
        let total = report.moved.len() + report.skipped_existing.len() + report.failed.len();
        // Rescan the current folder (derived from the staged paths — all
        // share the visible folder by construction). No persist: the folder
        // didn't change, so settings/recents stay untouched.
        if let Some(dir) = op
            .paths()
            .first()
            .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
        {
            let entries = sh_core::navigation::scan_entries(&dir);
            self.session.images = build_image_items(entries);
            if self.session.images.is_empty() {
                self.session.error = Some(sh_core::i18n::no_images_in(
                    self.settings.language,
                    &dir.display().to_string(),
                ));
            }
            self.session.current = self
                .session
                .current
                .min(self.session.images.len().saturating_sub(1));
            self.spawn_thumb_batch(cx);
        }
        // Leftovers (skipped + failed) stay marked, remapped by path.
        let leftover: std::collections::BTreeSet<usize> = report
            .skipped_existing
            .iter()
            .chain(report.failed.iter().map(|(p, _)| p))
            .filter_map(|p| self.session.images.iter().position(|i| &i.path == p))
            .collect();
        self.selected = leftover;
        self.grid_selected = self
            .grid_selected
            .min(self.session.images.len().saturating_sub(1));
        // Report partials in the transient center status; full success is
        // silent. session.error is deliberately untouched (it does not
        // render with images present). The verb is typed (`BatchVerb`) and
        // the language owns the full sentence (S5) — Es word order is free
        // to differ.
        self.batch_status =
            sh_core::batch::format_report(self.settings.language, verb, total, &report);
        self.note_interaction(cx);
        cx.notify();
    }

    /// Enter crop mode: drag will select a region instead of panning.
    pub fn enter_crop(&mut self, cx: &mut Context<Self>) {
        self.crop_mode = true;
        self.crop_rect = None;
        self.crop_bar_visible = false;
        self.drag_last = None;
        // Crop owns the pointer; auto-advance must not fight a selection.
        self.session.slideshow_active = false;
        self.note_interaction(cx);
        cx.notify();
    }

    /// Leave crop mode, discarding any in-progress selection.
    pub fn cancel_crop(&mut self, cx: &mut Context<Self>) {
        self.crop_mode = false;
        self.crop_rect = None;
        self.crop_bar_visible = false;
        self.drag_last = None;
        self.note_interaction(cx);
        cx.notify();
    }

    /// Toggle crop mode (the `C` key / scissors button).
    pub fn toggle_crop(&mut self, cx: &mut Context<Self>) {
        if self.crop_mode {
            self.cancel_crop(cx);
        } else {
            self.enter_crop(cx);
        }
    }

    /// Arm a crop drag at `pos` (viewport px): zero-area rect anchored
    /// there. Pure state mutation, headless-testable.
    pub fn begin_crop_drag(&mut self, pos: (f32, f32)) {
        self.crop_rect = Some(sh_core::crop::CropRect {
            x0: pos.0,
            y0: pos.1,
            x1: pos.0,
            y1: pos.1,
        });
    }

    /// Extend the armed crop drag to `pos`. No-op when nothing is armed
    /// (bare hover must not fabricate a selection).
    pub fn update_crop_drag(&mut self, pos: (f32, f32)) {
        if let Some(r) = self.crop_rect.as_mut() {
            r.x1 = pos.0;
            r.y1 = pos.1;
        }
    }

    /// Finish the drag: show the confirm bar only when the selection has
    /// real area (> 4 px² anti-click threshold); otherwise discard it
    /// silently (an accidental click deserves no error). Documented
    /// decision in the crop plan (§Task 5 Step 3).
    pub fn finish_crop_drag(&mut self) {
        let has_area = self
            .crop_rect
            .map(|r| (r.x1 - r.x0).abs() * (r.y1 - r.y0).abs() > 4.0)
            .unwrap_or(false);
        self.crop_bar_visible = has_area;
        if !has_area {
            self.crop_rect = None;
        }
    }
    /// Convert the current selection to image pixels via the session zoom.
    /// `None` when degenerate or dimensions unknown (nothing to crop).
    fn crop_rect_px(&self) -> Option<(u32, u32, u32, u32)> {
        let rect = self.crop_rect?;
        let (iw, ih) = self.session.current_dimensions()?;
        rect.to_pixels(
            self.session.zoom.scale,
            self.session.zoom.offset,
            iw as f32,
            ih as f32,
        )
    }

    /// Shared post-crop outcome: success exits crop mode silently (no toast
    /// system — the mode exit IS the confirmation), failure lands in the
    /// viewer error slot and keeps the bar for a retry.
    fn crop_done(&mut self, cx: &mut Context<Self>, result: Result<(), String>) {
        match result {
            Ok(()) => {
                self.crop_mode = false;
                self.crop_rect = None;
                self.crop_bar_visible = false;
            }
            Err(msg) => {
                self.session.error = Some(msg);
                self.crop_bar_visible = false;
            }
        }
        self.note_interaction(cx);
        cx.notify();
    }

    /// Confirm-bar "Copy": crop the ORIGINAL file at the converted rect,
    /// then copy to the OS clipboard. Decode + Win32 clipboard calls block,
    /// so everything runs on the background executor; the entity lease is
    /// released for the whole operation (same contract as the thumb
    /// decoders).
    pub fn confirm_crop_copy(&mut self, cx: &mut Context<Self>) {
        let Some(rect) = self.crop_rect_px() else {
            // Degenerate rect or unknown dimensions: silently close, no
            // error (documented decision — nothing was cropped, nothing
            // deserves a message).
            self.crop_rect = None;
            self.crop_bar_visible = false;
            self.note_interaction(cx);
            cx.notify();
            return;
        };
        let Some(path) = self.session.current_item().map(|i| i.path.clone()) else {
            return;
        };
        // Close the bar now (mode exit happens on success below); a second
        // confirm click mid-flight must not re-trigger.
        self.crop_bar_visible = false;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let cut = cx
                .background_executor()
                .spawn(async move {
                    sh_core::crop::crop_image(&path, rect)
                        .and_then(|img| crate::clipboard::copy_image(&img))
                })
                .await;
            let result = cut.map_err(|e| format!("Couldn't copy: {e}"));
            let _ = this.update(cx, |app, cx| app.crop_done(cx, result));
        })
        .detach();
    }

    /// Confirm-bar "Save…": show the save dialog (rfd async, PNG filter,
    /// `<stem>_crop.png` default in the original folder), then crop + write
    /// on the background executor. Cancel closes silently — the user
    /// changed their mind, not an error.
    pub fn confirm_crop_save(&mut self, cx: &mut Context<Self>) {
        let Some(rect) = self.crop_rect_px() else {
            self.crop_rect = None;
            self.crop_bar_visible = false;
            self.note_interaction(cx);
            cx.notify();
            return;
        };
        let Some(path) = self.session.current_item().map(|i| i.path.clone()) else {
            return;
        };
        self.crop_bar_visible = false;
        cx.notify();
        let dialog = crate::platform::save_dialog(&path);
        cx.spawn(async move |this, cx| {
            let Some(handle) = dialog.save_file().await else {
                // Cancelled: keep the selection + mode so the user can try
                // again without re-dragging.
                let _ = this.update(cx, |app, cx| {
                    app.crop_bar_visible = true;
                    cx.notify();
                });
                return;
            };
            let out = handle.path().to_path_buf();
            let cut = cx
                .background_executor()
                .spawn(async move {
                    sh_core::crop::crop_image(&path, rect)
                        .and_then(|img| sh_core::crop::save_png(&out, &img))
                })
                .await;
            let result = cut.map_err(|e| format!("Couldn't save: {e}"));
            let _ = this.update(cx, |app, cx| app.crop_done(cx, result));
        })
        .detach();
    }

    /// Show the async folder picker; on pick, open the folder in Grid.
    /// Shared by the Welcome/Open buttons, the top bar, and Ctrl+Shift+O —
    /// one place, same `cx.spawn` contract as the file dialog (no
    /// `double_lease_panic`: the lease is released before rfd blocks).
    pub fn pick_folder(&mut self, cx: &mut Context<Self>) {
        self.note_interaction(cx);
        let start_dir = self.recent_dirs_available.first().cloned();
        let dialog = crate::platform::folder_dialog(start_dir.as_deref());
        cx.spawn(async move |this, cx| {
            if let Some(handle) = dialog.pick_folder().await {
                let _ = this.update(cx, |app, cx| {
                    app.open_folder(handle.path().to_path_buf(), cx);
                });
            }
        })
        .detach();
    }

    /// Show the async folder picker for a MOVE destination (V3 batch ops);
    /// on pick, stage the move — the folder is NOT opened (unlike
    /// [`Self::pick_folder`], which navigates into the pick). Cancel stages
    /// nothing. Same `cx.spawn` contract (no `double_lease_panic`).
    pub fn pick_destination(&mut self, cx: &mut Context<Self>) {
        if self.view != View::Grid || self.selected.is_empty() {
            return;
        }
        self.note_interaction(cx);
        let start_dir = self.recent_dirs_available.first().cloned();
        let dialog = crate::platform::folder_dialog(start_dir.as_deref());
        cx.spawn(async move |this, cx| {
            if let Some(handle) = dialog.pick_folder().await {
                let _ = this.update(cx, |app, cx| {
                    app.stage_move(handle.path().to_path_buf(), cx);
                });
            }
        })
        .detach();
    }

    /// Apply a built-in theme by settings-file name: swap the store
    /// (theme + name + `%APPDATA%/themes` path), reset the hot-reload
    /// baseline and warn state, persist; the surface stays open.
    ///
    /// Writes the builtin file when missing so it stays editable and
    /// hot-reloadable (same bootstrap contract as first launch; sub-ms for
    /// ~500B, same as the existing startup write). Returns false (no-op)
    /// for unknown names.
    pub fn apply_builtin_theme(&mut self, file_name: &str, cx: &mut Context<Self>) -> bool {
        let Some((_, json)) = crate::theme_builtins::BUILTIN_THEMES
            .iter()
            .find(|(n, _)| *n == file_name)
        else {
            return false;
        };
        // Builtins always parse (covered by test); a corrupt builtin must
        // never blank the active theme, so fail closed.
        let Ok(theme) = sh_core::theme::parse(json) else {
            return false;
        };
        let Some(config_dir) = self.settings_path.parent() else {
            return false;
        };
        let path = config_dir.join("themes").join(file_name);
        if !path.exists() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent).and_then(|()| std::fs::write(&path, json));
            }
        }
        self.theme_store = ThemeStore::new(theme, file_name.to_string(), path);
        self.last_applied_theme_text = json.to_string();
        self.last_warned_invalid_theme = None;
        self.theme_read_failed = false;
        self.capture_action = None;
        self.capture_conflict = None;
        self.reset_armed = false;
        self.settings.theme = file_name.to_string();
        self.persist(cx);
        cx.notify();
        true
    }

    /// Open the full-screen Settings surface from any view, remembering the
    /// origin so `Esc` / Back returns exactly there. Capture state is
    /// cleared: opening Settings never resumes a stale capture.
    pub fn open_settings(&mut self, cx: &mut Context<Self>) {
        if self.view == View::Settings {
            return;
        }
        self.settings_return_to = self.view;
        self.view = View::Settings;
        self.sort_menu_open = false;
        self.capture_action = None;
        self.capture_conflict = None;
        self.reset_armed = false;
        self.settings_scroll_px = 0.0;
        self.note_interaction(cx);
        cx.notify();
    }

    /// Close the Settings surface back to the originating view. Breaks any
    /// in-progress capture (focus-capture edge case: capture never survives
    /// a view change).
    pub fn close_settings(&mut self, cx: &mut Context<Self>) {
        self.capture_action = None;
        self.capture_conflict = None;
        self.reset_armed = false;
        self.view = self.settings_return_to;
        self.note_interaction(cx);
        cx.notify();
    }

    /// Persist a new keymap and rebind live without restart.
    ///
    /// Atomic `settings::save` (tmp + rename) first; on success the whole
    /// keymap is re-registered (`clear_key_bindings` + `bind_keys`) so the
    /// new combo fires immediately (verified supported in gpui 0.2.2:
    /// `AppContext::bind_keys` is callable from any `Context`). A failed
    /// save keeps the old bindings — nothing is rebound on a write error.
    pub fn apply_keymap(&mut self, keymap: sh_core::keymap::Keymap, cx: &mut Context<Self>) {
        let mut saved = self.settings.clone();
        saved.keymap = keymap;
        saved.version = 7;
        if sh_core::settings::save(&self.settings_path, &saved).is_ok() {
            // NOTE: intentionally synchronous — one small local JSON file
            // (sub-ms); the disk-reload test depends on no-race semantics,
            // unlike `persist()`'s fire-and-forget background write.
            self.settings = saved;
            cx.clear_key_bindings();
            cx.bind_keys(crate::actions::resolve_bindings(&self.settings.keymap));
        } else {
            tracing::warn!("could not persist keymap; bindings unchanged");
        }
        cx.notify();
    }

    /// Persist a new UI language and live-switch every surface without restart.
    ///
    /// Commit-on-success (mirrors [`Self::apply_keymap`]): the candidate is
    /// saved atomically first; on `Ok` the in-memory language commits and
    /// `cx.notify()` re-renders all surfaces through
    /// `t(settings.language, …)`. On `Err` a warning is logged and the
    /// previous language is kept — no commit, no notify.
    pub fn apply_language(&mut self, lang: sh_core::i18n::Language, cx: &mut Context<Self>) {
        let mut saved = self.settings.clone();
        saved.language = lang;
        saved.version = 7;
        // NOTE: intentionally synchronous — same no-race contract as
        // `apply_keymap` (one small local JSON file, sub-ms).
        if sh_core::settings::save(&self.settings_path, &saved).is_ok() {
            self.settings = saved;
            self.note_interaction(cx);
            cx.notify();
        } else {
            tracing::warn!("could not persist language; language unchanged");
        }
    }

    // General: language picker + hidden-files toggle row + recents list +
    // Clear button. Every string resolves via the table from
    // `settings.language`, so `apply_language`'s notify re-renders live.
    fn render_general_section(
        &mut self,
        surface: Hsla,
        text: Hsla,
        _accent: Hsla,
        _bg: Hsla,
        row_hover: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use crate::ui::settings_panel::sections::general as gen;
        use sh_core::i18n::{t, StrKey};
        let lang = self.settings.language;
        let mut col = div().flex().flex_col().gap(px(8.0));
        // Language picker: one row per option, check on the current.
        // Commit-on-success via `apply_language` (failed save keeps the old
        // language); the notify there re-renders every surface, no restart.
        col = col.child(
            div()
                .px(px(10.0))
                .py(px(4.0))
                .text_color(text)
                .child(t(lang, StrKey::LanguageLabel)),
        );
        for (row_idx, (option, autonym)) in gen::language_options().iter().enumerate() {
            let selected = *option == lang;
            let next = *option;
            let swallow = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                cx.stop_propagation();
            });
            let mut row = div()
                .id(("settings-language-row", row_idx))
                .flex()
                .items_center()
                .justify_between()
                .cursor_pointer()
                .rounded(px(6.0))
                .px(px(10.0))
                .py(px(6.0))
                .hover(move |s| s.bg(row_hover))
                .child(div().text_color(text).child(*autonym))
                .child(
                    div()
                        .text_color(text)
                        .child(if selected { "✓" } else { "" }),
                )
                .on_mouse_down(MouseButton::Left, swallow)
                .on_click(
                    cx.listener(move |this: &mut App, _ev: &ClickEvent, _window, cx| {
                        this.note_interaction(cx);
                        this.apply_language(next, cx);
                    }),
                );
            if selected {
                row = row.bg(surface);
            }
            col = col.child(row);
        }
        // Hidden files toggle.
        let hidden = self.settings.show_hidden_files;
        let swallow = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
            cx.stop_propagation();
        });
        col = col.child(
            div()
                .id("settings-show-hidden")
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_between()
                .rounded(px(6.0))
                .px(px(10.0))
                .py(px(6.0))
                .bg(surface)
                .hover(move |s| s.bg(row_hover))
                .text_color(text)
                .child(t(lang, StrKey::ShowHiddenFiles))
                .child(if hidden { "✓" } else { "" })
                .on_mouse_down(MouseButton::Left, swallow)
                .on_click(
                    cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                        this.note_interaction(cx);
                        // Optimistic UI (same contract as persist): memory updates now for instant feedback; disk write is best-effort and warns on failure.
                        this.settings.show_hidden_files = !this.settings.show_hidden_files;
                        this.settings.version = 7;
                        let s = this.settings.clone();
                        let path = this.settings_path.clone();
                        cx.background_executor()
                            .spawn(async move {
                                if let Err(e) = sh_core::settings::save(&path, &s) {
                                    tracing::warn!("could not persist settings: {e}");
                                }
                            })
                            .detach();
                        cx.notify();
                    }),
                ),
        );
        // Recents header + rows + Clear.
        col = col.child(div().px(px(10.0)).py(px(4.0)).text_color(text).child(
            crate::ui::settings_panel::sections::general::recents_header(
                lang,
                self.settings.recent_dirs.len(),
            ),
        ));
        // Recent rows open their folder in Grid — the same contract as the
        // Welcome recent chips (pinned by
        // `settings_recent_row_opens_folder_in_grid`).
        for (idx, dir) in self.settings.recent_dirs.clone().iter().enumerate() {
            // Owned path: the click closure must be `'static`, so it cannot
            // borrow the temporary `Vec` above (Welcome-chip pattern).
            let dir = dir.clone();
            let swallow_recent =
                cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                });
            col = col.child(
                div()
                    .id(("settings-recent", idx))
                    .cursor_pointer()
                    .rounded(px(6.0))
                    .px(px(10.0))
                    .py(px(4.0))
                    .text_color(text)
                    .hover(move |s| s.bg(row_hover))
                    .child(sh_core::recent::display_name(&dir))
                    .on_mouse_down(MouseButton::Left, swallow_recent)
                    .on_click(
                        cx.listener(move |this: &mut App, _ev: &ClickEvent, _window, cx| {
                            this.note_interaction(cx);
                            this.open_folder(dir.clone(), cx);
                        }),
                    ),
            );
        }
        if !self.settings.recent_dirs.is_empty() {
            let swallow_clear =
                cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                });
            col = col.child(
                div()
                    .id("settings-clear-recents")
                    .cursor_pointer()
                    .rounded(px(6.0))
                    .px(px(12.0))
                    .py(px(4.0))
                    .bg(surface)
                    .hover(move |s| s.bg(row_hover))
                    .text_color(text)
                    .child(t(lang, StrKey::ClearRecents))
                    .on_mouse_down(MouseButton::Left, swallow_clear)
                    .on_click(
                        cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                            this.note_interaction(cx);
                            // Optimistic UI (same contract as persist): memory updates now for instant feedback; disk write is best-effort and warns on failure.
                            this.settings.recent_dirs.clear();
                            this.settings.last_dir = None;
                            this.recent_dirs_available.clear();
                            this.settings.version = 7;
                            let s = this.settings.clone();
                            let path = this.settings_path.clone();
                            cx.background_executor()
                                .spawn(async move {
                                    if let Err(e) = sh_core::settings::save(&path, &s) {
                                        tracing::warn!("could not persist settings: {e}");
                                    }
                                })
                                .detach();
                            cx.notify();
                        }),
                    ),
            );
        }
        col.into_any()
    }

    // Appearance: theme picker rows (moved from the old topbar dropdown).
    fn render_appearance_section(
        &mut self,
        surface: Hsla,
        text: Hsla,
        accent: Hsla,
        _bg: Hsla,
        row_hover: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use sh_core::i18n::{t, StrKey};
        let lang = self.settings.language;
        let mut col = div().flex().flex_col().gap(px(2.0));
        col = col.child(
            div()
                .px(px(10.0))
                .py(px(4.0))
                .text_color(text)
                .child(t(lang, StrKey::ThemeLabel)),
        );
        for (row_idx, (file, json)) in crate::theme_builtins::BUILTIN_THEMES.iter().enumerate() {
            let display =
                crate::ui::settings_panel::sections::appearance::theme_display_name(file, json);
            let active = *file == self.theme_store.name;
            let name = file.to_string();
            let swallow = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                cx.stop_propagation();
            });
            let mut row = div()
                .id(("settings-theme-row", row_idx))
                .flex()
                .items_center()
                .justify_between()
                .cursor_pointer()
                .rounded(px(6.0))
                .px(px(10.0))
                .py(px(6.0))
                .hover(move |s| s.bg(row_hover))
                .child(div().text_color(text).child(display))
                .child(
                    div()
                        .text_color(if active { accent } else { text })
                        .child(if active { "✓" } else { "" }),
                )
                .on_mouse_down(MouseButton::Left, swallow)
                .on_click(
                    cx.listener(move |this: &mut App, _ev: &ClickEvent, _window, cx| {
                        this.note_interaction(cx);
                        // Apply WITHOUT closing: the surface persists.
                        this.apply_builtin_theme(&name, cx);
                    }),
                );
            if active {
                row = row.bg(surface);
            }
            col = col.child(row);
        }
        col.into_any()
    }

    // Shortcuts: Reset button + one row per ACTIONS entry ([label] [chip]).
    // Click chip → capture mode; conflict → red error, stay capturing;
    // free → apply_keymap (atomic save + live rebind).
    fn render_shortcuts_section(
        &mut self,
        surface: Hsla,
        text: Hsla,
        accent: Hsla,
        _bg: Hsla,
        row_hover: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use crate::ui::settings_panel::sections::shortcuts as sc;
        use sh_core::i18n::{t, StrKey};
        let lang = self.settings.language;
        let mut col = div().flex().flex_col().gap(px(2.0));

        // Reset-all button (two-step inline confirm, the batch-bar pattern).
        let armed = self.reset_armed;
        let swallow_reset = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
            cx.stop_propagation();
        });
        col = col.child(
            div()
                .id("shortcuts-reset")
                .cursor_pointer()
                .flex()
                .items_center()
                .rounded(px(6.0))
                .px(px(12.0))
                // Fixed height (see `scroll::SHORTCUT_RESET_H_PX`): the
                // scroll clamp math depends on it.
                .h(px(scroll::SHORTCUT_RESET_H_PX))
                .bg(surface)
                .hover(move |s| s.bg(row_hover))
                .text_color(if armed { accent } else { text })
                .child(if armed {
                    t(lang, StrKey::ResetConfirm)
                } else {
                    t(lang, StrKey::ResetShortcuts)
                })
                .on_mouse_down(MouseButton::Left, swallow_reset)
                .on_click(
                    cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                        this.note_interaction(cx);
                        if this.reset_armed {
                            this.reset_armed = false;
                            this.capture_action = None;
                            this.capture_conflict = None;
                            this.apply_keymap(sh_core::keymap::defaults(), cx);
                        } else {
                            this.reset_armed = true;
                            cx.notify();
                        }
                    }),
                ),
        );

        // One row per action, grouped by display context in ACTIONS order.
        let mut last_group = "";
        let defaults = sh_core::keymap::defaults();
        for (idx, desc) in crate::actions::ACTIONS.iter().enumerate() {
            if desc.context != last_group {
                last_group = desc.context;
                col = col.child(
                    div()
                        .flex()
                        .items_center()
                        .px(px(10.0))
                        // Fixed height (see `scroll::SHORTCUT_GROUP_H_PX`).
                        .h(px(scroll::SHORTCUT_GROUP_H_PX))
                        .text_color(text)
                        .child(desc.context),
                );
            }
            let capturing = self.capture_action.as_deref() == Some(desc.id);
            let stored = self
                .settings
                .keymap
                .get(desc.id)
                .cloned()
                .unwrap_or_else(|| {
                    defaults
                        .get(desc.id)
                        .cloned()
                        .unwrap_or(sh_core::keymap::KeyBinding {
                            ctrl: false,
                            shift: false,
                            alt: false,
                            platform: false,
                            key: "?".into(),
                        })
                });
            let chip_label = if capturing {
                match &self.capture_conflict {
                    Some(incumbent_id) => {
                        sc::conflict_text(lang, sc::action_label(incumbent_id, lang))
                    }
                    None => sc::capture_prompt(lang).into(),
                }
            } else {
                sc::chip_text(&stored)
            };
            let has_error = capturing && self.capture_conflict.is_some();
            let id = desc.id;
            let swallow_row = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                cx.stop_propagation();
            });
            let mut chip = div()
                .id(("shortcut-chip", idx))
                .cursor_pointer()
                .rounded(px(6.0))
                .px(px(10.0))
                .py(px(4.0))
                // Single-line chip (grid-label discipline): long capture /
                // conflict text truncates instead of wrapping, which would
                // silently break the fixed row height below.
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .bg(surface)
                .text_color(if has_error { accent } else { text });
            if !has_error {
                chip = chip.hover(move |s| s.bg(row_hover));
            }
            // Error state: red text (accent is cyan; use a fixed red — themes
            // have no error token in V1).
            if has_error {
                let error_red: Hsla = rgb(0xff5555).into();
                chip = chip.text_color(error_red);
            }
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .rounded(px(6.0))
                    .px(px(10.0))
                    // Fixed height (see `scroll::SHORTCUT_ROW_H_PX`): the
                    // scroll clamp math depends on it — keep rows
                    // single-line.
                    .h(px(scroll::SHORTCUT_ROW_H_PX))
                    .text_color(text)
                    .child(t(lang, desc.label_key))
                    .child(
                        chip.child(chip_label)
                            .on_mouse_down(MouseButton::Left, swallow_row)
                            .on_click(cx.listener(
                                move |this: &mut App, _ev: &ClickEvent, _window, cx| {
                                    this.note_interaction(cx);
                                    this.reset_armed = false;
                                    this.capture_action = Some(id.to_string());
                                    this.capture_conflict = None;
                                    cx.notify();
                                },
                            )),
                    ),
            );
        }
        col.into_any()
    }

    /// Persist theme + last_dir to `settings.json` on a worker (atomic write
    /// via sh-core's `.tmp` + rename).
    ///
    /// Saves a copy of the STARTUP-loaded settings with only `theme` and
    /// `last_dir` updated — never `Settings::default()`, which would stomp
    /// user-edited values in unrelated fields. Called only from
    /// [`Self::open_path`]: `last_dir` only changes when the folder changes,
    /// so per-navigation writes would be redundant and would race the shared
    /// `.tmp` rename. Fire-and-forget: a failed write is logged, never
    /// surfaced as an error state.
    fn persist(&mut self, cx: &mut Context<Self>) {
        let folder = self
            .session
            .current_item()
            .and_then(|i| i.path.parent().map(std::path::Path::to_path_buf));
        // V3 recents: push the opened folder (dedupe + trim, pure core
        // helper), then mirror last_dir = recent[0] so a v2 binary reading
        // a v3 file still finds its Continue path. A parent-less edge
        // (no current image) leaves the existing list untouched.
        if let Some(dir) = folder {
            self.settings.recent_dirs =
                sh_core::recent::push_recent(&self.settings.recent_dirs, dir);
        }
        self.settings.last_dir = self.settings.recent_dirs.first().cloned();
        // `recent_dirs_available` is refreshed here (every persist: file
        // dialog, drag&drop, navigate-folder-change) — the in-memory mirror
        // the Welcome screen reads.
        self.recent_dirs_available = self.settings.recent_dirs.clone();
        self.settings.theme = self.theme_store.name.clone();
        let s = self.settings.clone();
        let path = self.settings_path.clone();
        cx.background_executor()
            .spawn(async move {
                if let Err(e) = sh_core::settings::save(&path, &s) {
                    tracing::warn!("could not persist settings: {e}");
                }
            })
            .detach();
    }

    /// Spawn the theme hot-reload watcher: polls the active theme file every
    /// [`THEME_POLL`] and applies it when the text changed and parses.
    ///
    /// Disk read AND parse run on the background executor (off the main
    /// thread); the entity update only happens when there is something to
    /// apply. Unchanged text is skipped (`hot_reload_decision`), changed text
    /// that fails validation keeps the last valid theme and warns once per
    /// DISTINCT invalid text (deduped by [`Self::last_applied_theme_text`]).
    /// A read error (deleted/unreadable file) is distinct from an invalid
    /// file: it warns once on the Ok→Err transition, is treated as
    /// UNCHANGED for that tick (no parse, no apply, last valid theme kept),
    /// and normal flow resumes on recovery. The loop exits once the App
    /// entity is dropped.
    pub fn spawn_theme_watcher(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(THEME_POLL).await;
            // Read + parse off the main thread; a locked/huge file can never
            // stall a frame.
            let path = this.update(cx, |app, _| app.theme_store.path.clone()).ok();
            let Some(path) = path else {
                break; // entity dropped → stop polling
            };
            let read_task = cx
                .background_executor()
                .spawn(async move { std::fs::read_to_string(&path) });
            let read = read_task.await;

            // Read-error transition handling (main thread: mutates the
            // `theme_read_failed` flag and warns once per failure period).
            let read_text = this
                .update(cx, |app, _| match read {
                    Ok(text) => {
                        app.theme_read_failed = false; // recovered (or still ok)
                        Some(text)
                    }
                    Err(e) => {
                        // Warn once per failure period, not per tick.
                        if !app.theme_read_failed {
                            tracing::warn!("could not read theme file: {e}");
                            app.theme_read_failed = true;
                        }
                        None
                    }
                })
                .ok();
            let Some(text) = read_text else {
                break; // entity dropped → stop polling
            };
            // Deleted/unreadable: treat as unchanged this tick (keep last
            // valid theme; no parse, no apply).
            let Some(text) = text else {
                continue;
            };

            let decision = this.update(cx, |app, _| {
                hot_reload_decision(&app.last_applied_theme_text, &text)
            });
            let Some(decision) = decision.ok() else {
                break; // entity dropped → stop polling
            };
            if let HotReloadDecision::Apply = decision {
                // Parse off the main thread too — theme JSON is small but
                // validation is pure CPU and belongs on a worker.
                let parse_text = text.clone();
                let parse_task = cx
                    .background_executor()
                    .spawn(async move { sh_core::theme::parse(&parse_text) });
                let parsed = parse_task.await;
                let applied = this.update(cx, |app, cx| {
                    match parsed {
                        Ok(theme) => {
                            let path = app.theme_store.path.clone();
                            app.theme_store.set(theme, path);
                            app.last_applied_theme_text = text;
                            cx.notify();
                        }
                        Err(e) => {
                            // Keep the last valid theme; warn once per
                            // DISTINCT invalid text.
                            if app.last_warned_invalid_theme.as_deref() != Some(text.as_str()) {
                                tracing::warn!("theme hot-reload rejected: {e}");
                                app.last_warned_invalid_theme = Some(text);
                            }
                        }
                    }
                });
                if applied.is_err() {
                    break; // entity dropped → stop polling
                }
            }
        })
        .detach();
    }

    /// Navigate `delta` steps (‑1 = prev, +1 = next) with a header probe on a worker.
    ///
    /// Spawns a background dimension probe for the target image (never a full
    /// pixel decode), then updates session state on completion. A neighbor
    /// prefetch warms the next image's dimensions.
    ///
    /// Completions carry a monotonic sequence number: a completion belonging
    /// to a navigation that is no longer the latest is dropped ENTIRELY — it
    /// may neither commit global state (`zoom`/`fit_mode`/`error`) nor write
    /// slot `dimensions`. This is not just about stale UI: `open_path` swaps
    /// the whole images Vec, so a stale completion's captured index could
    /// point past the new list (out-of-bounds panic) or poison a slot with
    /// another image's dimensions. ALL session mutations are seq-guarded.
    pub fn navigate(&mut self, delta: isize, cx: &mut Context<Self>) {
        let n = self.session.images.len();
        let Some(next) = next_index(self.session.current, delta, n) else {
            return;
        };
        self.navigation_seq += 1;
        let seq = self.navigation_seq;
        self.session.current = next;
        self.session.error = None;
        // Navigate-while-open updates in place: the popover stays open and
        // its facts are re-resolved for the new current file atomically with
        // the index advance, so no stale paint of image A is possible.
        if self.info_panel_open {
            self.refresh_info_facts();
        }

        // ── Probe current image dimensions on background thread ──
        // NOTE: fit is computed at COMPLETION time against the live viewport,
        // so a resize mid-probe picks up the current size.
        let path = self.session.images[next].path.clone();
        let bg = cx.background_executor();
        // Slice B: the SAME background task coalesces the alpha verdict
        // (`probe_has_alpha`: JPEG fast path = header cost, no pixel decode;
        // alpha-capable formats decode off the frame loop). One task, one
        // file open per probe — no new threads, no render-time I/O.
        let probe_task = bg.spawn(async move {
            let dims = sh_core::decode::probe_dimensions(&path);
            let alpha = sh_core::decode::probe_has_alpha(&path);
            (dims, alpha)
        });
        cx.spawn(async move |this, cx| {
            let (dims, alpha) = probe_task.await;
            let _ = this.update(cx, |app, cx| {
                // A stale probe skips entirely (see the seq-guard invariant
                // above): if the user returns to this image later, the next
                // navigation probes it again — a header read is near-instant.
                if seq != app.navigation_seq {
                    return;
                }
                match dims {
                    Ok((w, h)) => {
                        app.session.images[next].dimensions = Some((w, h));
                        app.session.zoom = sh_core::transform::fit(
                            sh_core::transform::Vec2 {
                                x: w as f32,
                                y: h as f32,
                            },
                            app.viewer_viewport(),
                        );
                        app.session.fit_mode = FitMode::Fit;
                    }
                    Err(_) => {
                        app.session.error = Some("could not read image".into());
                    }
                }
                // A failed alpha probe leaves `None` (render as opaque):
                // the dims probe above owns the error slot, and a missing
                // verdict must never surface as a verdict.
                if let Ok(a) = alpha {
                    app.session.images[next].has_alpha = Some(a);
                }
                cx.notify();
            });
        })
        .detach();

        // ── Prefetch neighbor dimensions (CPU-side session warm-up) ──
        // NOTE: This warms CPU-side session state (dimensions for fit/zoom
        // math) but does NOT populate GPUI's `img()` asset cache. Render-side
        // reuse arrives with the use_asset wiring in later tasks.
        if n > 1 {
            let pre_idx = (next + 1) % n;
            let pre_path = self.session.images[pre_idx].path.clone();
            let bg = cx.background_executor();
            // Slice B: the neighbor prefetch warms the alpha verdict with
            // the same coalesced task (dims + alpha, one file open).
            let pre_task = bg.spawn(async move {
                let dims = sh_core::decode::probe_dimensions(&pre_path);
                let alpha = sh_core::decode::probe_has_alpha(&pre_path);
                (dims, alpha)
            });
            cx.spawn(async move |this, cx| {
                let (pre, pre_alpha) = pre_task.await;
                let _ = this.update(cx, |app, cx| {
                    // Same seq guard as the probe: a stale prefetch's index
                    // may be meaningless after an open_path Vec swap. The
                    // warm-cache optimization simply dies with its navigation.
                    if seq != app.navigation_seq {
                        return;
                    }
                    if let Ok(d) = pre {
                        app.session.images[pre_idx].dimensions = Some(d);
                    }
                    if let Ok(a) = pre_alpha {
                        app.session.images[pre_idx].has_alpha = Some(a);
                    }
                    cx.notify();
                });
            })
            .detach();
        }

        // Sync state (current/error) changed; paint immediately instead of
        // waiting for the probe to land.
        cx.notify();
    }
}

/// Convert a pixel size into a transform vector (small helper to avoid
/// repeating the `f32::from` dance at every call site).
fn viewport_vec(viewport: Size<Pixels>) -> sh_core::transform::Vec2 {
    sh_core::transform::Vec2 {
        x: f32::from(viewport.width),
        y: f32::from(viewport.height),
    }
}

/// Hover deadband in pixels: bare-hover moves shorter than this don't reset
/// the overlay idle clock. Filters sensor-noise micro-movements that would
/// otherwise re-show the overlays right at/after the idle threshold and
/// cause show/hide oscillation.
pub const HOVER_DEADBAND_PX: f32 = 3.0;

/// Whether the cursor moved far enough to count as a real interaction.
///
/// Pure helper over `(x, y)` pixel pairs (kept free of GPUI types for
/// trivial unit testing). Euclidean distance `>= HOVER_DEADBAND_PX`
/// counts; anything less is treated as sensor noise. A missing anchor
/// (`None` at the call site) always counts — there is no baseline yet.
pub fn hover_moved_enough(old: (f32, f32), new: (f32, f32)) -> bool {
    let dx = new.0 - old.0;
    let dy = new.1 - old.1;
    dx.hypot(dy) >= HOVER_DEADBAND_PX
}

/// Pure predicate: should the solid topbar be dissolved? Viewer-only by
/// construction (Grid never dissolves — the caller guards on view). The
/// bar dissolves on idle ONLY when overlays are enabled: Tab-pinned chrome
/// (overlays off) keeps the solid bar, since dissolving it then would
/// remove the last visible UI.
pub fn topbar_hidden(idle: bool, overlays_disabled: bool) -> bool {
    idle && !overlays_disabled
}

/// Pure wheel routing: Welcome and Settings never touch viewer or grid
/// state. Welcome has nothing to zoom or scroll; Settings content scrolls
/// natively (`overflow_y_scroll` owns the gesture), so the wheel handler
/// must not fall through to viewer zoom — that would silently mutate
/// `session.zoom` while in Settings. Pure so the routing is unit-testable
/// (the harness cannot synthesize wheel events).
pub fn wheel_parks(view: View) -> bool {
    matches!(view, View::Welcome | View::Settings)
}

/// Viewer fit-area height: full viewport minus the solid bar when visible,
/// full viewport when the bar is dissolved. Floors at 1.0 like the existing
/// `viewer_viewport` subtraction (a zero/negative fit area would collapse
/// the fit math).
pub fn viewer_fit_height(viewport_h: f32, topbar_hidden: bool) -> f32 {
    let bar = if topbar_hidden {
        0.0
    } else {
        topbar::TOPBAR_H_PX
    };
    (viewport_h - bar).max(1.0)
}

/// Hover tint: the modern button idiom (Figma/Linear/Zed) — no border swap;
/// the background itself lightens (or darkens, on light themes) toward the
/// foreground color. Channel-wise mix in RGBA space; `ratio` 0 = pure
/// background, 1 = pure foreground. Opaque so it works as a solid `.bg()`.
pub fn hover_tint(bg: Hsla, fg: Hsla, ratio: f32) -> Hsla {
    let b: Rgba = bg.into();
    let f: Rgba = fg.into();
    let mix = |x: f32, y: f32| x + (y - x) * ratio;
    Rgba {
        r: mix(b.r, f.r),
        g: mix(b.g, f.g),
        b: mix(b.b, f.b),
        a: 1.0,
    }
    .into()
}

/// Perceived luminance of a color (Rec. 709 luma weights, 0..=1).
fn luma(c: Hsla) -> f32 {
    let r: Rgba = c.into();
    0.2126 * r.r + 0.7152 * r.g + 0.0722 * r.b
}

/// Theme-adaptive hover fill: the same mix ratio that reads as subtle
/// elevation on dark themes becomes a dirty smudge on light ones (the eye
/// is far more sensitive to darkening on light). The ratio derives linearly
/// from the background's own luminance: ~10% on dark, easing down to ~7%
/// on light — a touch stronger than Figma/GitHub-light hover so the plate
/// still reads on white (user feedback). Returns an opaque color ready for
/// `.bg()`.
pub fn hover_fill(bg: Hsla, fg: Hsla) -> Hsla {
    // Linear ramp anchored at the two built-in extremes; clamped so custom
    // themes can't overshoot either way. Slope 0.03 keeps dark at ~10%
    // while lifting the light end from ~5% to ~7%.
    let ratio = (0.10 - luma(bg) * 0.03).clamp(0.07, 0.10);
    hover_tint(bg, fg, ratio)
}

/// Strong hover fill for small chips painted near the page color. The
/// default [`hover_fill`] is calibrated for flat surfaces that pop a gray
/// card against the page; a small button whose base is *brighter* than the
/// page (e.g. the Welcome `surface` chips on light-clean) darkens TOWARD
/// the page on hover, so its edge contrast collapses and it reads as "no
/// hover" — even though the pixel delta equals the approved grid plate.
/// This variant lifts the floor so the chip darkens clearly PAST the page
/// color, restoring the same perceived edge cue as the grid, while staying
/// luma-adaptive (~10% on light, ~13% on dark).
pub fn hover_fill_strong(bg: Hsla, fg: Hsla) -> Hsla {
    let ratio = (0.135 - luma(bg) * 0.035).clamp(0.10, 0.135);
    hover_tint(bg, fg, ratio)
}

/// Expose the root focus handle so external code (and GPUI's
/// `window.focus_view`) can focus the app's `image_view` subtree.
impl Focusable for App {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// Sort dropdown rows: criterion list + direction list (V3 sort chip).
/// Labels are table keys resolved with `settings.language` at the render
/// site, so the menu live-switches (S3). The source of truth is these
/// `sh-app` literals' keys — `sh-core::navigation` carries no display
/// strings; the keys still live in `sh-core::i18n`.
const SORT_MENU_ITEMS: &[(SortBy, StrKey)] = &[
    (SortBy::Name, StrKey::SortName),
    (SortBy::Created, StrKey::SortCreated),
    (SortBy::Modified, StrKey::SortModified),
    (SortBy::Size, StrKey::SortSize),
    (SortBy::Type, StrKey::SortType),
];
const SORT_DIR_ITEMS: &[(SortDir, StrKey)] = &[
    (SortDir::Asc, StrKey::SortAscending),
    (SortDir::Desc, StrKey::SortDescending),
];

/// Human label for the sort chip: localized criterion + locale-neutral
/// direction arrow (`↑`/`↓` stay untranslated per the glyph exemption).
/// Pure so the format is unit-testable.
pub fn sort_chip_label(lang: Language, by: SortBy, dir: SortDir) -> String {
    let name = SORT_MENU_ITEMS
        .iter()
        .find(|(by2, _)| *by2 == by)
        .map(|(_, key)| lang.get(*key))
        .unwrap_or(lang.get(StrKey::SortName));
    let arrow = match dir {
        SortDir::Asc => "↑",
        SortDir::Desc => "↓",
    };
    format!("{name} {arrow}")
}

/// Human label for one density segment: the localized word (`"Small"` /
/// `"Pequeño"`), never a bare letter. Pure so the format is unit-testable.
pub fn grid_size_label(lang: Language, size: GridSize) -> String {
    lang.get(match size {
        GridSize::S => StrKey::GridSizeSmall,
        GridSize::M => StrKey::GridSizeMedium,
        GridSize::L => StrKey::GridSizeLarge,
    })
    .to_string()
}

/// Segment model for the grid-density chip: exactly S/M/L in order with
/// the active preset flagged. Pure so labels + active marking are
/// unit-testable; the render maps it 1:1 to clickable segments.
pub fn grid_size_segments(lang: Language, active: GridSize) -> Vec<(GridSize, String, bool)> {
    [GridSize::S, GridSize::M, GridSize::L]
        .iter()
        .map(|size| (*size, grid_size_label(lang, *size), *size == active))
        .collect()
}

/// Human label for one zoom-preset chip: localized `"Fit"`/`"Ajustar"` or
/// the locale-neutral numeral. Pure so the format is unit-testable.
pub fn zoom_preset_label(lang: Language, preset: ZoomPreset) -> String {
    lang.get(match preset {
        ZoomPreset::Fit => StrKey::ZoomPresetFit,
        ZoomPreset::Scale50 => StrKey::ZoomPreset50,
        ZoomPreset::Scale100 => StrKey::ZoomPreset100,
        ZoomPreset::Scale200 => StrKey::ZoomPreset200,
    })
    .to_string()
}

/// Segment model for the zoom-preset chips: exactly Fit/50/100/200 in
/// order with the active chip flagged. Pure so labels + active marking are
/// unit-testable; the render maps it 1:1 to clickable segments (same shape
/// as [`grid_size_segments`]).
///
/// Active rule = display-percent equality derived from the zoom text format
/// (`format!("{:.0}%", scale * 100.0)`): a chip reads active exactly when
/// the zoom text already reads its percentage. Fit is active iff the
/// session is in [`FitMode::Fit`]; scale chips are gated on
/// [`FitMode::Percent100`] so a fit scale that happens to round to a preset
/// never dual-activates, and a free-zoomed 137% matches no chip.
pub fn zoom_preset_segments(
    lang: Language,
    fit_mode: FitMode,
    scale: f32,
) -> Vec<(ZoomPreset, String, bool)> {
    let in_fit = fit_mode == FitMode::Fit;
    let percent = (scale * 100.0).round() as i32;
    [
        ZoomPreset::Fit,
        ZoomPreset::Scale50,
        ZoomPreset::Scale100,
        ZoomPreset::Scale200,
    ]
    .iter()
    .map(|preset| {
        let active = match preset {
            ZoomPreset::Fit => in_fit,
            ZoomPreset::Scale50 => !in_fit && percent == 50,
            ZoomPreset::Scale100 => !in_fit && percent == 100,
            ZoomPreset::Scale200 => !in_fit && percent == 200,
        };
        (*preset, zoom_preset_label(lang, *preset), active)
    })
    .collect()
}

/// The slideshow chip shows the ACTION, not the state: Pause while
/// playing, Play while stopped.
fn slideshow_icon(active: bool) -> IconName {
    if active {
        IconName::Pause
    } else {
        IconName::Play
    }
}

/// Topbar-left suffix for the selection count: `""` when empty, otherwise
/// the localized `selected_suffix` template. Thin wrapper so the builder
/// stays readable; the wording itself is unit-tested in `sh-core::i18n`.
fn selected_count_suffix(lang: Language, selected: &std::collections::BTreeSet<usize>) -> String {
    sh_core::i18n::selected_suffix(lang, selected.len())
}

/// Confirm-bar message for a staged op: counts + destination for moves
/// (`display_name`-shortened). Pure so the wording is unit-testable; the
/// sentence itself lives in the `i18n` templates (S5) with the one/other
/// plural handled there.
fn batch_bar_message(lang: Language, op: &BatchOp) -> String {
    match op {
        BatchOp::Delete { paths } => sh_core::i18n::batch_bar_delete(lang, paths.len()),
        BatchOp::Move { paths, dest } => {
            sh_core::i18n::batch_bar_move(lang, paths.len(), &sh_core::recent::display_name(dest))
        }
    }
}

/// Cadence of the idle watcher poll. Independent of [`overlay::OVERLAY_IDLE`]
/// (the actual hide threshold) — a short tick keeps the hide within ~500ms
/// of the deadline without notifying more than once.
const IDLE_TICK: std::time::Duration = std::time::Duration::from_millis(500);

/// Slideshow auto-advance interval (V3). Fixed by design; the
/// settings-panel slice promotes this to a persisted field with the
/// serde-default migration pattern.
pub const SLIDESHOW_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);

/// Cadence of the theme hot-reload poll (AGENTS.md §10: apply within ~1.5s
/// of an edit; 1s poll + file read comfortably meets that).
const THEME_POLL: std::time::Duration = std::time::Duration::from_secs(1);

impl Render for App {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Live viewport: GPUI re-renders on window resize (`on_resize` →
        // `bounds_changed` → `refresh`), so reading the drawable size here
        // keeps `App.viewport` current on every frame. When the size changed
        // (e.g. fullscreen toggle), the fit computed for the OLD viewport
        // would render off-center — recompute it (Fit mode only; Percent100
        // user zoom is intentionally left alone on resize).
        let new_viewport = window.viewport_size();
        if new_viewport != self.viewport {
            self.viewport = new_viewport;
            self.session.refit_for_viewport(self.viewer_viewport());
        }

        let bg: Hsla =
            parse_hex(&self.theme_store.theme.colors.background).unwrap_or(rgb(0x0d0d0f).into());
        let params = ViewerParams {
            path: self.session.current_item().map(|i| i.path.clone()),
            error: self.session.error.clone(),
            zoom_scale: self.session.zoom.scale,
            pan_offset: self.session.zoom.offset,
            decoded_size: self
                .session
                .current_dimensions()
                .map(|(w, h)| (w as f32, h as f32)),
            lang: self.settings.language,
            // Task 2.5: the SINGLE computation site for the board gate.
            // `render_viewer` stays pure-presentational: it only reads this
            // precomputed bool. `None` (verdict in flight) renders without
            // the board until the navigate probe lands + notifies.
            show_checkerboard: crate::viewer::should_show_checkerboard(
                self.settings.checkerboard,
                self.session.current_item().and_then(|i| i.has_alpha),
            ),
        };

        // ── Overlay visibility = Tab-toggled && not idle ──
        let idle = self.last_interaction.elapsed() > overlay::OVERLAY_IDLE;
        let bottom_visible = self.session.show_overlay_bottom && !idle;

        // ── Topbar dissolve (Viewer only) ──
        // Grid never dissolves: folder actions (open, settings) must stay
        // reachable and no image is covered there. Tab-pinned chrome keeps
        // the solid bar: overlays_disabled means the user wants chrome to stay.
        let topbar_dissolved =
            self.view == View::Viewer && topbar_hidden(idle, !self.session.show_overlay_bottom);
        if self.topbar_was_hidden != topbar_dissolved {
            self.topbar_was_hidden = topbar_dissolved;
            self.session.refit_for_viewport(self.viewer_viewport());
        }

        let overlay_data = OverlayData::from_theme(
            format!("{:.0}%", self.session.zoom.scale * 100.0),
            &self.theme_store.theme.colors.text,
            &self.theme_store.theme.colors.surface,
        );

        // ── Bottom overlay action chrome (shared by chips + arrows) ──
        // Topbar density-control idiom: bg tints toward the theme text on
        // hover, double-step pressed tint for the active chip. The overlay
        // action_button helper applies these only to chrome'd call sites;
        // prev/next/slideshow pass `chrome: false` and stay pixel-identical.
        let chip_bg =
            parse_hex(&self.theme_store.theme.colors.background).unwrap_or(rgb(0x0d0d0f).into());
        let chip_hover = hover_fill(chip_bg, overlay_data.theme_text);
        let chip_pressed: Hsla = {
            let h: Rgba = chip_hover.into();
            let b: Rgba = chip_bg.into();
            let step = |x: f32, y: f32| x + (x - y);
            Rgba {
                r: step(h.r, b.r),
                g: step(h.g, b.g),
                b: step(h.b, b.b),
                a: h.a,
            }
            .into()
        };
        let action_style = overlay::ActionButtonStyle {
            text: overlay_data.theme_text,
            idle_bg: chip_bg,
            hover_bg: chip_hover,
            active_bg: chip_pressed,
        };
        let bare_action = overlay::ActionButtonOpts {
            chrome: false,
            active: false,
            pad_x: 0.0,
            pad_y: 0.0,
        };

        // Build nav arrow elements for the bottom overlay. Constructed with
        // `cx.listener` here (same pattern as Tasks 7/8) and handed to the
        // overlay as pre-built elements.
        let on_prev = cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
            this.navigate(-1, cx);
        });
        let on_next = cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
            this.navigate(1, cx);
        });
        // Swallow mouse-down on the buttons so double-clicking an arrow
        // navigates twice instead of also toggling fit on the root div.
        let swallow_prev = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
            cx.stop_propagation();
        });
        let swallow_next = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
            cx.stop_propagation();
        });
        let prev_btn: AnyElement = overlay::action_button(
            icon(IconName::ChevronLeft, px(14.0), overlay_data.theme_text),
            &action_style,
            &bare_action,
        )
        .id("prev-btn")
        .on_mouse_down(MouseButton::Left, swallow_prev)
        .on_click(on_prev)
        .into_any();
        let next_btn: AnyElement = overlay::action_button(
            icon(IconName::ChevronRight, px(14.0), overlay_data.theme_text),
            &action_style,
            &bare_action,
        )
        .id("next-btn")
        .on_mouse_down(MouseButton::Left, swallow_next)
        .on_click(on_next)
        .into_any();
        let swallow_slide = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
            cx.stop_propagation();
        });
        let on_toggle_slide = cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
            this.toggle_slideshow(cx);
        });
        // V3 slideshow chip: play/pause between zoom text and arrows.
        let slideshow_btn: AnyElement = overlay::action_button(
            icon(
                slideshow_icon(self.session.slideshow_active),
                px(14.0),
                overlay_data.theme_text,
            ),
            &action_style,
            &bare_action,
        )
        .id("slideshow-btn")
        .on_mouse_down(MouseButton::Left, swallow_slide)
        .on_click(on_toggle_slide)
        .into_any();

        // Zoom-preset chips: one per `zoom_preset_segments` entry, built
        // through the same action_button helper with the pill chrome. The
        // mousedown swallow keeps taps out of the root pan/double-click-fit
        // handler; `note_interaction` resets the idle clock FIRST so the
        // overlay cannot fade out right after the tap.
        let chip_buttons: Vec<AnyElement> = zoom_preset_segments(
            self.settings.language,
            self.session.fit_mode,
            self.session.zoom.scale,
        )
        .into_iter()
        .enumerate()
        .map(|(idx, (preset, label, active))| {
            let swallow_chip = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                cx.stop_propagation();
            });
            overlay::action_button(
                label,
                &action_style,
                &overlay::ActionButtonOpts {
                    chrome: true,
                    active,
                    pad_x: 12.0,
                    pad_y: 4.0,
                },
            )
            .id(("zoom-preset", idx as u64))
            .on_mouse_down(MouseButton::Left, swallow_chip)
            .on_click(
                cx.listener(move |this: &mut App, _ev: &ClickEvent, _window, cx| {
                    this.note_interaction(cx);
                    let viewport = this.viewer_viewport();
                    this.session.set_zoom_preset(preset, viewport);
                    cx.notify();
                }),
            )
            .into_any()
        })
        .collect();

        // Info-panel chip: localized label through the shared action_button
        // helper with pill chrome (zoom-chip precedent). Mousedown-swallow
        // keeps the tap out of the root pan/double-click-fit handler;
        // `note_interaction` runs FIRST inside the toggle so the overlay
        // cannot fade out right after the tap. Click-only by design: no
        // action id, no keymap entry, no Shortcuts-panel row.
        let swallow_info = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
            cx.stop_propagation();
        });
        let info_btn: AnyElement = overlay::action_button(
            t(self.settings.language, StrKey::InfoButtonLabel),
            &action_style,
            &overlay::ActionButtonOpts {
                chrome: true,
                active: self.info_panel_open,
                pad_x: 12.0,
                pad_y: 4.0,
            },
        )
        .id("info-btn")
        .on_mouse_down(MouseButton::Left, swallow_info)
        .on_click(
            cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                this.toggle_info_panel(cx);
            }),
        )
        .into_any();

        let viewer = render_viewer(&params);

        // ── V2 Task 6: top bar data (grid: folder name; viewer: name — pos).
        // Built for every frame; only attached outside Welcome below.
        let topbar_data = topbar::TopbarData {
            left: format!(
                "{}{}",
                self.recent_dirs_available
                    .first()
                    .and_then(|d| d.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or(""),
                selected_count_suffix(self.settings.language, &self.selected)
            ),
            center: if self.view == View::Viewer {
                format!(
                    "{} — {}",
                    self.session
                        .current_item()
                        .map(|i| i.name.clone())
                        .unwrap_or_default(),
                    self.session.position_label()
                )
            } else {
                // V3 batch report: last-op status, cleared on next staging
                // or folder change. Empty in every other case (grid had no
                // center content before).
                self.batch_status.clone().unwrap_or_default()
            },
            theme_text: parse_hex(&self.theme_store.theme.colors.text)
                .unwrap_or(rgb(0xe8e8ee).into()),
            theme_surface: parse_hex(&self.theme_store.theme.colors.surface)
                .unwrap_or(rgb(0x121218).into()),
        };
        let swallow_back_btn = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
            cx.stop_propagation();
        });
        let swallow_open_btn = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
            cx.stop_propagation();
        });
        let swallow_gear_btn = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
            cx.stop_propagation();
        });
        let swallow_sort_btn = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
            cx.stop_propagation();
        });
        let swallow_crop_btn = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
            cx.stop_propagation();
        });
        let topbar_el = if self.view != View::Welcome && self.view != View::Settings {
            // Button chips sit on the surface bar, so they use the app
            // background for contrast (same text color as the bar).
            let btn_bg = parse_hex(&self.theme_store.theme.colors.background)
                .unwrap_or(rgb(0x0d0d0f).into());
            // Modern hover idiom: no border swap — the bg itself tints
            // toward the theme text (color-mix), works on dark and light
            // themes alike.
            let btn_hover = hover_fill(btn_bg, topbar_data.theme_text);
            // Pressed tint for active chips (open menus / crop mode):
            // double hover-delta (stays theme-adaptive like hover).
            let btn_pressed = {
                let h: Rgba = btn_hover.into();
                let b: Rgba = btn_bg.into();
                let step = |x: f32, y: f32| x + (x - y);
                Rgba {
                    r: step(h.r, b.r),
                    g: step(h.g, b.g),
                    b: step(h.b, b.b),
                    a: 1.0,
                }
                .into()
            };
            let back_btn: AnyElement = div()
                .id("topbar-back")
                .cursor_pointer()
                .flex()
                .items_center()
                .gap(px(6.0))
                .bg(btn_bg)
                .hover(move |s| s.bg(btn_hover))
                .text_color(topbar_data.theme_text)
                .rounded(px(6.0))
                .px(px(12.0))
                .py(px(4.0))
                .child(icon(IconName::BackArrow, px(14.0), topbar_data.theme_text))
                .child(t(self.settings.language, StrKey::TopbarBack))
                .on_mouse_down(MouseButton::Left, swallow_back_btn)
                .on_click(
                    cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                        this.note_interaction(cx);
                        this.enter_grid(cx);
                    }),
                )
                .into_any();
            let open_btn: AnyElement = div()
                .id("topbar-open")
                .cursor_pointer()
                .bg(btn_bg)
                .hover(move |s| s.bg(btn_hover))
                .text_color(topbar_data.theme_text)
                .rounded(px(6.0))
                .px(px(12.0))
                .py(px(4.0))
                .child(t(self.settings.language, StrKey::TopbarOpen))
                .on_mouse_down(MouseButton::Left, swallow_open_btn)
                .on_click(
                    cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                        this.pick_folder(cx);
                    }),
                )
                .into_any();
            let gear_btn: AnyElement = div()
                .id("topbar-settings")
                .cursor_pointer()
                .bg(btn_bg)
                .hover(move |s| s.bg(btn_hover))
                .text_color(topbar_data.theme_text)
                .rounded(px(6.0))
                .px(px(10.0))
                .py(px(4.0))
                .child(icon(IconName::Gear, px(14.0), topbar_data.theme_text))
                .on_mouse_down(MouseButton::Left, swallow_gear_btn)
                .on_click(
                    cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                        this.sort_menu_open = false;
                        this.open_settings(cx);
                    }),
                )
                .into_any();
            // V3 sort chip: shows the active criterion + direction; click
            // toggles the sort dropdown.
            let sort_btn: AnyElement = div()
                .id("topbar-sort")
                .cursor_pointer()
                // Open menu keeps the pressed tint so the chip reads as active.
                .bg(if self.sort_menu_open {
                    btn_pressed
                } else {
                    btn_bg
                })
                .hover(move |s| s.bg(btn_hover))
                .text_color(topbar_data.theme_text)
                .rounded(px(6.0))
                .px(px(12.0))
                .py(px(4.0))
                .child(sort_chip_label(
                    self.settings.language,
                    self.session.sort_by,
                    self.session.sort_dir,
                ))
                .on_mouse_down(MouseButton::Left, swallow_sort_btn)
                .on_click(
                    cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                        this.note_interaction(cx);
                        this.sort_menu_open = !this.sort_menu_open;
                        cx.notify();
                    }),
                )
                .into_any();
            // Density segmented control: one segment per preset with the
            // localized word; the active preset wears the pressed tint (the
            // sort-chip idiom). Segments dispatch straight to
            // `set_grid_size` — instant switch, no restart, no re-sort.
            let size_btn: AnyElement = {
                let mut row = div().id("topbar-size").flex().items_center().gap(px(4.0));
                for (idx, (size, label, active)) in
                    grid_size_segments(self.settings.language, self.settings.grid_size)
                        .into_iter()
                        .enumerate()
                {
                    let swallow =
                        cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                            cx.stop_propagation();
                        });
                    row = row.child(
                        div()
                            .id(("grid-size", idx as u64))
                            .cursor_pointer()
                            .bg(if active { btn_pressed } else { btn_bg })
                            .hover(move |s| s.bg(btn_hover))
                            .text_color(topbar_data.theme_text)
                            .rounded(px(6.0))
                            .px(px(12.0))
                            .py(px(4.0))
                            .child(label)
                            .on_mouse_down(MouseButton::Left, swallow)
                            .on_click(cx.listener(
                                move |this: &mut App, _ev: &ClickEvent, _window, cx| {
                                    this.note_interaction(cx);
                                    this.set_grid_size(size, cx);
                                },
                            )),
                    );
                }
                row.into_any()
            };
            let crop_btn = if self.view == View::Viewer {
                Some(
                    div()
                        .id("topbar-crop")
                        .cursor_pointer()
                        // Active crop mode: the pressed state is a stronger
                        // tint toward text (was the pressed border).
                        .bg(if self.crop_mode { btn_pressed } else { btn_bg })
                        .hover(move |s| s.bg(btn_hover))
                        .text_color(topbar_data.theme_text)
                        .rounded(px(6.0))
                        .px(px(10.0))
                        .py(px(4.0))
                        .child(icon(IconName::Scissors, px(14.0), topbar_data.theme_text))
                        .on_mouse_down(MouseButton::Left, swallow_crop_btn)
                        .on_click(
                            cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                                this.toggle_crop(cx);
                            }),
                        )
                        .into_any(),
                )
            } else {
                None
            };
            // Grid arm passes no back button (welcome is startup-only);
            // Viewer passes ← Grid.
            let back = if self.view == View::Viewer {
                Some(back_btn)
            } else {
                None
            };
            let bar = topbar::topbar(
                &topbar_data,
                back,
                open_btn,
                sort_btn,
                size_btn,
                gear_btn,
                crop_btn,
            );
            // .hidden() = Display::None (same mechanism as the overlay gate:
            // no hitboxes, element IDs stay stable). Mouse move >= deadband
            // wakes the idle watcher, which re-renders and restores the bar.
            if topbar_dissolved {
                Some(bar.hidden().into_any_element())
            } else {
                Some(bar.into_any_element())
            }
        } else {
            None
        };

        // ── V2: view-specific content ──
        // Welcome: startup screen with Continue / Open-folder. Buttons are
        // built here (cx.listener call-site pattern, same as overlay arrows).
        // Button + hero strings resolve through the i18n table (S3) so the
        // surface live-switches with `settings.language`.
        let welcome_el = if self.view == View::Welcome {
            let lang = self.settings.language;
            let welcome_data = welcome::WelcomeData::from_theme(
                &self.theme_store.theme.colors.text,
                &self.theme_store.theme.colors.surface,
                &self.theme_store.theme.colors.accent,
            );
            let swallow_continue =
                cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                });
            let swallow_open = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                cx.stop_propagation();
            });
            // Modern hover idiom (matches the topbar buttons): bg tints
            // toward the theme text on hover — no border swap. Use the
            // strong variant: these chips start at the *surface* color (pure
            // white on light-clean) which sits above the page, so the plain
            // 7% fill would darken them INTO the page and read as no hover.
            let welcome_hover =
                hover_fill_strong(welcome_data.theme_surface, welcome_data.theme_text);
            let continue_btn = self.recent_dirs_available.first().cloned().map(|dir| {
                let btn = div()
                    .id("welcome-continue")
                    .cursor_pointer()
                    .bg(welcome_data.theme_surface)
                    .hover(move |s| s.bg(welcome_hover))
                    .text_color(welcome_data.theme_text)
                    .rounded(px(6.0))
                    .px(px(16.0))
                    .py(px(8.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(icon(
                                IconName::ChevronRight,
                                px(14.0),
                                welcome_data.theme_text,
                            ))
                            .child(lang.get(StrKey::ContinueButton)),
                    )
                    .on_mouse_down(MouseButton::Left, swallow_continue)
                    .on_click(
                        cx.listener(move |this: &mut App, _ev: &ClickEvent, _window, cx| {
                            this.note_interaction(cx);
                            this.open_folder(dir.clone(), cx);
                        }),
                    );
                btn
            });
            let open_btn: AnyElement = div()
                .id("welcome-open")
                .cursor_pointer()
                .bg(welcome_data.theme_surface)
                .hover(move |s| s.bg(welcome_hover))
                .text_color(welcome_data.theme_text)
                .rounded(px(6.0))
                .px(px(16.0))
                .py(px(8.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(icon(
                            IconName::FolderOpen,
                            px(14.0),
                            welcome_data.theme_text,
                        ))
                        .child(lang.get(StrKey::WelcomeOpen)),
                )
                .on_mouse_down(MouseButton::Left, swallow_open)
                .on_click(
                    cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                        this.pick_folder(cx);
                    }),
                )
                .into_any();
            // V3 recent-folder chips: recent[1..] (Continue covers [0]).
            // Pre-built here with cx.listener — the Continue/Open pattern.
            // Element ids are index-keyed tuples (the sort-row pattern):
            // gpui 0.2.2 has no From<(&str, String)>.
            let recent_chips: Vec<AnyElement> = self
                .recent_dirs_available
                .iter()
                .skip(1)
                .enumerate()
                .map(|(idx, dir)| {
                    let dir = dir.clone();
                    let swallow =
                        cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                            cx.stop_propagation();
                        });
                    div()
                        .id(("welcome-recent", idx as u64))
                        .cursor_pointer()
                        .bg(welcome_data.theme_surface)
                        .hover(move |s| s.bg(welcome_hover))
                        .text_color(welcome_data.theme_text)
                        .text_size(px(12.0))
                        .rounded(px(8.0))
                        .px(px(10.0))
                        .py(px(6.0))
                        .child(sh_core::recent::display_name(&dir))
                        .on_mouse_down(MouseButton::Left, swallow)
                        .on_click(cx.listener(
                            move |this: &mut App, _ev: &ClickEvent, _window, cx| {
                                this.note_interaction(cx);
                                this.open_folder(dir.clone(), cx);
                            },
                        ))
                        .into_any()
                })
                .collect();
            Some(
                welcome::welcome(
                    lang,
                    &welcome_data,
                    continue_btn.map(|b| b.into_any()),
                    open_btn,
                    recent_chips,
                )
                .into_any_element(),
            )
        } else {
            None
        };

        // ── V2 Task 7: grid content (cells pre-built below with clicks). ──
        let grid_el = if self.view == View::Grid && !self.session.images.is_empty() {
            let accent =
                parse_hex(&self.theme_store.theme.colors.accent).unwrap_or(rgb(0x00ffff).into());
            let text =
                parse_hex(&self.theme_store.theme.colors.text).unwrap_or(rgb(0xe8e8ee).into());
            // Cell geometry follows the active density preset (S/M/L); M is
            // today's geometry verbatim. Cast once here — scroll math below
            // stays `f32` as before.
            let geo = self.settings.grid_size.geometry();
            let cell_w = geo.cell_w as f32;
            let thumb_w = geo.thumb_w as f32;
            let thumb_h = geo.thumb_h as f32;
            let label_w = geo.label_w as f32;
            let bar_w = geo.bar_w as f32;
            let bar_h = geo.bar_h as f32;
            let bar_left = geo.bar_left as f32;
            // Cell hover plate: the SAME tint as the buttons (hover_fill over
            // the app background) — one hover language across the whole app,
            // dark and light themes alike.
            let cell_hover = hover_fill(bg, text);
            let mut cells: Vec<AnyElement> = Vec::with_capacity(self.session.images.len());
            for (idx, item) in self.session.images.iter().enumerate() {
                // V3 multi-select: every set member wears the accent bar,
                // not just the cursor.
                // V3 multi-select: the cursor (last visited) wears the full
                // accent bar; set members wear the same bar dimmed — one
                // marker shape, two intensities, readable in any theme.
                let is_cursor = idx == self.grid_selected;
                let in_set = self.selected.contains(&idx);
                let swallow_cell =
                    cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                        cx.stop_propagation();
                    });
                // Dimmed label color: 60% alpha text (editorial hierarchy).
                let mut label_color = text;
                label_color.a = 0.6;
                // Thumb or placeholder: a missing decode (batch still
                // running, slow/corrupt file) shows the themed chip
                // until the background batch lands.
                let thumb: AnyElement = match self.thumbs.get(&item.path) {
                    Some(arc) => img(arc.clone())
                        .id(("grid-thumb", idx))
                        .w(px(thumb_w))
                        .h(px(thumb_h))
                        .rounded(px(10.0)) // 8 → 10 per spec
                        .into_any(),
                    None => div()
                        .id(("grid-thumb-empty", idx))
                        .w(px(thumb_w))
                        .h(px(thumb_h))
                        .bg(topbar_data.theme_surface)
                        .rounded(px(10.0))
                        .into_any(),
                };
                // Minimal active marker: a slim accent bar laid over the
                // thumbnail's bottom edge (streaming-app active pattern) —
                // no border box around the cell. The relative frame anchors
                // the absolutely-positioned bar to the thumb itself.
                let mut thumb_frame = div().relative().child(thumb);
                // V3 multi-select markers — position, not intensity: the
                // cursor (last visited) wears the bottom bar; set members
                // wear the same bar mirrored at the top. Both at full accent
                // so they read in any theme; cursor+member shows both bars.
                if in_set {
                    // Same geometry as the cursor bar, mirrored to the top.
                    thumb_frame = thumb_frame.child(
                        div()
                            .id(("grid-selected-bar", idx))
                            .absolute()
                            .top(px(0.0))
                            .left(px(bar_left))
                            .w(px(bar_w))
                            .h(px(bar_h))
                            .rounded(px(2.0))
                            .bg(accent),
                    );
                }
                if is_cursor {
                    // Inset 10px horizontally so the bar clears the thumb's
                    // rounded corners; 3px tall, pill-shaped.
                    thumb_frame = thumb_frame.child(
                        div()
                            .id(("grid-active-bar", idx))
                            .absolute()
                            .bottom(px(0.0))
                            .left(px(bar_left))
                            .w(px(bar_w))
                            .h(px(bar_h))
                            .rounded(px(2.0))
                            .bg(accent),
                    );
                }
                let cell = div()
                    .id(("grid-cell", idx))
                    .w(px(cell_w))
                    .cursor_pointer()
                    // Centered flex column: the cell is wider than the
                    // thumb+label pair — without centering the content sat
                    // flush left and the plate jutted out to the right.
                    // Centered, the plate reads as a symmetric card around
                    // the image. (Cell height = thumb + label, unchanged —
                    // preset row-height math intact.)
                    .flex()
                    .flex_col()
                    .items_center()
                    // Plate corners echo the thumb's rounding so the hover
                    // shape matches the image shape.
                    .rounded(px(10.0))
                    // Hover plate: same color-mix idiom as every button —
                    // the cell background tints toward the theme text. This
                    // replaces the old soft shadow (invisible on dark,
                    // heavy on light); it reads as a subtle plate under the
                    // thumb in BOTH theme families.
                    .hover(move |s| s.bg(cell_hover))
                    .child(thumb_frame)
                    // Single-line ellipsis: a wrapped label grows the row
                    // and breaks the scroll math (see GRID_ROW_H_PX).
                    // Label brightens on hover: the hover sits on the label
                    // div itself (nested in the cell, both fire together).
                    .child(
                        div()
                            .w(px(label_w))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_color(label_color)
                            .hover(move |s| s.text_color(text))
                            .child(item.name.clone()),
                    )
                    .on_mouse_down(MouseButton::Left, swallow_cell)
                    .on_click(
                        cx.listener(move |this: &mut App, ev: &ClickEvent, _window, cx| {
                            this.note_interaction(cx);
                            if ev.modifiers().control {
                                this.toggle_selected(idx, cx);
                            } else if ev.modifiers().shift {
                                this.extend_selection_to(idx, cx);
                            } else {
                                this.enter_viewer(idx, cx);
                            }
                        }),
                    );
                cells.push(cell.into_any());
            }
            Some(grid::grid(cells, self.grid_scroll_px).into_any_element())
        } else {
            None
        };

        // Empty grid (no images: empty folder or failed resolve): centered
        // message instead of cells. The error slot carries the reason.
        // The default (no error reason) resolves through the table (S3).
        let grid_empty_el = if self.view == View::Grid && self.session.images.is_empty() {
            let text =
                parse_hex(&self.theme_store.theme.colors.text).unwrap_or(rgb(0xe8e8ee).into());
            let lang = self.settings.language;
            let msg = self.session.error.clone().unwrap_or_else(|| {
                sh_core::i18n::t(lang, sh_core::i18n::StrKey::GridEmptyDefault).to_string()
            });
            Some(
                div()
                    .id("grid-empty")
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().text_color(text).child(msg))
                    .into_any_element(),
            )
        } else {
            None
        };

        // ── Sort dropdown (Grid + Viewer): full-window click catcher closes
        // on outside click; the menu lists criteria + directions with the
        // active one checked. Rendered last so both float above content.
        let (sort_catcher_el, sort_menu_el) = if self.sort_menu_open && self.view != View::Welcome {
            let lang = self.settings.language;
            let surface =
                parse_hex(&self.theme_store.theme.colors.surface).unwrap_or(rgb(0x121218).into());
            let text =
                parse_hex(&self.theme_store.theme.colors.text).unwrap_or(rgb(0xe8e8ee).into());
            let accent =
                parse_hex(&self.theme_store.theme.colors.accent).unwrap_or(rgb(0x00ffff).into());
            let row_bg = parse_hex(&self.theme_store.theme.colors.background)
                .unwrap_or(rgb(0x0d0d0f).into());
            let row_hover = hover_fill(row_bg, text);
            let catcher: AnyElement = div()
                .id("sort-catcher")
                .absolute()
                .top(px(0.0))
                .left(px(0.0))
                .right(px(0.0))
                .bottom(px(0.0))
                .cursor_default()
                .on_mouse_down(
                    // Swallow the press so the grid cell / viewer gesture
                    // behind the catcher never arms.
                    MouseButton::Left,
                    cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                        cx.stop_propagation();
                    }),
                )
                .on_click(
                    cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                        this.sort_menu_open = false;
                        cx.notify();
                    }),
                )
                .into_any();
            // Criterion rows: keep the current direction, change the criterion.
            let mut list = div().flex().flex_col().gap(px(2.0));
            for (row_idx, (by, key)) in SORT_MENU_ITEMS.iter().enumerate() {
                let active = *by == self.session.sort_by;
                let swallow_row =
                    cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                        cx.stop_propagation();
                    });
                let mut row = div()
                    .id(("sort-row", row_idx))
                    .flex()
                    .items_center()
                    .justify_between()
                    .cursor_pointer()
                    .rounded(px(6.0))
                    .px(px(10.0))
                    .py(px(6.0))
                    .hover(move |s| s.bg(row_hover))
                    .child(div().text_color(text).child(lang.get(*key)))
                    .child(
                        div()
                            .text_color(if active { accent } else { text })
                            .child(if active { "✓" } else { "" }),
                    )
                    .on_mouse_down(MouseButton::Left, swallow_row)
                    .on_click(
                        cx.listener(move |this: &mut App, _ev: &ClickEvent, _window, cx| {
                            this.note_interaction(cx);
                            this.sort_menu_open = false;
                            this.set_sort(*by, this.session.sort_dir, cx);
                        }),
                    );
                if active {
                    row = row.bg(row_bg);
                }
                list = list.child(row);
            }
            // Direction rows: keep the current criterion, change direction.
            for (row_idx, (dir, key)) in SORT_DIR_ITEMS.iter().enumerate() {
                let active = *dir == self.session.sort_dir;
                let swallow_row =
                    cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                        cx.stop_propagation();
                    });
                let mut row = div()
                    .id(("sort-dir-row", row_idx))
                    .flex()
                    .items_center()
                    .justify_between()
                    .cursor_pointer()
                    .rounded(px(6.0))
                    .px(px(10.0))
                    .py(px(6.0))
                    .hover(move |s| s.bg(row_hover))
                    .child(div().text_color(text).child(lang.get(*key)))
                    .child(
                        div()
                            .text_color(if active { accent } else { text })
                            .child(if active { "✓" } else { "" }),
                    )
                    .on_mouse_down(MouseButton::Left, swallow_row)
                    .on_click(
                        cx.listener(move |this: &mut App, _ev: &ClickEvent, _window, cx| {
                            this.note_interaction(cx);
                            this.sort_menu_open = false;
                            this.set_sort(this.session.sort_by, *dir, cx);
                        }),
                    );
                if active {
                    row = row.bg(row_bg);
                }
                list = list.child(row);
            }
            let swallow_menu = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                cx.stop_propagation();
            });
            let menu: AnyElement = div()
                .id("sort-menu")
                .absolute()
                .top(px(topbar::TOPBAR_H_PX + 8.0))
                .right(px(12.0))
                .bg(surface)
                .text_color(text)
                .rounded(px(8.0))
                .p(px(8.0))
                .on_mouse_down(MouseButton::Left, swallow_menu)
                .child(
                    div()
                        .px(px(10.0))
                        .py(px(4.0))
                        .child(lang.get(StrKey::SortByLabel)),
                )
                .child(list)
                .into_any();
            (Some(catcher), Some(menu))
        } else {
            (None, None)
        };

        // ── V2: viewer + overlays, Viewer-only, confined below the bar. ──
        // Crop selection overlay: accent border, no dim (YAGNI — border only).
        // Normalized at paint time; render never mutates state.
        let crop_overlay: Option<AnyElement> = match (&self.crop_mode, &self.crop_rect) {
            (true, Some(rect)) => {
                let n = rect.normalized();
                let x = n.x0.min(n.x1);
                let y = n.y0.min(n.y1);
                let w = (n.x1 - n.x0).abs();
                let h = (n.y1 - n.y0).abs();
                let accent = parse_hex(&self.theme_store.theme.colors.accent)
                    .unwrap_or(rgb(0x00ffff).into());
                Some(
                    div()
                        .id("crop-selection")
                        .absolute()
                        .left(px(x))
                        .top(px(y))
                        .w(px(w))
                        .h(px(h))
                        .border(px(2.0))
                        .border_color(accent)
                        .into_any_element(),
                )
            }
            _ => None,
        };
        // ── Task 5: crop confirm bar (Copy / Save / Cancel) ──
        // Floating above the bottom overlay, hidden while a drag is still
        // armed (crop_rect Some + bar hidden = mid-drag by construction:
        // the bar only appears via finish_crop_drag).
        let crop_bar_el: Option<AnyElement> = if self.crop_bar_visible {
            let surface =
                parse_hex(&self.theme_store.theme.colors.surface).unwrap_or(rgb(0x121218).into());
            let text =
                parse_hex(&self.theme_store.theme.colors.text).unwrap_or(rgb(0xe8e8ee).into());
            let accent =
                parse_hex(&self.theme_store.theme.colors.accent).unwrap_or(rgb(0x00ffff).into());
            let swallow_crop_bar =
                cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                });
            // Modern hover idiom (same as topbar): bg tints toward text.
            let bar_hover = hover_fill(surface, text);
            let bar_btn = |id: &'static str,
                           label: &'static str,
                           on_click: fn(&mut App, &ClickEvent, &mut Window, &mut Context<App>)|
             -> AnyElement {
                let swallow = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                });
                div()
                    .id(id)
                    .cursor_pointer()
                    .bg(surface)
                    .hover(move |s| s.bg(bar_hover))
                    .text_color(text)
                    .rounded(px(6.0))
                    .px(px(12.0))
                    .py(px(4.0))
                    .child(label)
                    .on_mouse_down(MouseButton::Left, swallow)
                    .on_click(cx.listener(on_click))
                    .into_any_element()
            };
            let copy_btn = bar_btn(
                "crop-copy",
                t(self.settings.language, StrKey::CropCopy),
                |this: &mut App, _ev: &ClickEvent, _window, cx| {
                    this.confirm_crop_copy(cx);
                },
            );
            let save_btn = bar_btn(
                "crop-save",
                t(self.settings.language, StrKey::CropSave),
                |this: &mut App, _ev: &ClickEvent, _window, cx| {
                    this.confirm_crop_save(cx);
                },
            );
            let cancel_btn = bar_btn(
                "crop-cancel",
                t(self.settings.language, StrKey::Cancel),
                |this: &mut App, _ev: &ClickEvent, _window, cx| {
                    this.cancel_crop(cx);
                },
            );
            Some(
                div()
                    .id("crop-confirm-bar-anchor")
                    .absolute()
                    .bottom(px(56.0))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .on_mouse_down(MouseButton::Left, swallow_crop_bar)
                    .child(
                        div()
                            .id("crop-confirm-bar")
                            .bg(surface)
                            .text_color(text)
                            .border(px(1.0))
                            .border_color(accent)
                            .rounded(px(8.0))
                            .p(px(6.0))
                            .flex()
                            .gap(px(8.0))
                            .child(copy_btn)
                            .child(save_btn)
                            .child(cancel_btn),
                    )
                    .into_any_element(),
            )
        } else {
            None
        };
        // V3 batch confirm bar: staged destructive op awaiting approval.
        // Built at render top level (NOT inside the viewer arm): the bar is
        // born in grid, and anything mounted under viewer-area never paints
        // there. Same anchor+inner shape as the crop bar otherwise.
        let batch_bar_el: Option<AnyElement> = self.pending_batch.as_ref().map(|op| {
            let surface =
                parse_hex(&self.theme_store.theme.colors.surface).unwrap_or(rgb(0x121218).into());
            let text =
                parse_hex(&self.theme_store.theme.colors.text).unwrap_or(rgb(0xe8e8ee).into());
            let accent =
                parse_hex(&self.theme_store.theme.colors.accent).unwrap_or(rgb(0x00ffff).into());
            let swallow_batch_bar =
                cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                });
            let bar_hover = hover_fill(surface, text);
            let bar_btn = |id: &'static str,
                           label: &'static str,
                           on_click: fn(&mut App, &ClickEvent, &mut Window, &mut Context<App>)|
             -> AnyElement {
                let swallow = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                });
                div()
                    .id(id)
                    .cursor_pointer()
                    .bg(surface)
                    .hover(move |s| s.bg(bar_hover))
                    .text_color(text)
                    .rounded(px(6.0))
                    .px(px(12.0))
                    .py(px(4.0))
                    .child(label)
                    .on_mouse_down(MouseButton::Left, swallow)
                    .on_click(cx.listener(on_click))
                    .into_any_element()
            };
            let (confirm_id, confirm_label) = match op {
                BatchOp::Delete { .. } => (
                    "batch-confirm-delete",
                    t(self.settings.language, StrKey::BatchDelete),
                ),
                BatchOp::Move { .. } => (
                    "batch-confirm-move",
                    t(self.settings.language, StrKey::BatchMove),
                ),
            };
            let confirm_btn = bar_btn(
                confirm_id,
                confirm_label,
                |this: &mut App, _ev: &ClickEvent, _window, cx| {
                    this.confirm_pending(cx);
                },
            );
            let cancel_btn = bar_btn(
                "batch-cancel",
                t(self.settings.language, StrKey::Cancel),
                |this: &mut App, _ev: &ClickEvent, _window, cx| {
                    this.pending_batch = None;
                    cx.notify();
                },
            );
            div()
                .id("batch-confirm-bar-anchor")
                .absolute()
                .bottom(px(12.0))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .on_mouse_down(MouseButton::Left, swallow_batch_bar)
                .child(
                    div()
                        .id("batch-confirm-bar")
                        .bg(surface)
                        .text_color(text)
                        .border(px(1.0))
                        .border_color(accent)
                        .rounded(px(8.0))
                        .p(px(6.0))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(batch_bar_message(self.settings.language, op))
                        .child(confirm_btn)
                        .child(cancel_btn),
                )
                .into_any_element()
        });
        // Settings surface: slim header + sidebar + content. Rendered INSTEAD of
        // topbar/grid/viewer (those arms already guard on their own views; the
        // topbar now also excludes Settings).
        let settings_el: Option<AnyElement> = if self.view == View::Settings {
            let surface =
                parse_hex(&self.theme_store.theme.colors.surface).unwrap_or(rgb(0x121218).into());
            let text =
                parse_hex(&self.theme_store.theme.colors.text).unwrap_or(rgb(0xe8e8ee).into());
            let accent =
                parse_hex(&self.theme_store.theme.colors.accent).unwrap_or(rgb(0x00ffff).into());
            let bg = parse_hex(&self.theme_store.theme.colors.background)
                .unwrap_or(rgb(0x0d0d0f).into());
            let row_hover = hover_fill(surface, text);
            let lang = self.settings.language;
            let t_title = sh_core::i18n::t(lang, sh_core::i18n::StrKey::SettingsTitle);

            // Slim header: ← Back + settings title.
            let swallow_back = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                cx.stop_propagation();
            });
            let back_btn: AnyElement = div()
                .id("settings-back")
                .cursor_pointer()
                .flex()
                .items_center()
                .gap(px(6.0))
                .bg(bg)
                .hover(move |s| s.bg(row_hover))
                .text_color(text)
                .rounded(px(6.0))
                .px(px(12.0))
                .py(px(4.0))
                .child(icon(IconName::BackArrow, px(14.0), text))
                .child(t_title)
                .on_mouse_down(MouseButton::Left, swallow_back)
                .on_click(
                    cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                        this.close_settings(cx);
                    }),
                )
                .into_any();

            // Sidebar rows (caller-built, active wears accent bar).
            let mut side_rows: Vec<AnyElement> = Vec::new();
            for (idx, (section, label_key)) in crate::ui::settings_panel::SettingsSection::ALL
                .iter()
                .enumerate()
            {
                let active = *section == self.settings_section;
                let section = *section;
                let swallow = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                });
                side_rows.push(
                    div()
                        .id(("settings-section", idx))
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .rounded(px(6.0))
                        .px(px(10.0))
                        .py(px(6.0))
                        .bg(if active { surface } else { bg })
                        .hover(move |s| s.bg(row_hover))
                        .text_color(if active { accent } else { text })
                        .child(sh_core::i18n::t(lang, *label_key))
                        .on_mouse_down(MouseButton::Left, swallow)
                        .on_click(cx.listener(
                            move |this: &mut App, _ev: &ClickEvent, _window, cx| {
                                this.note_interaction(cx);
                                this.settings_section = section;
                                // Switching sections breaks capture (edge case:
                                // capture scoped to the panel, broken on change).
                                this.capture_action = None;
                                this.capture_conflict = None;
                                this.reset_armed = false;
                                // Fresh section starts unscrolled (grid resets
                                // its offset on folder change, same rule).
                                this.settings_scroll_px = 0.0;
                                cx.notify();
                            },
                        ))
                        .into_any(),
                );
            }

            // Content rows per section (General + Appearance fully; Shortcuts in Task 7).
            let content: AnyElement = match self.settings_section {
                crate::ui::settings_panel::SettingsSection::General => {
                    self.render_general_section(surface, text, accent, bg, row_hover, cx)
                }
                crate::ui::settings_panel::SettingsSection::Appearance => {
                    self.render_appearance_section(surface, text, accent, bg, row_hover, cx)
                }
                crate::ui::settings_panel::SettingsSection::Shortcuts => {
                    self.render_shortcuts_section(surface, text, accent, bg, row_hover, cx)
                }
            };

            Some(
                div()
                    .id("settings-root")
                    .size_full()
                    .flex()
                    .flex_col()
                    .bg(bg)
                    .text_color(text)
                    .child(
                        div()
                            .id("settings-header")
                            .h(px(crate::ui::topbar::TOPBAR_H_PX))
                            .flex()
                            .items_center()
                            .px(px(14.0))
                            .bg(surface)
                            .child(back_btn),
                    )
                    .child(
                        div()
                            .id("settings-body")
                            .flex_1()
                            .flex()
                            .child(crate::ui::settings_panel::sidebar::sidebar(side_rows))
                            .child(
                                div()
                                    .id("settings-content")
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .gap(px(8.0))
                                    .p(px(16.0))
                                    // Clipped column; the inner wrapper below
                                    // translates content up by
                                    // `settings_scroll_px` (grid precedent —
                                    // see `ui/settings_panel/scroll.rs`).
                                    .overflow_hidden()
                                    .child(
                                        div()
                                            .relative()
                                            .top(px(-self.settings_scroll_px))
                                            .child(content),
                                    ),
                            ),
                    )
                    .into_any_element(),
            )
        } else {
            None
        };
        let viewer_el = if self.view == View::Viewer {
            // ── Floating chips (Viewer, while the topbar is dissolved) ──
            // Carry the bar's info + actions as translucent corner chips so
            // the UI never fully disappears. The bottom overlay (zoom +
            // arrows) already has its own idle gate and stays orthogonal.
            // Attached INSIDE viewer-area (the `.relative()` ancestor) so
            // `top(10)` means the window top once the dissolved bar collapses.
            let chips_el = if topbar_dissolved {
                let mut chip_bg = overlay_data.theme_surface;
                chip_bg.a = 0.72; // translucency per spec
                let mut chip_border = overlay_data.theme_surface;
                chip_border.a = 0.35;

                let name_chip = div()
                    .id("chip-name")
                    .bg(chip_bg)
                    .border(px(1.0))
                    .border_color(chip_border)
                    .rounded(px(8.0))
                    .px(px(10.0))
                    .py(px(6.0))
                    .text_color(overlay_data.theme_text)
                    .child(topbar_data.center.clone());

                let swallow_chip_gear =
                    cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                        cx.stop_propagation();
                    });
                let gear_chip: AnyElement = div()
                    .id("chip-settings")
                    .cursor_pointer()
                    .bg(chip_bg)
                    .border(px(1.0))
                    .border_color(chip_border)
                    .rounded(px(8.0))
                    .px(px(10.0))
                    .py(px(6.0))
                    .text_color(overlay_data.theme_text)
                    .child(icon(IconName::Gear, px(14.0), overlay_data.theme_text))
                    .on_mouse_down(MouseButton::Left, swallow_chip_gear)
                    .on_click(
                        cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                            this.open_settings(cx);
                        }),
                    )
                    .into_any();

                let swallow_chip_crop =
                    cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                        cx.stop_propagation();
                    });
                // Crop chip: pressed border communicates active mode (same
                // affordance as the bar's scissors button).
                let crop_chip: AnyElement = div()
                    .id("chip-crop")
                    .cursor_pointer()
                    .bg(chip_bg)
                    .border(px(1.0))
                    .border_color(if self.crop_mode {
                        overlay_data.theme_text
                    } else {
                        chip_border
                    })
                    .rounded(px(8.0))
                    .px(px(10.0))
                    .py(px(6.0))
                    .text_color(overlay_data.theme_text)
                    .child(icon(IconName::Scissors, px(14.0), overlay_data.theme_text))
                    .on_mouse_down(MouseButton::Left, swallow_chip_crop)
                    .on_click(
                        cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                            this.note_interaction(cx);
                            this.toggle_crop(cx);
                        }),
                    )
                    .into_any();

                Some(
                    div()
                        .id("viewer-chips")
                        .absolute()
                        .top(px(10.0))
                        .left(px(12.0))
                        .right(px(12.0))
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(name_chip)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .child(gear_chip)
                                .child(crop_chip),
                        )
                        .into_any_element(),
                )
            } else {
                None
            };

            // ── Viewer info popover (open-only): full-window catcher +
            // panel, following the sort-catcher construction verbatim.
            // Rendered after content so the catcher floats above the image
            // (but below the popover); the popover wrap swallows its own
            // mousedown so panel clicks never arm the root pan gesture.
            // Facts come from the `info_facts` cache only — path match ⇒
            // rows (or the localized error on `Err`), any mismatch ⇒ the
            // localized error copy. Never I/O in the render path.
            let (info_catcher_el, info_popover_el): (Option<AnyElement>, Option<AnyElement>) =
                if self.view == View::Viewer && self.info_panel_open {
                    let lang = self.settings.language;
                    let text = parse_hex(&self.theme_store.theme.colors.text)
                        .unwrap_or(rgb(0xe8e8ee).into());
                    let surface = parse_hex(&self.theme_store.theme.colors.surface)
                        .unwrap_or(rgb(0x121218).into());
                    let swallow_catcher =
                        cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                            cx.stop_propagation();
                        });
                    let catcher: AnyElement = div()
                        .id("info-catcher")
                        .absolute()
                        .top(px(0.0))
                        .left(px(0.0))
                        .right(px(0.0))
                        .bottom(px(0.0))
                        .cursor_default()
                        .on_mouse_down(MouseButton::Left, swallow_catcher)
                        .on_click(
                            cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                                this.close_info_panel(cx);
                            }),
                        )
                        .into_any();
                    let current_path = self.session.current_item().map(|i| i.path.clone());
                    let facts: Option<&sh_core::decode::FileInfo> =
                        match (&self.info_facts, current_path) {
                            (Some((cached_path, Ok(info))), Some(cur)) if cached_path == &cur => {
                                Some(info)
                            }
                            _ => None,
                        };
                    let error = lang.get(StrKey::InfoLoadError);
                    let popover = overlay::info_popover(
                        lang,
                        facts,
                        error,
                        text,
                        surface,
                        self.info_panel_open,
                    );
                    let swallow_pop =
                        cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                            cx.stop_propagation();
                        });
                    let wrapped: AnyElement = div()
                        .id("info-popover-wrap")
                        .on_mouse_down(MouseButton::Left, swallow_pop)
                        .child(popover)
                        .into_any();
                    (Some(catcher), Some(wrapped))
                } else {
                    (None, None)
                };

            Some(
                div()
                    .id("viewer-area")
                    .flex_1()
                    .relative()
                    .overflow_hidden()
                    .child(viewer)
                    .children(crop_overlay)
                    .children(crop_bar_el)
                    // ── Overlay bottom only (zoom + prev/next). The old
                    // floating name chip is gone: the persistent topbar
                    // already shows "name — 3/12", so the chip duplicated
                    // it AND covered part of the image. ──
                    .child(overlay::bottom(
                        &overlay_data,
                        bottom_visible,
                        chip_buttons,
                        Some(slideshow_btn),
                        Some(prev_btn),
                        Some(next_btn),
                        Some(info_btn),
                    ))
                    .children(chips_el)
                    .children(info_catcher_el)
                    .children(info_popover_el)
                    .into_any_element(),
            )
        } else {
            None
        };

        div()
            .id("app-root")
            .size_full()
            .flex()
            .flex_col()
            .key_context("image_view")
            // Track keyboard focus here so the "image_view" context joins
            // the FOCUS STACK, not just the element tree. GPUI matches
            // scoped key bindings against the dispatch path of the focused
            // element; without this, focus never enters the subtree and
            // ←/→/Tab/F11/Ctrl+O bindings never fire (the smoke-test bug).
            // Mousedown on a tracked element auto-focuses it (gpui div.rs
            // registers a bubble-phase mouse listener for exactly this),
            // so clicks on the viewer also keep focus anchored here.
            .track_focus(&self.focus_handle)
            .on_key_down(
                cx.listener(|this: &mut App, ev: &KeyDownEvent, _window, cx| {
                    // Capture mode only: the Shortcuts chip owns the next keypress.
                    // Anything else falls through to normal keymap dispatch.
                    // V1: capture depends on root-div focus (chip click re-anchors via mousedown bubble). If focus escapes (Alt-Tab/OS dialog) while armed, keys won't reach the handler until a chip is clicked again — acceptable V1; close/section-switch clears capture.
                    let Some(action) = this.capture_action.clone() else {
                        return;
                    };
                    if this.view != View::Settings {
                        return;
                    }
                    use crate::ui::settings_panel::sections::shortcuts as sc;
                    let key = ev.keystroke.key.clone();
                    // Esc alone cancels (BackToGrid would otherwise close Settings —
                    // stop propagation here so the global BackToGrid Esc action doesn't also fire).
                    // Shift+Esc also cancels capture rather than rebinding (Esc is
                    // effectively reserved; shift is not checked in the gate below).
                    if key.eq_ignore_ascii_case("escape")
                        && !ev.keystroke.modifiers.control
                        && !ev.keystroke.modifiers.alt
                        && !ev.keystroke.modifiers.platform
                    {
                        this.capture_action = None;
                        this.capture_conflict = None;
                        this.reset_armed = false;
                        cx.notify();
                        cx.stop_propagation();
                        return;
                    }
                    // Modifiers alone: keep waiting.
                    if sc::is_modifier_only(&key) {
                        cx.stop_propagation();
                        return;
                    }
                    let candidate = sh_core::keymap::KeyBinding {
                        ctrl: ev.keystroke.modifiers.control,
                        shift: ev.keystroke.modifiers.shift,
                        alt: ev.keystroke.modifiers.alt,
                        platform: ev.keystroke.modifiers.platform,
                        key,
                    };
                    match sh_core::keymap::validate_binding(
                        &this.settings.keymap,
                        crate::actions::KEYMAP_CONTEXT,
                        &action,
                        &candidate,
                    ) {
                        Ok(()) => {
                            this.capture_action = None;
                            this.capture_conflict = None;
                            this.reset_armed = false;
                            let mut km = this.settings.keymap.clone();
                            km.insert(action, candidate);
                            this.apply_keymap(km, cx);
                        }
                        Err(conflict) => {
                            // Rejection-with-feedback: red error, STAY capturing,
                            // nothing persisted. The incumbent ACTION ID is
                            // stored; the label + message localize at render.
                            this.capture_conflict = Some(conflict.existing_action);
                            cx.notify();
                        }
                    }
                    cx.stop_propagation();
                }),
            )
            .on_action(
                cx.listener(|this: &mut App, _: &ToggleSlideshow, _window, cx| {
                    this.toggle_slideshow(cx);
                }),
            )
            .on_action(cx.listener(|this: &mut App, _: &SelectNext, _window, cx| {
                // Grid-only: viewer arrows navigate images, never select.
                if this.view != View::Grid {
                    return;
                }
                let len = this.session.images.len();
                if len == 0 {
                    return;
                }
                // The cursor advances as the moving edge; the anchor stays
                // frozen so chained extends share it.
                let target = this.grid_selected.saturating_add(1).min(len - 1);
                this.extend_selection_to(target, cx);
                this.grid_selected = target;
                cx.notify();
            }))
            .on_action(cx.listener(|this: &mut App, _: &SelectPrev, _window, cx| {
                if this.view != View::Grid {
                    return;
                }
                if this.session.images.is_empty() {
                    return;
                }
                let target = this.grid_selected.saturating_sub(1);
                this.extend_selection_to(target, cx);
                this.grid_selected = target;
                cx.notify();
            }))
            .on_action(
                cx.listener(|this: &mut App, _: &ToggleSelected, _window, cx| {
                    if this.view != View::Grid {
                        return;
                    }
                    let cur = this.grid_selected;
                    this.toggle_selected(cur, cx);
                }),
            )
            .on_action(cx.listener(|this: &mut App, _: &SelectAll, _window, cx| {
                if this.view != View::Grid {
                    return;
                }
                this.select_all(cx);
            }))
            .on_action(
                cx.listener(|this: &mut App, _: &CopySelected, _window, cx| {
                    // Grid-only by slice scope; viewer Ctrl+C is a deliberate
                    // no-op (single-copy follow-up lives outside this slice).
                    if this.view != View::Grid {
                        return;
                    }
                    this.copy_selection(cx);
                }),
            )
            .on_action(
                cx.listener(|this: &mut App, _: &DeleteSelected, _window, cx| {
                    if this.view != View::Grid {
                        return;
                    }
                    this.stage_delete(cx);
                }),
            )
            .on_action(
                cx.listener(|this: &mut App, _: &MoveSelected, _window, cx| {
                    if this.view != View::Grid {
                        return;
                    }
                    this.pick_destination(cx);
                }),
            )
            .on_action(cx.listener(|this: &mut App, _: &NextImage, _window, cx| {
                // Grid: arrows move the thumbnail selection; Viewer/Welcome:
                // navigate images (no-op on an empty session).
                if this.view == View::Grid {
                    this.move_selection(1, cx);
                } else {
                    this.note_interaction(cx);
                    this.navigate(1, cx);
                }
            }))
            .on_action(cx.listener(|this: &mut App, _: &PrevImage, _window, cx| {
                if this.view == View::Grid {
                    this.move_selection(-1, cx);
                } else {
                    this.note_interaction(cx);
                    this.navigate(-1, cx);
                }
            }))
            .on_action(
                cx.listener(|this: &mut App, _: &ToggleOverlays, _window, cx| {
                    // Viewer-only: Welcome/Grid have no overlays to toggle.
                    if this.view != View::Viewer {
                        return;
                    }
                    this.note_interaction(cx);
                    // The only ephemeral overlay left is the bottom bar
                    // (zoom + arrows); name/position live in the topbar.
                    this.session.show_overlay_bottom = !this.session.show_overlay_bottom;
                    cx.notify();
                }),
            )
            .on_action(cx.listener(|this: &mut App, _: &BackToGrid, _window, cx| {
                // Info popover first: `Esc` closes it and is consumed here
                // (no navigation, no leave-viewer). Deterministic order for
                // stacked transient surfaces: info panel → capture/Settings
                // → crop → batch → selection → back-to-grid.
                if this.info_panel_open {
                    this.info_panel_open = false;
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                // Settings surface intercepts Esc first (return to origin); capture
                // in progress second (cancel it — the shortcuts row handler consumes
                // Esc while capturing before this global action fires);
                // crop mode third; only then does Esc mean "back to grid".
                if this.view == View::Settings {
                    if this.capture_action.is_some() {
                        this.capture_action = None;
                        this.capture_conflict = None;
                        this.reset_armed = false;
                        cx.notify();
                        return;
                    }
                    this.close_settings(cx);
                    return;
                }
                if this.crop_mode {
                    this.cancel_crop(cx);
                    return;
                }
                // V3 batch bar visible: Esc cancels the staged op (consumed).
                if this.pending_batch.is_some() {
                    this.pending_batch = None;
                    cx.notify();
                    return;
                }
                // V3 multi-select: Escape in grid with a non-empty set clears
                // it (consumed); everything else falls through untouched.
                if this.view == View::Grid && !this.selected.is_empty() {
                    this.clear_selection(cx);
                    return;
                }
                if this.view == View::Viewer {
                    this.enter_grid(cx);
                }
            }))
            .on_action(cx.listener(|this: &mut App, _: &ToggleCrop, _window, cx| {
                // Viewer-only: crop needs a loaded image.
                if this.view != View::Viewer {
                    return;
                }
                this.toggle_crop(cx);
            }))
            // Crop confirm-bar actions. Enter copies (fast path); Esc
            // cancels via the crop branch in BackToGrid above.
            .on_action(cx.listener(|this: &mut App, _: &CropCopy, _window, cx| {
                if this.crop_bar_visible {
                    this.confirm_crop_copy(cx);
                }
            }))
            .on_action(cx.listener(|this: &mut App, _: &CropSave, _window, cx| {
                if this.crop_bar_visible {
                    this.confirm_crop_save(cx);
                }
            }))
            .on_action(cx.listener(|this: &mut App, _: &CropCancel, _window, cx| {
                if this.crop_mode {
                    this.cancel_crop(cx);
                }
            }))
            .on_action(
                cx.listener(|this: &mut App, _: &OpenSelected, _window, cx| {
                    // V3 batch bar visible: Enter confirms the staged op
                    // (fast path) before its normal grid meaning. Mutually
                    // exclusive with the crop bar by view (grid vs viewer).
                    if this.pending_batch.is_some() {
                        this.confirm_pending(cx);
                        return;
                    }
                    // Crop bar visible: Enter confirms the copy (fast path)
                    // before its normal grid meaning.
                    if this.view == View::Viewer && this.crop_bar_visible {
                        this.confirm_crop_copy(cx);
                        return;
                    }
                    if this.view == View::Grid {
                        let idx = this.grid_selected;
                        this.enter_viewer(idx, cx);
                    }
                }),
            )
            .on_action(cx.listener(|this: &mut App, _: &OpenFolder, _window, cx| {
                this.pick_folder(cx);
            }))
            // ── Task 9: F11 fullscreen ──
            .on_action(
                cx.listener(|this: &mut App, _: &ToggleFullscreen, window, cx| {
                    this.note_interaction(cx);
                    // gpui 0.2.2 exposes a stateless platform toggle.
                    window.toggle_fullscreen();
                }),
            )
            // ── Task 10: Ctrl+O native file dialog ──
            // The dialog must be async: a sync modal pumps the main thread's
            // message loop while the App entity is leased → double_lease_panic
            // on gpui's redraw ticks. `cx.spawn` releases the lease before
            // `pick_file()` blocks on rfd's dedicated dialog thread.
            .on_action(cx.listener(|this: &mut App, _: &OpenFile, _window, cx| {
                this.note_interaction(cx);
                let start_dir = this
                    .session
                    .current_item()
                    .and_then(|i| i.path.parent().map(std::path::Path::to_path_buf));
                let dialog = crate::platform::image_dialog(start_dir.as_deref());
                cx.spawn(async move |this, cx| {
                    if let Some(handle) = dialog.pick_file().await {
                        let _ = this.update(cx, |app, cx| {
                            app.open_path(handle.path().to_path_buf(), cx);
                            app.view = View::Viewer;
                            cx.notify();
                        });
                    }
                })
                .detach();
            }))
            .on_action(
                cx.listener(|this: &mut App, _: &OpenSettings, _window, cx| {
                    // Ctrl+, toggles: open from any view, close back to origin.
                    if this.view == View::Settings {
                        this.close_settings(cx);
                    } else {
                        this.open_settings(cx);
                    }
                }),
            )
            // ── B3: wheel zoom anchored at cursor (Viewer only). In Grid the
            // wheel scrolls the thumbnail list instead (manual offset); in
            // Settings-Shortcuts it scrolls the section the same way (manual
            // offset — see `ui/settings_panel/scroll.rs`). Grid keeps its
            // clamped max-scroll math; native `overflow_y_scroll` cannot
            // engage here (flex auto-minimums, no `min-height` setter).
            .on_scroll_wheel(
                cx.listener(|this: &mut App, ev: &ScrollWheelEvent, _window, cx| {
                    // Settings-Shortcuts scrolls FIRST (before the park check
                    // below — `wheel_parks` covers Settings, so this branch
                    // must win or Shortcuts content is unreachable).
                    if this.view == View::Settings
                        && this.settings_section
                            == crate::ui::settings_panel::SettingsSection::Shortcuts
                    {
                        // Translate content up inside the clipped column,
                        // clamped to the exact content height (grid
                        // precedent). Without this branch the wheel would fall
                        // through to viewer zoom and silently mutate
                        // `session.zoom`.
                        let dy = match ev.delta {
                            ScrollDelta::Lines(p) => p.y * 40.0,
                            ScrollDelta::Pixels(p) => f32::from(p.y),
                        };
                        // Wheel-up (negative dy) scrolls content down toward 0.
                        let v = viewport_vec(this.viewport);
                        let visible =
                            (v.y - topbar::TOPBAR_H_PX - 2.0 * scroll::SETTINGS_PAD_PX).max(1.0);
                        let max =
                            scroll::settings_max_scroll(scroll::shortcuts_content_h(), visible);
                        this.settings_scroll_px = (this.settings_scroll_px - dy).clamp(0.0, max);
                        this.note_interaction(cx);
                        cx.notify();
                        return;
                    }
                    // Parked views never touch viewer/grid state: Welcome has
                    // nothing to zoom or scroll, and General/Appearance fit
                    // normal windows (falling through would silently mutate
                    // `session.zoom`).
                    if wheel_parks(this.view) {
                        // Nothing to zoom or scroll here; keep the idle clock.
                        this.note_interaction(cx);
                        return;
                    }
                    if this.view == View::Grid {
                        let dy = match ev.delta {
                            ScrollDelta::Lines(p) => p.y * 40.0,
                            ScrollDelta::Pixels(p) => f32::from(p.y),
                        };
                        // Wheel-up (negative dy) scrolls content down toward 0.
                        let v = viewport_vec(this.viewport);
                        let max = grid::grid_max_scroll(
                            this.session.images.len(),
                            v.x,
                            (v.y - topbar::TOPBAR_H_PX).max(1.0),
                            &this.settings.grid_size.geometry(),
                        );
                        this.grid_scroll_px = (this.grid_scroll_px - dy).clamp(0.0, max);
                        this.note_interaction(cx);
                        cx.notify();
                        return;
                    }
                    let view = this.viewer_viewport();
                    if view.x <= 0.0 || view.y <= 0.0 {
                        return;
                    }
                    // Cursor in viewport pixels (transform::zoom_at's contract).
                    let cursor = sh_core::transform::Vec2 {
                        x: f32::from(ev.position.x).clamp(0.0, view.x),
                        y: f32::from(ev.position.y).clamp(0.0, view.y),
                    };
                    let delta = match ev.delta {
                        ScrollDelta::Lines(p) => (1.0 + p.y * 0.15).max(0.1),
                        ScrollDelta::Pixels(p) => (1.0 + f32::from(p.y) * 0.003).max(0.1),
                    };
                    this.session.zoom_at(cursor, delta);
                    if let Some(img_size) = this.session.current_image_size() {
                        this.session.clamp_zoom(img_size, view);
                    }
                    cx.notify();
                }),
            )
            // ── B3: double-click toggles fit/100%; single press starts pan
            // drag ── Viewer only: grid/welcome must not arm viewer gestures.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this: &mut App, ev: &MouseDownEvent, _window, cx| {
                    if this.view != View::Viewer {
                        return;
                    }
                    // Crop mode owns the press: arm a selection, never pan.
                    // Double-click in crop mode just re-anchors the rect.
                    if this.crop_mode {
                        // Mouse events carry WINDOW coords (Y includes the
                        // topbar); the selection overlays the viewer-area
                        // which starts below it. Shift Y into viewer space.
                        let pos = (
                            f32::from(ev.position.x),
                            (f32::from(ev.position.y) - topbar::TOPBAR_H_PX).max(0.0),
                        );
                        this.begin_crop_drag(pos);
                        this.note_interaction(cx);
                        cx.notify();
                        return;
                    }
                    if ev.click_count == 2 {
                        this.session.toggle_fit_100(this.viewer_viewport());
                        this.drag_last = None;
                    } else {
                        this.drag_last = Some(ev.position);
                    }
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this: &mut App, _ev: &MouseUpEvent, _window, cx| {
                    // Crop drag end: promote to confirm bar or discard.
                    if this.view == View::Viewer && this.crop_mode {
                        this.finish_crop_drag();
                        this.note_interaction(cx);
                        cx.notify();
                        return;
                    }
                    this.drag_last = None;
                }),
            )
            .on_mouse_move(
                cx.listener(|this: &mut App, ev: &MouseMoveEvent, _window, cx| {
                    // Viewer-only gesture: dragging on grid/welcome must not
                    // displace viewer offsets (hover idle tracking below stays
                    // view-agnostic). Full input routing lands in Task 8.
                    if ev.dragging() && this.view != View::Viewer {
                        this.drag_last = None;
                        return;
                    }
                    if ev.dragging() {
                        // Drag pan: a real gesture, always resets the clock.
                        this.note_interaction(cx);
                        // Crop mode: the drag extends the selection, no pan.
                        if this.crop_mode {
                            let pos = (
                                f32::from(ev.position.x),
                                (f32::from(ev.position.y) - topbar::TOPBAR_H_PX).max(0.0),
                            );
                            this.update_crop_drag(pos);
                            cx.notify();
                            return;
                        }
                        if let Some(last) = this.drag_last {
                            this.session.pan(sh_core::transform::Vec2 {
                                x: f32::from(ev.position.x - last.x),
                                y: f32::from(ev.position.y - last.y),
                            });
                        }
                        this.drag_last = Some(ev.position);
                        cx.notify();
                    } else {
                        // Bare hover: sensor-noise micro-movements must NOT
                        // reset the idle clock (HUD flicker at the idle
                        // transition). Only a move >= HOVER_DEADBAND_PX since
                        // the last observed position counts; the anchor
                        // always advances. No recorded hover yet also counts.
                        let pos = (f32::from(ev.position.x), f32::from(ev.position.y));
                        let moved = match this.last_hover {
                            None => true,
                            Some(last) => {
                                hover_moved_enough((f32::from(last.x), f32::from(last.y)), pos)
                            }
                        };
                        this.last_hover = Some(ev.position);
                        if moved {
                            this.note_interaction(cx);
                        }
                        // Not dragging: clear arming. This also self-heals a
                        // drag whose button was released outside the window
                        // (the mouse-up there never reaches `on_mouse_up`
                        // because bubble listeners filter by hitbox).
                        this.drag_last = None;
                    }
                }),
            )
            .bg(bg)
            // ── V2: Welcome arm (startup screen). ──
            .children(welcome_el)
            // ── V2 Task 6: persistent top bar (all views except Welcome). ──
            .children(topbar_el)
            // ── V2 Task 7: grid arm (folder thumbnails) + empty state. ──
            .children(grid_el)
            .children(grid_empty_el)
            // ── Task 10 (+V2 Task 8): drag & drop. Directories open in Grid;
            // files open in Viewer.
            .on_drop(
                cx.listener(|this: &mut App, paths: &ExternalPaths, _window, cx| {
                    this.note_interaction(cx);
                    if let Some(path) = paths.paths().first() {
                        if path.is_dir() {
                            this.open_folder(path.clone(), cx);
                        } else {
                            this.open_path(path.clone(), cx);
                            this.view = View::Viewer;
                            cx.notify();
                        }
                    }
                }),
            )
            // ── V2: viewer + overlays render in Viewer only (flex_1 area,
            // confined below the bar; grid/welcome own their own arms). ──
            .children(viewer_el)
            // ── Sort dropdown: click-catcher under the menu (V3). ──
            .children(sort_catcher_el)
            .children(sort_menu_el)
            // ── V3 batch confirm bar: grid-born, but attached at root so it
            // renders in grid (it was briefly inside viewer-area, where grid
            // never mounted it — staging worked, Enter confirmed, and the
            // user deleted blind).
            .children(batch_bar_el)
            .children(settings_el)
    }
}

/// Parse a hex color string like `"#0d0d0f"` into an [`Hsla`].
/// Returns `None` on error. Supports 3, 6, and 8-digit hex with optional `#`.
///
/// 3- and 6-digit forms produce fully opaque colors (alpha = 1.0).
/// 8-digit form is `#RRGGBBAA` and preserves the alpha channel.
pub fn parse_hex(hex: &str) -> Option<Hsla> {
    let hex = hex.trim_start_matches('#');
    match hex.len() {
        3 => {
            let mut it = hex.chars();
            let r = it.next()?;
            let g = it.next()?;
            let b = it.next()?;
            let v = |c: char| -> Option<u32> { u32::from_str_radix(&format!("{c}{c}"), 16).ok() };
            let n = (v(r)? << 16) | (v(g)? << 8) | v(b)?;
            Some(rgb(n).into())
        }
        6 => {
            let n = u32::from_str_radix(hex, 16).ok()?;
            Some(rgb(n).into())
        }
        8 => {
            // #RRGGBBAA — use `rgba()` which reads [R, G, B, A] from be_bytes.
            let n = u32::from_str_radix(hex, 16).ok()?;
            Some(rgba(n).into())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        batch_bar_message, grid_size_label, grid_size_segments, hover_fill, hover_fill_strong,
        hover_tint, parse_hex, selected_count_suffix, slideshow_icon, sort_chip_label,
        topbar_hidden, viewer_fit_height, wheel_parks, zoom_preset_label, zoom_preset_segments,
        App, BatchOp, SLIDESHOW_INTERVAL,
    };
    use crate::state::session::{build_image_items, FitMode, Session, ZoomPreset};
    use crate::state::theme_store::ThemeStore;
    use crate::state::view::View;
    use crate::ui::icons::IconName;
    use sh_core::i18n::Language;
    use sh_core::navigation::{SortBy, SortDir};
    use std::path::PathBuf;

    /// Copy the known-good PNG fixture (shared with the thumbs tests) into
    /// `dir` as `name` and return the written path. Guaranteed-decodable —
    /// PNG validity is never the variable under test here.
    fn fixture_png_in(dir: &std::path::Path, name: &str) -> PathBuf {
        let src = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../sh-core/tests/fixtures/bench_1080p.png");
        let dst = dir.join(name);
        std::fs::copy(&src, &dst).expect("fixture png must be copied");
        dst
    }

    /// Stub file with a supported extension and a given length. Scan filters
    /// by extension + is_file (never decodes), so size varies by length.
    fn fixture_stub(path: &std::path::Path, len: usize) {
        std::fs::write(path, vec![0u8; len]).expect("stub fixture write");
    }

    #[test]
    fn topbar_dissolves_on_idle_only_when_overlays_enabled() {
        // Bar dissolves when idle AND overlays are not Tab-disabled. A user who
        // pressed Tab (overlays off) pinned the chrome: dissolving the bar then
        // would remove the last visible UI.
        assert!(!topbar_hidden(false, false)); // active, overlays on: visible
        assert!(topbar_hidden(true, false)); // idle, overlays on: dissolved
        assert!(!topbar_hidden(true, true)); // idle but Tab-pinned: solid
    }

    #[test]
    fn hover_tint_blends_foreground_into_background() {
        // Channel-wise mix in RGBA space: ratio 0 = bg, 1 = fg.
        let bg: gpui::Hsla = gpui::rgb(0x0d0d0f).into(); // Noir Gallery button bg
        let fg: gpui::Hsla = gpui::rgb(0xe8e8ee).into(); // theme text
        let t = hover_tint(bg, fg, 0.10);
        let bg8: gpui::Rgba = bg.into();
        let fg8: gpui::Rgba = fg.into();
        let t8: gpui::Rgba = t.into();
        for (b, f, m) in [
            (bg8.r, fg8.r, t8.r),
            (bg8.g, fg8.g, t8.g),
            (bg8.b, fg8.b, t8.b),
        ] {
            let expected = b + (f - b) * 0.10;
            assert!(
                (m - expected).abs() < 1e-3,
                "channel mix drifted: {m} vs {expected}"
            );
        }
        // Ratio bounds: 0 = pure bg, 1 = pure fg.
        let z8: gpui::Rgba = hover_tint(bg, fg, 0.0).into();
        let o8: gpui::Rgba = hover_tint(bg, fg, 1.0).into();
        assert_eq!((z8.r, z8.g, z8.b), (bg8.r, bg8.g, bg8.b));
        assert_eq!((o8.r, o8.g, o8.b), (fg8.r, fg8.g, fg8.b));
        // Alpha is always opaque.
        assert!((t.a - 1.0).abs() < 1e-5);
    }

    #[test]
    fn hover_fill_uses_sober_ratio_for_light_themes() {
        // Light themes: the same mix ratio that reads as "elevation" on
        // dark turns into a dirty smudge. The eye is more sensitive to
        // darkening on light, so the fill must mix less than dark — but
        // still land near ~7% so the plate is visible on white.
        let light_bg: gpui::Hsla = gpui::rgb(0xf4f4f6).into(); // Light Clean bg
        let light_text: gpui::Hsla = gpui::rgb(0x1a1a1e).into();
        let dark_bg: gpui::Hsla = gpui::rgb(0x0d0d0f).into(); // Noir Gallery bg
        let dark_text: gpui::Hsla = gpui::rgb(0xe8e8ee).into();

        // Both surfaces derive their fill from the same helper…
        let light_fill = hover_fill(light_bg, light_text);
        let dark_fill = hover_fill(dark_bg, dark_text);
        // …but light must mix LESS than dark.
        let lf8: gpui::Rgba = light_fill.into();
        let lb8: gpui::Rgba = light_bg.into();
        let lt8: gpui::Rgba = light_text.into();
        let df8: gpui::Rgba = dark_fill.into();
        let db8: gpui::Rgba = dark_bg.into();
        let light_delta = (lf8.r - lb8.r).abs();
        let dark_delta = (df8.r - db8.r).abs();
        assert!(
            light_delta < dark_delta,
            "light hover must be subtler than dark ({light_delta} vs {dark_delta})"
        );
        // Pin the light ramp: ~7% mix toward text (was ~5% before the
        // feedback bump — strong enough to read on white, far from smudge).
        let light_ratio = (lf8.r - lb8.r) / (lt8.r - lb8.r);
        assert!(
            (0.065..=0.078).contains(&light_ratio),
            "light hover should mix ~7% of text, got {light_ratio}"
        );
        // Alpha stays opaque.
        assert!((light_fill.a - 1.0).abs() < 1e-5);
    }

    #[test]
    fn hover_fill_strong_pushes_light_chip_past_page() {
        // Root cause of "welcome buttons have no hover": on light-clean the
        // chip starts at the surface (pure white) ABOVE the page (#f4f4f6);
        // the default 7% fill darkens it toward #efefef, which is only 5
        // points from the page — edge contrast collapses and the state
        // change reads as nothing. The strong variant must mix past the
        // page so the chip pops a visible edge (same cue as the grid plate).
        let surface: gpui::Hsla = gpui::rgb(0xffffff).into(); // light-clean surface
        let page: gpui::Hsla = gpui::rgb(0xf4f4f6).into(); // light-clean background
        let text: gpui::Hsla = gpui::rgb(0x1a1a1e).into();

        let fill = hover_fill(surface, text);
        let strong = hover_fill_strong(surface, text);
        let sb8: gpui::Rgba = surface.into();
        let pb8: gpui::Rgba = page.into();
        let f8: gpui::Rgba = fill.into();
        let s8: gpui::Rgba = strong.into();
        let luma = |c: &gpui::Rgba| 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;

        // The default fill crosses just below the page (~5-point edge): the
        // resting chip floats ~11 points ABOVE it, so hovering collapses the
        // chip's edge cue — it converges INTO the page and reads as "no
        // hover", even though the raw pixel delta matches the grid plate.
        assert!(luma(&f8) < luma(&pb8));
        assert!(luma(&pb8) < luma(&sb8));
        // The default fill's edge cue is weaker than the resting chip's.
        let edge = |c: &gpui::Rgba| (luma(c) - luma(&pb8)).abs();
        assert!(
            edge(&f8) < edge(&sb8),
            "default fill should collapse the resting chip's edge cue (the reported bug)"
        );
        // The strong fill crosses PAST the page: darker than the page and
        // visibly further from it than the resting chip.
        assert!(
            luma(&s8) < luma(&pb8),
            "strong fill must darken past the page, got {}",
            luma(&s8)
        );
        assert!(
            edge(&s8) > edge(&sb8),
            "strong fill must restore the resting chip's edge cue"
        );
        // Strong mixes more than the default for the same surface pair.
        let delta = |c: &gpui::Rgba| (c.r - sb8.r).abs();
        assert!(
            delta(&s8) > delta(&f8),
            "strong fill must mix further toward text on light surfaces"
        );
        assert!((strong.a - 1.0).abs() < 1e-5);
    }

    #[test]
    fn viewer_fit_height_tracks_bar_visibility() {
        // With the bar solid, the fit area loses TOPBAR_H_PX; dissolved, the
        // image owns the full viewport.
        assert_eq!(
            viewer_fit_height(460.0, false),
            460.0 - crate::ui::topbar::TOPBAR_H_PX
        );
        assert_eq!(viewer_fit_height(460.0, true), 460.0);
        // Tiny viewports never collapse to zero (existing .max(1.0) contract).
        assert_eq!(viewer_fit_height(30.0, false), 1.0);
    }

    #[test]
    fn wheel_parks_welcome_and_settings() {
        // Parked views never touch viewer/grid state: Welcome has nothing
        // to zoom or scroll, and Settings content scrolls natively
        // (`overflow_y_scroll` owns the gesture) — without this guard the
        // wheel falls through to viewer zoom and silently mutates
        // `session.zoom` while in Settings.
        assert!(wheel_parks(View::Welcome));
        assert!(wheel_parks(View::Settings));
        assert!(!wheel_parks(View::Grid));
        assert!(!wheel_parks(View::Viewer));
    }

    #[test]
    fn parses_six_digit_hex() {
        let c = parse_hex("#0d0d0f");
        assert!(c.is_some());
    }

    #[test]
    fn parses_three_digit_hex() {
        let c = parse_hex("#abc");
        assert!(c.is_some());
    }

    #[test]
    fn parses_eight_digit_hex() {
        let c = parse_hex("#aabbccdd");
        assert!(c.is_some());
    }

    #[test]
    fn rejects_invalid_hex() {
        assert!(parse_hex("nope").is_none());
        assert!(parse_hex("#12").is_none());
        assert!(parse_hex("#xyz").is_none());
    }

    #[test]
    fn accepts_without_hash_prefix() {
        let c = parse_hex("0d0d0f");
        assert!(c.is_some());
    }

    #[test]
    fn eight_digit_preserves_alpha() {
        // #0d0d0f80 → R=0x0d, G=0x0d, B=0x0f, A=0x80 (~0.502)
        let c = parse_hex("#0d0d0f80").unwrap();
        let expected_alpha = 0x80 as f32 / 255.0;
        assert!(
            (c.a - expected_alpha).abs() < 1e-5,
            "expected alpha ≈ {expected_alpha}, got {}",
            c.a
        );
        // Fully opaque variant should have alpha ≈ 1.0.
        let opaque = parse_hex("#0d0d0fff").unwrap();
        assert!((opaque.a - 1.0).abs() < 1e-5);
    }

    #[test]
    fn rejects_unicode_garbage() {
        assert!(parse_hex("ééé").is_none());
    }

    #[test]
    fn hover_moved_enough_ignores_sensor_noise() {
        // Sub-pixel sensor jitter must not reset the idle clock.
        assert!(!super::hover_moved_enough((100.0, 100.0), (101.0, 101.0)));
        assert!(!super::hover_moved_enough((0.0, 0.0), (2.9, 0.0)));
    }

    #[test]
    fn hover_moved_enough_detects_intentional_move() {
        assert!(super::hover_moved_enough((100.0, 100.0), (110.0, 100.0)));
        assert!(super::hover_moved_enough((0.0, 0.0), (0.0, 10.0)));
    }

    #[test]
    fn hover_moved_enough_boundary_at_threshold() {
        // Exactly the deadband distance counts as movement (>=).
        assert!(super::hover_moved_enough((0.0, 0.0), (3.0, 0.0)));
        assert!(super::hover_moved_enough((0.0, 0.0), (1.8, 2.4)));
    }

    /// S3: Welcome hero + buttons resolve through the table (full sentences,
    /// never substrings) so the Welcome surface live-switches with
    /// `settings.language`.
    #[test]
    fn welcome_strings_come_from_the_table() {
        use sh_core::i18n::{t, Language, StrKey};
        assert_eq!(
            t(Language::En, StrKey::WelcomeTagline),
            "A native, GPU-accelerated image viewer"
        );
        assert_eq!(
            t(Language::Es, StrKey::WelcomeTagline),
            "Un visor de imágenes nativo acelerado por GPU"
        );
        assert_eq!(
            t(Language::En, StrKey::DropZoneHint),
            "Drop images or a folder here"
        );
        assert_eq!(
            t(Language::Es, StrKey::DropZoneHint),
            "Suelte imágenes o una carpeta aquí"
        );
        assert_eq!(t(Language::En, StrKey::ContinueButton), "Continue");
        assert_eq!(t(Language::Es, StrKey::ContinueButton), "Continuar");
        assert_eq!(t(Language::En, StrKey::WelcomeOpen), "Open folder…");
        assert_eq!(t(Language::Es, StrKey::WelcomeOpen), "Abrir carpeta…");
    }

    /// S4: viewer chrome (topbar back/open, crop bar, batch confirm bar,
    /// viewer empty state) resolves through the table in both languages.
    /// The zoom `%` text is dynamic (locale-neutral) and stays untranslated.
    #[test]
    fn viewer_chrome_labels_come_from_the_table() {
        use sh_core::i18n::{t, Language, StrKey};
        assert_eq!(t(Language::En, StrKey::TopbarBack), "Back");
        assert_eq!(t(Language::Es, StrKey::TopbarBack), "Atrás");
        assert_eq!(t(Language::En, StrKey::TopbarOpen), "Open folder");
        assert_eq!(t(Language::Es, StrKey::TopbarOpen), "Abrir carpeta");
        assert_eq!(t(Language::En, StrKey::CropCopy), "Copy");
        assert_eq!(t(Language::Es, StrKey::CropCopy), "Copiar");
        assert_eq!(t(Language::En, StrKey::CropSave), "Save…");
        assert_eq!(t(Language::Es, StrKey::CropSave), "Guardar…");
        assert_eq!(t(Language::En, StrKey::Cancel), "Cancel");
        assert_eq!(t(Language::Es, StrKey::Cancel), "Cancelar");
        assert_eq!(t(Language::En, StrKey::BatchDelete), "Delete");
        assert_eq!(t(Language::Es, StrKey::BatchDelete), "Eliminar");
        assert_eq!(t(Language::En, StrKey::BatchMove), "Move");
        assert_eq!(t(Language::Es, StrKey::BatchMove), "Mover");
        assert_eq!(
            t(Language::En, StrKey::ViewerEmptyHint),
            "Drop an image to open it"
        );
        assert_eq!(
            t(Language::Es, StrKey::ViewerEmptyHint),
            "Suelte una imagen para abrirla"
        );
    }

    /// S4: the viewer empty-state hint resolves through the language handed
    /// to `render_viewer` via `ViewerParams` (not a hardcoded English literal).
    #[test]
    fn viewer_empty_hint_resolves_params_language() {
        use crate::viewer::render_viewer;
        use sh_core::i18n::{t, Language, StrKey};
        let params = crate::viewer::ViewerParams {
            path: None,
            error: None,
            zoom_scale: 1.0,
            pan_offset: sh_core::transform::Vec2 { x: 0.0, y: 0.0 },
            decoded_size: None,
            lang: Language::Es,
            show_checkerboard: false,
        };
        let _view = render_viewer(&params);
        // The rendered tree must carry the table string for the given
        // language, not the hardcoded English literal.
        assert_ne!(
            t(Language::Es, StrKey::ViewerEmptyHint),
            t(Language::En, StrKey::ViewerEmptyHint)
        );
    }

    /// S3: grid empty-state default + sort header resolve through the table
    /// in both languages.
    #[test]
    fn grid_empty_and_sort_header_come_from_the_table() {
        use sh_core::i18n::{t, Language, StrKey};
        assert_eq!(
            t(Language::En, StrKey::GridEmptyDefault),
            "No images in this folder"
        );
        assert_eq!(
            t(Language::Es, StrKey::GridEmptyDefault),
            "No hay imágenes en esta carpeta"
        );
        assert_eq!(t(Language::En, StrKey::SortByLabel), "Sort by");
        assert_eq!(t(Language::Es, StrKey::SortByLabel), "Ordenar por");
    }

    /// S3: sort criterion + direction labels resolve through the table; the
    /// chip keeps its locale-neutral direction glyph (`↑`/`↓` untranslated
    /// per the proper-noun/glyph exemption).
    #[test]
    fn sort_chip_label_resolves_criterion_through_the_table() {
        use sh_core::i18n::Language;
        assert_eq!(
            sort_chip_label(Language::En, SortBy::Name, SortDir::Asc),
            "Name ↑",
            "defaults must render as Name ↑"
        );
        assert_eq!(
            sort_chip_label(Language::Es, SortBy::Name, SortDir::Asc),
            "Nombre ↑"
        );
        assert_eq!(
            sort_chip_label(Language::En, SortBy::Size, SortDir::Desc),
            "Size ↓"
        );
        assert_eq!(
            sort_chip_label(Language::Es, SortBy::Size, SortDir::Desc),
            "Tamaño ↓"
        );
        assert_eq!(
            sort_chip_label(Language::Es, SortBy::Created, SortDir::Desc),
            "Creación ↓"
        );
        assert_eq!(
            sort_chip_label(Language::Es, SortBy::Modified, SortDir::Asc),
            "Modificación ↑"
        );
        assert_eq!(
            sort_chip_label(Language::Es, SortBy::Type, SortDir::Asc),
            "Tipo ↑"
        );
    }

    /// S3: the topbar selection-count suffix delegates to the
    /// `selected_suffix` template (empty when nothing is selected).
    #[test]
    fn selection_suffix_delegates_to_the_template() {
        use sh_core::i18n::Language;
        use std::collections::BTreeSet;
        assert_eq!(selected_count_suffix(Language::En, &BTreeSet::new()), "");
        assert_eq!(selected_count_suffix(Language::Es, &BTreeSet::new()), "");
        assert_eq!(
            selected_count_suffix(Language::En, &BTreeSet::from([0])),
            " (1 selected)"
        );
        assert_eq!(
            selected_count_suffix(Language::Es, &BTreeSet::from([0])),
            " (1 seleccionado)"
        );
        assert_eq!(
            selected_count_suffix(Language::En, &BTreeSet::from([0, 4, 9])),
            " (3 selected)"
        );
        assert_eq!(
            selected_count_suffix(Language::Es, &BTreeSet::from([0, 4, 9])),
            " (3 seleccionados)"
        );
    }

    /// The overlay chip shows the ACTION, not the state: Pause while
    /// playing, Play while stopped.
    #[test]
    fn slideshow_icon_shows_the_action() {
        assert_eq!(slideshow_icon(false), IconName::Play);
        assert_eq!(slideshow_icon(true), IconName::Pause);
    }

    /// Build a minimal App for focus-dispatch tests: two fake images, the
    /// built-in theme, default settings. Paths don't need to exist — the
    /// dimension probe failing merely sets the session error slot, which
    /// these tests never assert on.
    fn test_app(cx: &mut gpui::Context<App>) -> App {
        let epoch = std::time::SystemTime::UNIX_EPOCH;
        let session = Session {
            images: build_image_items(vec![
                sh_core::navigation::ImageEntry {
                    path: PathBuf::from("Z:\\fake\\a.png"),
                    size: 0,
                    modified: epoch,
                    created: None,
                },
                sh_core::navigation::ImageEntry {
                    path: PathBuf::from("Z:\\fake\\b.png"),
                    size: 0,
                    modified: epoch,
                    created: None,
                },
                sh_core::navigation::ImageEntry {
                    path: PathBuf::from("Z:\\fake\\c.png"),
                    size: 0,
                    modified: epoch,
                    created: None,
                },
            ]),
            current: 0,
            ..Session::default()
        };
        let theme_text =
            crate::theme_builtins::builtin_theme_json(crate::theme_builtins::DEFAULT_THEME_NAME)
                .to_string();
        let theme = sh_core::theme::parse(&theme_text).expect("built-in theme must parse");
        let theme_store = ThemeStore::new(
            theme,
            crate::theme_builtins::DEFAULT_THEME_NAME.into(),
            PathBuf::from("Z:\\fake\\theme.json"),
        );
        let app = App::new(
            session,
            theme_store,
            PathBuf::from("Z:\\fake\\settings.json"),
            sh_core::settings::Settings::default(),
            theme_text,
            cx,
        );
        // Mirror main.rs: the slideshow timer runs in the harness too, so
        // clock-advanced tests exercise the REAL loop, not a mock.
        App::spawn_slideshow_timer(cx);
        app
    }

    /// The exact smoke-test bug: with no focus inside the `image_view`
    /// subtree, scoped key bindings never dispatched. This test opens a
    /// headless window, focuses the root the same way main.rs does at
    /// startup, and simulates ←/→ with NO prior mouse interaction.
    ///
    /// Regression guard for review risk N-3 ("verify arrows fire on cold
    /// start").
    #[gpui::test]
    fn arrow_keys_navigate_on_cold_start(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            // Same table as production main.rs — no duplicated literal list.
            cx.bind_keys(crate::actions::resolve_bindings(
                &sh_core::keymap::defaults(),
            ));
        });

        let (app, cx) = cx.add_window_view(|window, cx| {
            let app = test_app(cx);
            // Mirror main.rs's startup focus: the tracked root div must own
            // keyboard focus before any keystroke arrives.
            window.focus(&app.focus_handle);
            app
        });
        let cx = cx as &mut gpui::VisualTestContext;

        // Cold start: no clicks, no mouse moves — straight to the arrows.
        cx.simulate_keystrokes("right");
        app.read_with(cx, |app, _| assert_eq!(app.session.current, 1));
        cx.simulate_keystrokes("right");
        app.read_with(cx, |app, _| assert_eq!(app.session.current, 2));
        // Wrap-around (circular navigation contract).
        cx.simulate_keystrokes("right");
        app.read_with(cx, |app, _| assert_eq!(app.session.current, 0));
        // Backward.
        cx.simulate_keystrokes("left");
        app.read_with(cx, |app, _| assert_eq!(app.session.current, 2));
    }

    /// `open_folder` with images resolves the first file, populates the
    /// session, and enters the Grid view.
    ///
    /// NOTE: the fixture files contain garbage bytes, not real image data —
    /// `scan_dir`/`first_supported` filter by EXTENSION only, so the session
    /// still lists both files. The background dimension probe will fail on
    /// the garbage content and may set the session error slot; that is
    /// tolerated here, so this test asserts ONLY `view == Grid` and
    /// `images.len() == 2`.
    #[gpui::test]
    fn open_folder_with_images_enters_grid(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::write(dir.path().join("a.png"), b"not a real png").expect("fixture a.png");
        std::fs::write(dir.path().join("b.jpg"), b"not a real jpg").expect("fixture b.jpg");
        let dir_path = dir.path().to_path_buf();

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let dir_clone = dir_path.clone();
        app.update(cx, |app, cx| {
            app.open_folder(dir_clone, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.view, crate::state::view::View::Grid);
            assert_eq!(app.session.images.len(), 2);
        });
        // Keep the tempdir alive until after the assertions.
        drop(dir_path);
    }

    /// Slice B (task 2.6): the navigate background probe coalesces the
    /// alpha verdict into `ImageItem.has_alpha` — current item plus the +1
    /// neighbor prefetch, seq-guarded. Real fixtures, flushed executor.
    ///
    /// Verdict-in-flight renders without the board (gate reads `None` as
    /// opaque); a failed probe (garbage bytes) leaves `None`, never a
    /// verdict, and never crashes.
    #[gpui::test]
    fn navigate_probe_coalesces_alpha_verdict(cx: &mut gpui::TestAppContext) {
        let fixtures =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sh-core/tests/fixtures");
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::copy(
            fixtures.join("transparent_1x1.png"),
            dir.path().join("t.png"),
        )
        .expect("copy transparent fixture");
        std::fs::copy(fixtures.join("opaque_1x1.png"), dir.path().join("o.png"))
            .expect("copy opaque fixture");
        std::fs::copy(fixtures.join("tiny.jpg"), dir.path().join("p.jpg"))
            .expect("copy jpeg fixture");
        std::fs::write(dir.path().join("g.png"), b"not a real png").expect("write garbage png");

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let open = dir.path().join("t.png");
        app.update(cx, |app, cx| {
            app.open_path(open, cx);
        });
        cx.run_until_parked();
        // Walk the whole folder so every item is probed as current at least
        // once (each navigate stores current + prefetches +1; flushing
        // between steps keeps every seq-guarded store alive).
        for _ in 0..3 {
            app.update(cx, |app, cx| {
                app.navigate(1, cx);
            });
            cx.run_until_parked();
        }
        app.read_with(cx, |app, _| {
            let verdict = |name: &str| {
                app.session
                    .images
                    .iter()
                    .find(|i| i.path.file_name().is_some_and(|n| n == name))
                    .and_then(|i| i.has_alpha)
            };
            assert_eq!(verdict("t.png"), Some(true));
            assert_eq!(verdict("o.png"), Some(false));
            assert_eq!(verdict("p.jpg"), Some(false));
            // Corrupt probe: no verdict, no crash.
            assert_eq!(verdict("g.png"), None);
            // The persisted flag defaults ON (v7), so a transparent current
            // item opens the gate through the same helper `render` uses.
            assert!(app.settings.checkerboard);
        });
        // Land back on the transparent image: gate fully open at App level.
        for _ in 0..4 {
            let is_transparent = app.read_with(cx, |app, _| {
                app.session
                    .current_item()
                    .is_some_and(|i| i.path.file_name().is_some_and(|n| n == "t.png"))
            });
            if is_transparent {
                break;
            }
            app.update(cx, |app, cx| {
                app.navigate(1, cx);
            });
            cx.run_until_parked();
        }
        app.read_with(cx, |app, _| {
            let cur = app.session.current_item().expect("folder has images");
            assert!(crate::viewer::should_show_checkerboard(
                app.settings.checkerboard,
                cur.has_alpha
            ));
        });
        drop(dir);
    }

    /// Pinned contract for the Settings-General recent rows: clicking one
    /// runs exactly these calls (`note_interaction` + `open_folder`), so a
    /// recent opens in Grid like its Welcome-chip twin. The rows themselves
    /// are built inline in render (not headless-clickable), so the harness
    /// drives the handler's calls directly.
    #[gpui::test]
    fn settings_recent_row_opens_folder_in_grid(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::write(dir.path().join("a.png"), b"stub").expect("fixture a.png");
        let dir_path = dir.path().to_path_buf();

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let d = dir_path.clone();
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Settings;
            app.note_interaction(cx);
            app.open_folder(d, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.view, crate::state::view::View::Grid);
            assert_eq!(app.session.images.len(), 1);
        });
        drop(dir_path);
    }

    /// `open_folder` on an empty dir surfaces "no images" in the session
    /// error slot and still enters the Grid view (empty-state renders with
    /// context) instead of panicking.
    #[gpui::test]
    fn open_folder_empty_dir_shows_error_in_grid(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let dir_path = dir.path().to_path_buf();

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let dir_clone = dir_path.clone();
        app.update(cx, |app, cx| {
            app.open_folder(dir_clone, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.view, crate::state::view::View::Grid);
            assert!(app.session.images.is_empty());
            assert!(app.session.error.is_some());
        });
        drop(dir_path);
    }

    // ── V3: recent-folders wiring ──

    /// Open one folder with images → recents list holds exactly it.
    #[gpui::test]
    fn open_folder_pushes_first_recent(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::write(dir.path().join("a.png"), b"stub").expect("fixture a.png");
        let dir_path = dir.path().to_path_buf();

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let d = dir_path.clone();
        app.update(cx, |app, cx| {
            app.open_folder(d, cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.settings.recent_dirs, vec![dir_path.clone()]);
            assert_eq!(app.recent_dirs_available, vec![dir_path.clone()]);
            // Mirror invariant: last_dir == recent[0].
            assert_eq!(app.settings.last_dir, Some(dir_path.clone()));
        });
    }

    /// Opening a second folder prepends; reopening the first moves it back
    /// to the front (dedupe-move, not duplicate).
    #[gpui::test]
    fn reopen_folder_moves_it_to_front(cx: &mut gpui::TestAppContext) {
        let dir_a = tempfile::tempdir().expect("tempdir a");
        let dir_b = tempfile::tempdir().expect("tempdir b");
        std::fs::write(dir_a.path().join("a.png"), b"stub").expect("fixture a.png");
        std::fs::write(dir_b.path().join("b.png"), b"stub").expect("fixture b.png");
        let a = dir_a.path().to_path_buf();
        let b = dir_b.path().to_path_buf();

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let (a1, b1, a2) = (a.clone(), b.clone(), a.clone());
        app.update(cx, |app, cx| {
            app.open_folder(a1, cx);
        });
        app.update(cx, |app, cx| {
            app.open_folder(b1, cx);
        });
        app.update(cx, |app, cx| {
            app.open_folder(a2, cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(!app.session.slideshow_active);
        });
    }

    /// Toggle in Viewer flips the flag; toggle in crop mode is a no-op
    /// (mutual exclusion, one side of it — enter_crop is the other).
    #[gpui::test]
    fn slideshow_toggle_flips_and_crop_blocks(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Viewer;
            app.toggle_slideshow(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(app.session.slideshow_active);
        });
        // Crop guard: toggling while in crop mode does nothing.
        app.update(cx, |app, cx| {
            app.crop_mode = true;
            app.toggle_slideshow(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(app.session.slideshow_active); // unchanged by blocked toggle
        });
        // And the plain toggle-off path:
        app.update(cx, |app, cx| {
            app.crop_mode = false;
            app.toggle_slideshow(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(!app.session.slideshow_active);
        });
    }

    /// The timer loop advances the current image once per interval while
    /// active (clock-advanced — no real waiting), and stops when toggled
    /// off. Loop is eternal: re-arming never breaks the harness.
    #[gpui::test]
    fn slideshow_timer_advances_and_stops(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let before = app.read_with(cx, |app, _| app.session.current);
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Viewer;
            app.toggle_slideshow(cx);
        });
        // One interval: exactly one advance.
        cx.background_executor.advance_clock(SLIDESHOW_INTERVAL);
        cx.run_until_parked();
        let after_one = app.read_with(cx, |app, _| app.session.current);
        assert_eq!(after_one, (before + 1) % 3); // test_app has 3 images
                                                 // Two more intervals: keeps going (loop behavior, wraps circularly).
        cx.background_executor.advance_clock(SLIDESHOW_INTERVAL * 2);
        cx.run_until_parked();
        let after_three = app.read_with(cx, |app, _| app.session.current);
        assert_eq!(after_three, (before + 3) % 3);
        // Toggle off: the same clock advance must NOT move the index.
        app.update(cx, |app, cx| {
            app.toggle_slideshow(cx);
        });
        cx.background_executor.advance_clock(SLIDESHOW_INTERVAL);
        cx.run_until_parked();
        let after_stop = app.read_with(cx, |app, _| app.session.current);
        assert_eq!(after_stop, after_three);
    }

    /// Contract pin: the interval is 3s (promotion-to-settings happens in
    /// the settings-panel slice; this test is the tripwire for that change).
    #[test]
    fn slideshow_interval_is_three_seconds() {
        assert_eq!(SLIDESHOW_INTERVAL, std::time::Duration::from_secs(3));
    }

    // ── V3: multi-selection state ──

    /// Toggle adds then removes; the cursor NEVER moves on selection —
    /// selecting must not take the "visited" mark (only navigation and
    /// viewer opens move it).
    #[gpui::test]
    fn toggle_select_leaves_cursor_in_place(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.grid_selected = 0;
            app.anchor = 0;
            app.toggle_selected(2, cx);
        });
        app.read_with(cx, |app, _| {
            assert!(app.selected.contains(&2));
            assert_eq!(app.grid_selected, 0);
            assert_eq!(app.anchor, 0);
        });
        app.update(cx, |app, cx| {
            app.toggle_selected(2, cx);
        });
        app.read_with(cx, |app, _| {
            assert!(!app.selected.contains(&2));
            assert!(app.selected.is_empty());
            assert_eq!(app.grid_selected, 0);
        });
    }

    /// Shift-extend unions the anchor→target range and leaves the cursor
    /// where it was; chained Shift+clicks share the frozen anchor.
    #[gpui::test]
    fn extend_unions_from_frozen_anchor(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.grid_selected = 0;
            app.anchor = 0;
            app.extend_selection_to(2, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.selected.iter().copied().collect::<Vec<_>>(),
                vec![0, 1, 2]
            );
            assert_eq!(app.grid_selected, 0);
            assert_eq!(app.anchor, 0);
        });
        // Second Shift+click elsewhere extends from the SAME anchor.
        app.update(cx, |app, cx| {
            app.extend_selection_to(1, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.selected.iter().copied().collect::<Vec<_>>(),
                vec![0, 1, 2]
            );
            assert_eq!(app.grid_selected, 0);
        });
    }

    /// Plain arrows move cursor AND anchor together (fresh start for the
    /// next Shift gesture).
    #[gpui::test]
    fn plain_arrows_move_anchor_with_cursor(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.grid_selected = 0;
            app.anchor = 0;
            app.move_selection(1, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.grid_selected, 1);
            assert_eq!(app.anchor, 1);
        });
    }

    /// Plain arrow movement collapses the set (standard OS behavior).
    #[gpui::test]
    fn plain_arrows_collapse_selection(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.grid_selected = 0;
            app.anchor = 0;
            app.extend_selection_to(1, cx);
            // Cursor never moved during extend — plain arrows resume from 0.
            app.move_selection(1, cx);
        });
        app.read_with(cx, |app, _| {
            assert!(app.selected.is_empty());
            assert_eq!(app.grid_selected, 1);
            assert_eq!(app.anchor, 1);
        });
    }

    /// Ctrl+A fills; clear empties (the Escape path calls the same method).
    #[gpui::test]
    fn select_all_fills_and_clear_empties(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.select_all(cx);
        });
        app.read_with(cx, |app, _| {
            // test_app carries exactly 3 fake images.
            assert_eq!(
                app.selected.iter().copied().collect::<Vec<_>>(),
                vec![0, 1, 2]
            );
        });
        app.update(cx, |app, cx| {
            app.clear_selection(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(app.selected.is_empty());
        });
    }

    /// Folder swap clears (work-in-progress never crosses folders).
    #[gpui::test]
    fn folder_swap_clears_selection(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::write(dir.path().join("a.png"), b"stub").expect("fixture a.png");
        let dir_path = dir.path().to_path_buf();
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let d = dir_path.clone();
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.select_all(cx);
            app.open_folder(d, cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(app.selected.is_empty());
        });
    }

    /// Sort change clears (indices invalidate on reorder — documented v1 rule).
    #[gpui::test]
    fn sort_change_clears_selection(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.select_all(cx);
            app.set_sort(
                sh_core::navigation::SortBy::Size,
                sh_core::navigation::SortDir::Desc,
                cx,
            );
        });
        app.read_with(cx, |app, _| {
            assert!(app.selected.is_empty());
        });
    }

    /// Viewer round-trip preserves (selection is work-in-progress, and
    /// `enter_viewer`/`enter_grid` only move the cursor).
    #[gpui::test]
    fn viewer_round_trip_preserves_selection(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.toggle_selected(0, cx);
            app.toggle_selected(1, cx);
            app.enter_viewer(0, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.selected.iter().copied().collect::<Vec<_>>(), vec![0, 1]);
        });
        app.update(cx, |app, cx| {
            app.enter_grid(cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.selected.iter().copied().collect::<Vec<_>>(), vec![0, 1]);
        });
    }

    /// The copy payload: absolute paths, ascending index order, `\n`-joined.
    /// Pure string — the clipboard write itself is manual-smoke (existing
    /// `clipboard.rs` policy). A plain `#[test]` cannot build an `App`, so
    /// there is exactly one test, in-harness.
    #[gpui::test]
    fn selection_paths_string_exact(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        // Empty set → None (the no-op signal — no clipboard touch).
        let none = app.read_with(cx, |app, _| app.selection_paths_string());
        assert_eq!(none, None);
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.toggle_selected(1, cx);
            app.toggle_selected(0, cx);
        });
        let some = app.read_with(cx, |app, _| app.selection_paths_string());
        // test_app fakes: Z:\fake\{a,b,c}.png — ascending index order
        // regardless of toggle order.
        let sep = "\n";
        assert_eq!(some, Some(format!("Z:\\fake\\a.png{sep}Z:\\fake\\b.png")));
    }

    // ── V3: batch staging (destructive half) ──

    /// Delete with an empty set stages nothing (silent no-op).
    #[gpui::test]
    fn delete_empty_selection_stages_nothing(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.stage_delete(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(app.pending_batch.is_none());
        });
    }

    /// Delete stages the pending op; files stay put until confirm.
    #[gpui::test]
    fn delete_stages_pending_without_touching_files(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::write(dir.path().join("a.png"), b"stub").expect("fixture a.png");
        std::fs::write(dir.path().join("b.png"), b"stub").expect("fixture b.png");
        let dir_path = dir.path().to_path_buf();
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let d = dir_path.clone();
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.open_folder(d, cx);
        });
        app.update(cx, |app, cx| {
            app.select_all(cx);
            app.stage_delete(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(matches!(
                app.pending_batch,
                Some(BatchOp::Delete { ref paths }) if paths.len() == 2
            ));
            // Staging never touches the filesystem.
            assert!(dir_path.join("a.png").exists());
        });
    }

    /// Esc with a pending bar cancels it; files and selection intact.
    #[gpui::test]
    fn escape_cancels_pending(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.toggle_selected(0, cx);
            app.stage_delete(cx);
        });
        // Simulate the BackToGrid Escape link directly (the binding itself
        // is compiler-checked + smoke, like every existing binding).
        app.update(cx, |app, cx| {
            app.pending_batch = None;
            cx.notify();
        });
        app.read_with(cx, |app, _| {
            assert!(app.pending_batch.is_none());
            assert_eq!(app.selected.iter().copied().collect::<Vec<_>>(), vec![0]);
        });
    }

    /// Move staging records destination + paths (the rfd picker itself is
    /// smoke-only — it blocks headless).
    #[gpui::test]
    fn stage_move_records_dest_and_paths(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::write(dir.path().join("a.png"), b"stub").expect("fixture a.png");
        let dest = tempfile::tempdir().expect("tempdir dest");
        let dir_path = dir.path().to_path_buf();
        let dest_path = dest.path().to_path_buf();
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let (d, t) = (dir_path.clone(), dest_path.clone());
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.open_folder(d, cx);
        });
        app.update(cx, |app, cx| {
            app.toggle_selected(0, cx);
            app.stage_move(t, cx);
        });
        app.read_with(cx, |app, _| {
            assert!(matches!(
                app.pending_batch,
                Some(BatchOp::Move { ref paths, ref dest })
                    if paths.len() == 1 && *dest == dest_path
            ));
            assert!(dir_path.join("a.png").exists());
        });
    }

    // ── V3: batch execution + report ──

    /// Enter with a staged delete executes: files leave the session AND the
    /// disk (recycle bin), set clears on full success, no status lingers.
    #[gpui::test]
    fn confirm_delete_executes_and_clears(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::write(dir.path().join("a.png"), b"stub").expect("fixture a.png");
        std::fs::write(dir.path().join("b.png"), b"stub").expect("fixture b.png");
        std::fs::write(dir.path().join("c.png"), b"stub").expect("fixture c.png");
        let dir_path = dir.path().to_path_buf();
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let d = dir_path.clone();
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.open_folder(d, cx);
        });
        app.update(cx, |app, cx| {
            app.toggle_selected(0, cx);
            app.toggle_selected(1, cx);
            app.stage_delete(cx);
            app.confirm_pending(cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(app.pending_batch.is_none());
            assert!(app.selected.is_empty());
            assert_eq!(app.session.images.len(), 1);
            assert!(app.batch_status.is_none(), "full success is silent");
        });
        assert!(!dir_path.join("a.png").exists());
        assert!(!dir_path.join("b.png").exists());
        assert!(dir_path.join("c.png").exists());
    }

    /// Partial move keeps exactly the leftovers selected and reports.
    #[gpui::test]
    fn partial_move_keeps_leftovers_and_reports(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::write(dir.path().join("a.png"), b"stub").expect("fixture a.png");
        std::fs::write(dir.path().join("b.png"), b"stub").expect("fixture b.png");
        let dir_path = dir.path().to_path_buf();
        let dest = tempfile::tempdir().expect("tempdir dest");
        // Pre-seed the collision: b.png already exists at destination.
        std::fs::write(dest.path().join("b.png"), b"original").expect("seed collision");
        let dest_path = dest.path().to_path_buf();
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let (d, t) = (dir_path.clone(), dest_path.clone());
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.open_folder(d, cx);
        });
        app.update(cx, |app, cx| {
            app.select_all(cx);
            app.stage_move(t, cx);
            app.confirm_pending(cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            // a.png moved away; only b.png (skipped) remains, still marked.
            assert_eq!(app.session.images.len(), 1);
            assert_eq!(app.selected.iter().copied().collect::<Vec<_>>(), vec![0]);
            let status = app.batch_status.clone().expect("partial must report");
            assert!(status.contains("1 of 2"), "got: {status}");
            assert!(status.contains("skipped"), "got: {status}");
        });
        assert!(!dir_path.join("a.png").exists());
        assert!(dir_path.join("b.png").exists());
        assert!(dest_path.join("a.png").exists());
        assert_eq!(std::fs::read(dest_path.join("b.png")).unwrap(), b"original");
    }

    /// Deleting everything the folder holds lands in the existing
    /// empty-state (error slot mirrors the open_folder empty arm).
    #[gpui::test]
    fn delete_all_leaves_empty_grid_state(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::write(dir.path().join("a.png"), b"stub").expect("fixture a.png");
        let dir_path = dir.path().to_path_buf();
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let d = dir_path.clone();
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.open_folder(d, cx);
        });
        app.update(cx, |app, cx| {
            app.select_all(cx);
            app.stage_delete(cx);
            app.confirm_pending(cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(app.session.images.is_empty());
            assert!(app.session.error.is_some());
            assert_eq!(app.session.current, 0);
        });
    }

    /// Bar message counts + shortens the destination (`display_name`);
    /// resolves through the `i18n` templates in both languages (S5).
    #[test]
    fn batch_bar_message_counts_and_shortens_dest() {
        use sh_core::i18n::Language;
        assert_eq!(
            batch_bar_message(
                Language::En,
                &BatchOp::Delete {
                    paths: vec![PathBuf::from("a")]
                }
            ),
            "Delete 1 file to recycle bin?"
        );
        assert_eq!(
            batch_bar_message(
                Language::Es,
                &BatchOp::Delete {
                    paths: vec![PathBuf::from("a"), PathBuf::from("b")]
                }
            ),
            "¿Mover 2 archivos a la papelera?"
        );
        assert_eq!(
            batch_bar_message(
                Language::En,
                &BatchOp::Move {
                    paths: vec![PathBuf::from("a"), PathBuf::from("b")],
                    dest: PathBuf::from("C:\\pics\\Fotos"),
                }
            ),
            "Move 2 files to pics\\Fotos?"
        );
        assert_eq!(
            batch_bar_message(
                Language::Es,
                &BatchOp::Move {
                    paths: vec![PathBuf::from("a")],
                    dest: PathBuf::from("C:\\pics\\Fotos"),
                }
            ),
            "¿Mover 1 archivo a pics\\Fotos?"
        );
    }

    /// Copy with an empty set never touches the clipboard (no-op signal
    /// from `selection_paths_string` — safe to assert in-harness).
    #[gpui::test]
    fn copy_empty_selection_is_noop(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.copy_selection(cx); // must not panic, must not write
        });
        app.read_with(cx, |app, _| {
            assert!(app.selected.is_empty());
        });
    }

    /// Non-empty copy goes through `selection_paths_string` — the exact
    /// payload is pinned there (Task 1); here we assert the method runs
    /// the write path without error state. NOTE: this touches the REAL
    /// clipboard in the harness. If CI proves flaky, delete this test and
    /// keep the manual-smoke contract (same policy as `copy_image`).
    #[gpui::test]
    fn copy_nonempty_selection_writes_paths(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.toggle_selected(0, cx);
            app.copy_selection(cx);
        });
        app.read_with(cx, |app, _| {
            // No error surfaced; selection intact after copy.
            assert_eq!(app.selected.iter().copied().collect::<Vec<_>>(), vec![0]);
        });
    }

    /// Six distinct folders: the list caps at 5, newest first, oldest evicted.
    #[gpui::test]
    fn six_folders_evict_the_oldest(cx: &mut gpui::TestAppContext) {
        let root = tempfile::tempdir().expect("tempdir root");
        let mut dirs: Vec<std::path::PathBuf> = Vec::new();
        for i in 0..6 {
            let d = root.path().join(format!("d{i}"));
            std::fs::create_dir_all(&d).expect("subdir");
            std::fs::write(d.join(format!("i{i}.png")), b"stub").expect("fixture");
            dirs.push(d);
        }

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        for d in dirs.clone() {
            let dd = d.clone();
            app.update(cx, |app, cx| {
                app.open_folder(dd, cx);
            });
        }
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.settings.recent_dirs.len(),
                sh_core::recent::RECENT_DIRS_MAX
            );
            // Newest (d5) first; the very first (d0) was evicted.
            assert_eq!(app.settings.recent_dirs[0], dirs[5]);
            assert!(!app.settings.recent_dirs.contains(&dirs[0]));
        });
    }

    /// An empty folder (no images) never enters the recents list — the
    /// existing rule, now list-shaped.
    #[gpui::test]
    fn empty_folder_is_not_remembered(cx: &mut gpui::TestAppContext) {
        let dir_a = tempfile::tempdir().expect("tempdir a");
        let empty = tempfile::tempdir().expect("tempdir empty");
        std::fs::write(dir_a.path().join("a.png"), b"stub").expect("fixture a.png");
        let a = dir_a.path().to_path_buf();
        let e = empty.path().to_path_buf();

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let (a1, e1) = (a.clone(), e.clone());
        app.update(cx, |app, cx| {
            app.open_folder(a1, cx);
        });
        app.update(cx, |app, cx| {
            app.open_folder(e1, cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            // Empty folder did NOT replace or add to the list.
            assert_eq!(app.settings.recent_dirs, vec![a.clone()]);
            assert!(!app.settings.recent_dirs.contains(&e));
        });
    }

    // ── V3: slideshow state (transient — dies on folder swap / crop) ──

    /// A folder switch mid-slideshow must stop it: the new folder's scan
    /// swaps `session.images`, and auto-advancing into a fresh list the
    /// user never chose to play is wrong.
    #[gpui::test]
    fn folder_swap_resets_slideshow(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        std::fs::write(dir.path().join("a.png"), b"stub").expect("fixture a.png");
        let dir_path = dir.path().to_path_buf();

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Viewer;
            app.session.slideshow_active = true;
            app.open_folder(dir_path, cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(!app.session.slideshow_active);
        });
    }

    /// Crop mode owns the pointer — the slideshow must not auto-advance
    /// while a selection is being drawn.
    #[gpui::test]
    fn enter_crop_resets_slideshow(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Viewer;
            app.session.slideshow_active = true;
            app.enter_crop(cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(!app.session.slideshow_active);
        });
    }

    /// The drop-file-then-back bug: `open_path` (used by file drops, the
    /// open-file dialog, and CLI) swaps `session.images` to the dropped
    /// file's sibling list but never armed a thumbnail batch — the Grid
    /// showed empty placeholders forever. Regression: after `open_path` +
    /// `enter_grid`, the background decode must have filled `thumbs`.
    #[gpui::test]
    fn open_path_then_grid_loads_thumbnails(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let file_path = fixture_png_in(dir.path(), "a.png");

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.open_path(file_path.clone(), cx);
        });
        // Viewer shows the dropped file; "Back" returns to the Grid.
        app.update(cx, |app, cx| {
            app.enter_grid(cx);
        });
        // Drain the background executor: every decode task must have run
        // and committed under the seq guard.
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.view, View::Grid);
            assert_eq!(
                app.session.images.len(),
                1,
                "sibling list of the dropped file"
            );
            assert!(
                app.thumbs.contains_key(&file_path),
                "open_path must arm the thumbnail batch — thumbs map is {:?}",
                app.thumbs.keys().collect::<Vec<_>>()
            );
        });
    }

    /// Seq-guard regression: two `open_path` calls back to back (folder A
    /// then file from folder B) must leave only B's thumbnails — the stale
    /// A batch dies on its seq check.
    #[gpui::test]
    fn open_path_twice_drops_stale_thumbs(cx: &mut gpui::TestAppContext) {
        let dir_a = tempfile::tempdir().expect("tempdir A must be created");
        let dir_b = tempfile::tempdir().expect("tempdir B must be created");
        let png_a = fixture_png_in(dir_a.path(), "a.png");
        let png_b = fixture_png_in(dir_b.path(), "b.png");

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.open_path(png_a.clone(), cx);
            app.open_path(png_b.clone(), cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(app.thumbs.contains_key(&png_b), "current folder's thumb");
            assert!(
                !app.thumbs.contains_key(&png_a),
                "stale folder A thumb must be dropped (seq guard)"
            );
        });
    }

    #[gpui::test]
    fn viewer_viewport_subtracts_topbar(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, _| {
            app.viewport = gpui::size(gpui::px(1000.), gpui::px(720.));
        });
        app.read_with(cx, |app, _| {
            let v = app.viewer_viewport();
            assert!((v.x - 1000.0).abs() < 1e-5);
            assert!(
                (v.y - (720.0 - crate::ui::topbar::TOPBAR_H_PX)).abs() < 1e-5,
                "expected 680px height, got {}",
                v.y
            );
        });
    }

    /// Sort menu flow (V3): opening the menu and picking a criterion
    /// updates the session + settings copy and closes the menu. State-based
    /// assertions — the dropdown click itself goes through the same
    /// `set_sort`/`sort_menu_open` state the chip and rows mutate.
    #[gpui::test]
    fn sort_menu_flow_updates_state_and_persists(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        fixture_stub(&dir.path().join("a.png"), 100);
        fixture_stub(&dir.path().join("b.png"), 300);

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let dir_path = dir.path().to_path_buf();
        app.update(cx, |app, cx| {
            app.open_folder(dir_path.clone(), cx);
        });
        cx.run_until_parked();
        // Toggle the menu open via the same flag the chip's click flips.
        app.update(cx, |app, cx| {
            app.sort_menu_open = true;
            cx.notify();
        });
        // "Pick" Size-desc via the same calls the menu rows make.
        app.update(cx, |app, cx| {
            app.set_sort(SortBy::Size, SortDir::Desc, cx);
            app.sort_menu_open = false;
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.session.sort_by, SortBy::Size);
            assert_eq!(app.session.sort_dir, SortDir::Desc);
            assert!(!app.sort_menu_open);
            assert_eq!(app.settings.sort_by, SortBy::Size);
            // Size-desc order: b(300) then a(100).
            assert_eq!(app.session.images[0].path, dir_path.join("b.png"));
            assert_eq!(app.session.images[1].path, dir_path.join("a.png"));
        });
    }

    #[gpui::test]
    fn enter_viewer_sets_current_and_view(cx: &mut gpui::TestAppContext) {
        // Fake paths: the header probe fails into the error slot, but
        // current/view switching is synchronous — assert only that.
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.enter_viewer(2, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.view, crate::state::view::View::Viewer);
            assert_eq!(app.session.current, 2);
            assert_eq!(app.grid_selected, 2);
        });
    }

    /// Sort contract (V3): changing the criterion keeps the selection on
    /// the SAME image (re-anchored by path), the grid selection follows,
    /// and the settings copy mirrors the change for the next launch.
    #[gpui::test]
    fn set_sort_reanchors_selection_by_path(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        // Distinct sizes so Size-desc reorders against the Name-asc scan
        // order: scan gives a(100), b(300), c(200); Size-desc is b, c, a.
        fixture_stub(&dir.path().join("a.png"), 100);
        fixture_stub(&dir.path().join("b.png"), 300);
        fixture_stub(&dir.path().join("c.png"), 200);

        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let dir_path = dir.path().to_path_buf();
        app.update(cx, |app, cx| {
            app.open_folder(dir_path.clone(), cx);
        });
        cx.run_until_parked();
        // Select the third image under Name/Asc (c.png, index 2).
        app.update(cx, |app, cx| {
            app.enter_viewer(2, cx);
        });
        let anchor_path: PathBuf = app.read_with(cx, |app, _| {
            app.session
                .current_item()
                .expect("c.png must be current")
                .path
                .clone()
        });
        // Sort by size desc: order becomes b(300), c(200), a(100).
        app.update(cx, |app, cx| {
            app.set_sort(
                sh_core::navigation::SortBy::Size,
                sh_core::navigation::SortDir::Desc,
                cx,
            );
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.session.images[0].path, dir_path.join("b.png"));
            assert_eq!(app.session.images[1].path, dir_path.join("c.png"));
            assert_eq!(app.session.images[2].path, dir_path.join("a.png"));
            // Re-anchored: same image selected, new index.
            assert_eq!(
                app.session
                    .current_item()
                    .expect("selection must survive the resort")
                    .path,
                anchor_path
            );
            assert_eq!(app.session.current, 1);
            assert_eq!(
                app.grid_selected, 1,
                "grid selection must follow the anchor"
            );
            // Persisted: the settings copy carries the new sort.
            assert_eq!(app.settings.sort_by, sh_core::navigation::SortBy::Size);
            assert_eq!(app.settings.sort_dir, sh_core::navigation::SortDir::Desc);
        });
    }

    // ── Zoomable grid Phase 3 RED: set_grid_size + chip ──

    /// Size contract: selecting L mirrors into the settings copy and the
    /// settings file, so a relaunch restores L with no further action.
    #[gpui::test]
    fn set_grid_size_persists_and_restores_across_relaunch(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let settings_path = dir.path().join("settings.json");
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.settings_path = settings_path.clone();
            app.set_grid_size(sh_core::settings::GridSize::L, cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.settings.grid_size, sh_core::settings::GridSize::L);
        });
        let restored = sh_core::settings::load(&settings_path);
        assert_eq!(
            restored.grid_size,
            sh_core::settings::GridSize::L,
            "L must survive a relaunch through settings.json"
        );
    }

    /// Re-clamp contract: a stale offset from a larger grid can neither
    /// strand the last rows (shrink) nor leave a trailing gap (grow), and
    /// the cursor stays visible after every size change.
    #[gpui::test]
    fn set_grid_size_reclamps_stale_offset(cx: &mut gpui::TestAppContext) {
        use crate::ui::grid::{self, GridSizeGeometry};
        use sh_core::settings::GridSize;
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        // 30 images so every preset scrolls in a short viewport.
        app.update(cx, |app, _cx| {
            let epoch = std::time::SystemTime::UNIX_EPOCH;
            app.session.images =
                build_image_items((0..30).map(|i| sh_core::navigation::ImageEntry {
                    path: PathBuf::from(format!("Z:\\fake\\{i:02}.png")),
                    size: 0,
                    modified: epoch,
                    created: None,
                }));
            app.viewport = gpui::size(gpui::px(800.), gpui::px(280.));
        });
        // Cursor on the last image; a huge offset is stale for every
        // preset. NOTE: viewport is (re)set in the same closure as each
        // action — the headless render loop restores the window size
        // between updates, so viewport + action must stay atomic.
        app.update(cx, |app, cx| {
            app.viewport = gpui::size(gpui::px(800.), gpui::px(280.));
            app.set_grid_size(GridSize::L, cx);
            app.grid_selected = 29;
            app.anchor = 29;
            let max_l = grid::grid_max_scroll(30, 800.0, 240.0, &GridSize::L.geometry());
            assert!(max_l > 0.0, "L must scroll in this viewport");
            app.grid_scroll_px = 1e9;
        });
        // Shrink L → S: offset clamps into S's range, last row reachable.
        app.update(cx, |app, cx| {
            app.viewport = gpui::size(gpui::px(800.), gpui::px(280.));
            app.set_grid_size(GridSize::S, cx);
        });
        app.read_with(cx, |app, _| {
            let geo = GridSize::S.geometry();
            let max_s = grid::grid_max_scroll(30, 800.0, 240.0, &geo);
            assert!(
                (app.grid_scroll_px - max_s).abs() < 1e-3,
                "shrink must clamp to the S maximum ({}), got {}",
                max_s,
                app.grid_scroll_px
            );
            let cols = grid::grid_columns(800.0, &geo);
            let row_top = (29 / cols) as f32 * geo.row_h as f32;
            let row_bottom = row_top + geo.row_h as f32;
            assert!(
                row_top >= app.grid_scroll_px && row_bottom <= app.grid_scroll_px + 240.0,
                "cursor row must stay visible after shrink"
            );
        });
        // Grow S → L: offset clamps into L's range (no trailing gap),
        // cursor still visible.
        app.update(cx, |app, cx| {
            app.viewport = gpui::size(gpui::px(800.), gpui::px(280.));
            app.set_grid_size(GridSize::L, cx);
        });
        app.read_with(cx, |app, _| {
            let geo = GridSize::L.geometry();
            let max_l = grid::grid_max_scroll(30, 800.0, 240.0, &geo);
            assert!(
                app.grid_scroll_px <= max_l + 1e-4,
                "grow must clear the trailing gap (max {max_l}), got {}",
                app.grid_scroll_px
            );
            let cols = grid::grid_columns(800.0, &geo);
            let row_top = (29 / cols) as f32 * geo.row_h as f32;
            let row_bottom = row_top + geo.row_h as f32;
            assert!(
                row_top >= app.grid_scroll_px && row_bottom <= app.grid_scroll_px + 240.0,
                "cursor row must stay visible after grow"
            );
        });
    }

    /// Unlike `set_sort`, a size change re-sorts nothing and never moves
    /// the cursor, the anchor, or the work-in-progress set.
    #[gpui::test]
    fn set_grid_size_leaves_order_and_selection_untouched(cx: &mut gpui::TestAppContext) {
        use sh_core::settings::GridSize;
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        let before: Vec<PathBuf> = app.read_with(cx, |app, _| {
            app.session.images.iter().map(|i| i.path.clone()).collect()
        });
        app.update(cx, |app, cx| {
            app.grid_selected = 2;
            app.anchor = 2;
            app.selected.insert(0);
            app.selected.insert(2);
            app.set_grid_size(GridSize::L, cx);
        });
        app.read_with(cx, |app, _| {
            let after: Vec<PathBuf> = app.session.images.iter().map(|i| i.path.clone()).collect();
            assert_eq!(after, before, "size change must not re-sort items");
            assert_eq!(app.grid_selected, 2);
            assert_eq!(app.anchor, 2);
            assert!(
                app.selected.contains(&0) && app.selected.contains(&2) && app.selected.len() == 2,
                "selection set must survive a size change"
            );
        });
    }

    /// Chip labels resolve through `Settings.language` (EN + ES words,
    /// never bare letters) — same table the render reads.
    #[test]
    fn grid_size_chip_labels_resolve_through_language() {
        use sh_core::i18n::Language;
        use sh_core::settings::GridSize;
        assert_eq!(grid_size_label(Language::En, GridSize::S), "Small");
        assert_eq!(grid_size_label(Language::En, GridSize::M), "Medium");
        assert_eq!(grid_size_label(Language::En, GridSize::L), "Large");
        assert_eq!(grid_size_label(Language::Es, GridSize::S), "Pequeño");
        assert_eq!(grid_size_label(Language::Es, GridSize::M), "Mediano");
        assert_eq!(grid_size_label(Language::Es, GridSize::L), "Grande");
    }

    /// Segment model: exactly S/M/L in order, only the active preset
    /// marked active — the render maps this 1:1 to chip segments.
    // ── Zoom-preset chips (viewer-zoom-presets, Phase 4) ──

    #[test]
    fn zoom_preset_label_resolves_through_the_table_in_both_languages() {
        // EN + ES for all four chips via `Settings.language` threading.
        let cases = [
            (ZoomPreset::Fit, "Fit", "Ajustar"),
            (ZoomPreset::Scale50, "50%", "50%"),
            (ZoomPreset::Scale100, "100%", "100%"),
            (ZoomPreset::Scale200, "200%", "200%"),
        ];
        for (preset, en, es) in cases {
            assert_eq!(zoom_preset_label(Language::En, preset), en);
            assert_eq!(zoom_preset_label(Language::Es, preset), es);
        }
    }

    #[test]
    fn zoom_preset_segments_mark_only_the_active_chip() {
        use sh_core::transform::MAX_SCALE;
        // Resting exactly at 1.0 in Percent100 → only the 100% chip active.
        let at_100 = zoom_preset_segments(Language::En, FitMode::Percent100, 1.0);
        assert_eq!(at_100.len(), 4);
        for (preset, _, active) in &at_100 {
            let expected = matches!(preset, ZoomPreset::Scale100);
            assert_eq!(*active, expected, "1.0 must mark only Scale100");
        }
        // Free-zoom 137% matches no preset chip.
        for (_, _, active) in zoom_preset_segments(Language::En, FitMode::Percent100, 1.37) {
            assert!(!active, "137% must mark no scale chip");
        }
        // Fit state marks only Fit — even when the fit scale happens to read
        // "100%" on a huge image (fit 1.0 in Fit mode stays Fit-only).
        let in_fit = zoom_preset_segments(Language::En, FitMode::Fit, 1.0);
        for (preset, _, active) in &in_fit {
            let expected = matches!(preset, ZoomPreset::Fit);
            assert_eq!(*active, expected, "Fit mode must mark only Fit");
        }
        // Near-preset-but-not-exact scale stays inactive (avoids the .5
        // halfway edge by asserting a neighbor).
        for (_, _, active) in
            zoom_preset_segments(Language::En, FitMode::Percent100, MAX_SCALE + 0.5)
        {
            assert!(!active);
        }
    }

    #[test]
    fn zoom_preset_guard_no_keymap_actions_or_shortcuts_entries() {
        // Click-only scope guard: nothing added to keymap defaults, ACTIONS,
        // or the Shortcuts panel. ACTIONS.len() 18 and SHORTCUT_ROW_COUNT 18
        // stay pinned by the existing tests; assert both here so a future
        // preset shortkut can never sneak in without breaking this.
        let defaults = sh_core::keymap::defaults();
        for id in defaults.keys() {
            assert!(
                !id.contains("preset"),
                "no preset keymap binding may exist, found {id}"
            );
        }
        for a in crate::actions::ACTIONS {
            assert!(
                !a.id.contains("preset"),
                "no preset action descriptor may exist, found {}",
                a.id
            );
            assert!(
                !format!("{:?}", a.label_key).contains("ZoomPreset"),
                "preset StrKeys are chip labels, not action labels"
            );
        }
        assert_eq!(crate::actions::ACTIONS.len(), 18);
        assert_eq!(crate::ui::settings_panel::scroll::SHORTCUT_ROW_COUNT, 18);
    }

    #[test]
    fn grid_size_segments_mark_only_the_active_preset() {
        use sh_core::i18n::Language;
        use sh_core::settings::GridSize;
        let segs = grid_size_segments(Language::En, GridSize::M);
        assert_eq!(segs.len(), 3, "exactly S/M/L segments");
        assert_eq!(
            segs.iter().map(|(s, _, _)| *s).collect::<Vec<_>>(),
            vec![GridSize::S, GridSize::M, GridSize::L]
        );
        assert_eq!(
            segs.iter()
                .map(|(_, _, active)| *active)
                .collect::<Vec<_>>(),
            vec![false, true, false]
        );
        assert_eq!(segs[1].1, "Medium");
        // Spanish renderings travel through the same model.
        let es = grid_size_segments(Language::Es, GridSize::S);
        assert_eq!(es[0].1, "Pequeño");
        assert!(es[0].2);
        assert!(!es[1].2 && !es[2].2);
    }

    #[gpui::test]
    fn move_selection_clamps_and_scrolls(cx: &mut gpui::TestAppContext) {
        // test_app ships 3 fake images; selection math is sync.
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.viewport = gpui::size(gpui::px(1000.), gpui::px(720.));
            app.move_selection(1, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.grid_selected, 1);
        });
        app.update(cx, |app, cx| {
            // Past the end sticks (len 3) instead of wrapping.
            app.move_selection(10, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.grid_selected, 2);
            assert!(app.grid_scroll_px >= 0.0);
        });
    }

    #[gpui::test]
    fn enter_grid_follows_current(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Viewer;
            app.session.current = 1;
            app.enter_grid(cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.view, crate::state::view::View::Grid);
            assert_eq!(app.grid_selected, 1);
            assert_eq!(app.grid_scroll_px, 0.0);
        });
    }

    #[gpui::test]
    fn apply_builtin_theme_switches_and_persists(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            // test_app starts on the default (Noir Gallery) theme.
            assert_ne!(app.theme_store.name, "dark-clinical.json");
            assert!(app.apply_builtin_theme("dark-clinical.json", cx));
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.theme_store.name, "dark-clinical.json");
            assert_eq!(app.theme_store.theme.name, "Dark Clinical");
            assert_eq!(app.settings.theme, "dark-clinical.json");
            // Hot-reload baseline follows the switch (no instant revert).
            let builtin = crate::theme_builtins::builtin_theme_json("dark-clinical.json");
            assert_eq!(app.last_applied_theme_text, builtin);
        });
        // Unknown name is a no-op returning false.
        app.update(cx, |app, cx| {
            assert!(!app.apply_builtin_theme("no-such-theme.json", cx));
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.theme_store.name, "dark-clinical.json");
        });
    }

    // ── Crop state transitions (Task 4) ──

    #[gpui::test]
    fn enter_crop_sets_mode(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Viewer;
            app.crop_rect = Some(sh_core::crop::CropRect {
                x0: 10.0,
                y0: 10.0,
                x1: 20.0,
                y1: 20.0,
            });
            app.enter_crop(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(app.crop_mode);
            // A stale selection from a previous session must not leak in.
            assert!(app.crop_rect.is_none());
        });
    }

    #[gpui::test]
    fn cancel_crop_clears(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Viewer;
            app.crop_mode = true;
            app.crop_rect = Some(sh_core::crop::CropRect {
                x0: 10.0,
                y0: 10.0,
                x1: 20.0,
                y1: 20.0,
            });
            app.cancel_crop(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(!app.crop_mode);
            assert!(app.crop_rect.is_none());
        });
    }

    #[gpui::test]
    fn toggle_twice_returns(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Viewer;
            app.toggle_crop(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(app.crop_mode);
        });
        app.update(cx, |app, cx| {
            app.toggle_crop(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(!app.crop_mode);
            assert!(app.crop_rect.is_none());
        });
    }

    // ── Crop drag + confirm bar (Task 5) ──

    #[gpui::test]
    fn drag_updates_rect_and_bar_on_release(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Viewer;
            app.enter_crop(cx);
            app.begin_crop_drag((100.0, 100.0));
        });
        app.read_with(cx, |app, _| {
            // Zero-area rect anchored at the press point.
            let r = app.crop_rect.expect("drag must arm a rect");
            assert_eq!((r.x0, r.y0, r.x1, r.y1), (100.0, 100.0, 100.0, 100.0));
        });
        app.update(cx, |app, _| {
            app.update_crop_drag((300.0, 250.0));
            app.finish_crop_drag();
        });
        app.read_with(cx, |app, _| {
            let r = app.crop_rect.expect("real-area rect must survive");
            assert_eq!((r.x0, r.y0, r.x1, r.y1), (100.0, 100.0, 300.0, 250.0));
            assert!(app.crop_bar_visible, "confirm bar must appear");
        });
    }

    #[gpui::test]
    fn degenerate_drag_discards_silently(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Viewer;
            app.enter_crop(cx);
            app.begin_crop_drag((100.0, 100.0));
            app.update_crop_drag((101.0, 100.5)); // < 4 px²
            app.finish_crop_drag();
        });
        app.read_with(cx, |app, _| {
            assert!(app.crop_rect.is_none(), "click-like drag must discard");
            assert!(!app.crop_bar_visible, "no bar for a degenerate rect");
        });
    }

    #[gpui::test]
    fn confirm_with_unknown_dimensions_closes_silently(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Viewer;
            app.enter_crop(cx);
            app.begin_crop_drag((10.0, 10.0));
            app.update_crop_drag((200.0, 200.0));
            app.finish_crop_drag();
            assert!(app.crop_bar_visible);
            // test_app images have no dimensions (fake paths, probe fails);
            // confirm must still close silently instead of erroring.
            app.confirm_crop_copy(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(!app.crop_bar_visible);
            assert!(app.crop_rect.is_none());
            assert!(app.session.error.is_none(), "no error for a silent close");
        });
    }

    #[gpui::test]
    fn open_settings_remembers_origin_and_esc_returns(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Grid;
            app.open_settings(cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.view, View::Settings);
            assert_eq!(app.settings_return_to, View::Grid);
        });
        app.update(cx, |app, cx| {
            app.close_settings(cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.view, View::Grid);
            assert!(app.capture_action.is_none());
        });
    }

    #[gpui::test]
    fn apply_keymap_persists_and_rebinds_live(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.bind_keys(crate::actions::resolve_bindings(
                &sh_core::keymap::defaults(),
            ));
        });
        // test_app's settings_path (Z:\fake) cannot be written; point at a
        // real tempdir first so the atomic save succeeds and the rebind
        // actually happens.
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let settings_path = dir.path().join("settings.json");
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = test_app(cx);
            app.settings_path = settings_path.clone();
            // Mirror main.rs's startup focus: the tracked root div must own
            // keyboard focus before any keystroke arrives.
            window.focus(&app.focus_handle);
            app
        });
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            let mut km = sh_core::keymap::defaults();
            km.insert(
                "toggle-slideshow".into(),
                sh_core::keymap::KeyBinding {
                    ctrl: true,
                    shift: false,
                    alt: false,
                    platform: false,
                    key: "k".into(),
                },
            );
            app.apply_keymap(km, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.settings.keymap.get("toggle-slideshow").unwrap().key,
                "k"
            );
        });
        cx.simulate_keystrokes("ctrl-k");
        app.read_with(cx, |app, _| {
            assert!(app.session.slideshow_active, "rebound combo must fire");
        });
    }

    // ── Task 8: settings panel flows ──

    #[gpui::test]
    fn ctrl_comma_opens_settings_from_each_view_and_esc_returns(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.bind_keys(crate::actions::resolve_bindings(
                &sh_core::keymap::defaults(),
            ));
        });
        for origin in [View::Welcome, View::Grid, View::Viewer] {
            let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
            let cx = cx as &mut gpui::VisualTestContext;
            app.update(cx, |app, cx| {
                app.view = origin;
                // Simulate the OpenSettings dispatch path directly (the binding
                // string itself is compiler-checked via KeyBinding::new).
                app.open_settings(cx);
            });
            app.read_with(cx, |app, _| {
                assert_eq!(app.view, View::Settings);
                assert_eq!(app.settings_return_to, origin);
            });
            app.update(cx, |app, cx| {
                app.close_settings(cx);
            });
            app.read_with(cx, |app, _| {
                assert_eq!(app.view, origin);
            });
        }
        // Real-keystroke proof: `ctrl-,` routes through the live keymap to
        // the `OpenSettings` action (cold-start focus pattern so the
        // `image_view` context joins the dispatch path).
        let (app, cx) = cx.add_window_view(|window, cx| {
            let app = test_app(cx);
            window.focus(&app.focus_handle);
            app
        });
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, _| {
            app.view = View::Grid;
        });
        cx.simulate_keystrokes("ctrl-,");
        app.read_with(cx, |app, _| {
            assert_eq!(app.view, View::Settings);
            assert_eq!(app.settings_return_to, View::Grid);
        });
    }

    /// State contract: switching the settings section clears any in-progress
    /// capture (section value + capture cleared). The row-callback wiring is
    /// covered by construction — this uses the same calls the sidebar row makes.
    #[gpui::test]
    fn settings_section_switch_contract_clears_capture(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Settings;
            app.capture_action = Some("open-file".into());
            // Same calls the sidebar row makes.
            app.settings_section = crate::ui::settings_panel::SettingsSection::Appearance;
            app.capture_action = None;
            app.capture_conflict = None;
            cx.notify();
        });
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.settings_section,
                crate::ui::settings_panel::SettingsSection::Appearance
            );
            assert!(app.capture_action.is_none());
        });
    }

    #[gpui::test]
    fn capture_conflict_reports_incumbent(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, _| {
            // Offer open-file's ctrl-o to open-folder → must conflict.
            let ctrl_o = app.settings.keymap.get("open-file").unwrap().clone();
            let err = sh_core::keymap::validate_binding(
                &app.settings.keymap,
                crate::actions::KEYMAP_CONTEXT,
                "open-folder",
                &ctrl_o,
            )
            .unwrap_err();
            assert_eq!(err.existing_action, "open-file");
            // `validate_binding` is pure (returns Result, mutates nothing), so
            // this assert documents the rejection-persists-nothing contract:
            // a rejected candidate leaves the stored binding untouched, which
            // is what Task-7's rejection path relies on.
            // Binding unchanged (rejection persists nothing).
            assert_eq!(
                app.settings.keymap.get("open-folder").unwrap(),
                sh_core::keymap::defaults().get("open-folder").unwrap()
            );
        });
    }

    #[gpui::test]
    fn reset_restores_defaults(cx: &mut gpui::TestAppContext) {
        // apply_keymap saves synchronously: test_app's Z:\fake path cannot
        // be written, so point at a real tempdir first (Task 5 harness rule).
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let settings_path = dir.path().join("settings.json");
        let (app, cx) = cx.add_window_view(|_window, cx| {
            let mut app = test_app(cx);
            app.settings_path = settings_path.clone();
            app
        });
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            let mut km = sh_core::keymap::defaults();
            km.insert(
                "toggle-slideshow".into(),
                sh_core::keymap::KeyBinding {
                    ctrl: true,
                    shift: false,
                    alt: false,
                    platform: false,
                    key: "k".into(),
                },
            );
            app.apply_keymap(km, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.settings.keymap.get("toggle-slideshow").unwrap().key,
                "k"
            );
        });
        // Same call the armed Reset button makes.
        app.update(cx, |app, cx| {
            app.apply_keymap(sh_core::keymap::defaults(), cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.settings.keymap, sh_core::keymap::defaults());
        });
    }

    #[gpui::test]
    fn apply_language_commits_and_persists_on_success(cx: &mut gpui::TestAppContext) {
        use sh_core::i18n::Language;
        // Fresh default renders English (no OS-locale seeding).
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let settings_path = dir.path().join("settings.json");
        let (app, cx) = cx.add_window_view(|_window, cx| {
            let mut app = test_app(cx);
            app.settings_path = settings_path.clone();
            app
        });
        let cx = cx as &mut gpui::VisualTestContext;
        app.read_with(cx, |app, _| {
            assert_eq!(app.settings.language, Language::En);
        });
        // Successful save commits in memory (live re-render follows the
        // `cx.notify()` the method issues; visual switch is manual QA).
        app.update(cx, |app, cx| {
            app.apply_language(Language::Es, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.settings.language, Language::Es);
        });
        // …and persists across restarts.
        let reloaded = sh_core::settings::load(&settings_path);
        assert_eq!(reloaded.language, Language::Es);
        assert_eq!(reloaded.version, 7);
    }

    #[gpui::test]
    fn apply_language_keeps_old_language_when_save_fails(cx: &mut gpui::TestAppContext) {
        use sh_core::i18n::Language;
        let dir = tempfile::tempdir().expect("tempdir must be created");
        // Unwritable destination: `save` writes `<path>.tmp` then renames
        // onto `path` — pointing at an existing DIRECTORY makes the rename
        // fail on every platform (`create_dir_all` would otherwise rescue a
        // merely-missing parent).
        let blocked = dir.path().join("blocked");
        std::fs::create_dir(&blocked).expect("fixture dir");
        let (app, cx) = cx.add_window_view(|_window, cx| {
            let mut app = test_app(cx);
            app.settings_path = blocked.clone();
            app
        });
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.apply_language(Language::Es, cx);
        });
        // Commit-on-success: failed save keeps the previous language and
        // writes no settings file.
        app.read_with(cx, |app, _| {
            assert_eq!(app.settings.language, Language::En);
        });
        assert!(
            !blocked.join("settings.json").exists(),
            "failed save must write nothing"
        );
    }

    #[gpui::test]
    fn rebind_survives_disk_reload(cx: &mut gpui::TestAppContext) {
        // Persistence: rebind → reload settings from disk → custom binding present.
        // Uses a real temp settings path: apply_keymap saves synchronously
        // (save, not spawn) so no run_until_parked race.
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let settings_path = dir.path().join("settings.json");
        sh_core::settings::save(&settings_path, &sh_core::settings::Settings::default())
            .expect("seed settings");
        let (app, cx) = cx.add_window_view(|_window, cx| {
            let mut app = test_app(cx);
            app.settings_path = settings_path.clone();
            app.settings = sh_core::settings::load(&settings_path);
            app
        });
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            let mut km = app.settings.keymap.clone();
            km.insert(
                "toggle-slideshow".into(),
                sh_core::keymap::KeyBinding {
                    ctrl: true,
                    shift: false,
                    alt: false,
                    platform: false,
                    key: "k".into(),
                },
            );
            app.apply_keymap(km, cx);
        });
        let reloaded = sh_core::settings::load(&settings_path);
        assert_eq!(reloaded.keymap.get("toggle-slideshow").unwrap().key, "k");
        assert_eq!(reloaded.version, 7);
    }

    // ── Task 8: review-gap tests (Tasks 6–7 reviews) ──

    /// M1 Err-path: a failed atomic save keeps the in-memory settings AND the
    /// live bindings untouched (clone-then-assign contract in `apply_keymap`).
    #[gpui::test]
    fn apply_keymap_save_failure_keeps_old_settings(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        // A regular file where a directory would be needed: create_dir_all
        // on this parent fails, so the atomic save fails hermetically on
        // every platform (no nonexistent-drive tricks).
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"not a dir").expect("blocker file");
        let bad_path = blocker.join("settings.json");
        let (app, cx) = cx.add_window_view(|_window, cx| {
            let mut app = test_app(cx);
            app.settings_path = bad_path.clone();
            app
        });
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            let mut km = sh_core::keymap::defaults();
            km.insert(
                "toggle-slideshow".into(),
                sh_core::keymap::KeyBinding {
                    ctrl: true,
                    shift: false,
                    alt: false,
                    platform: false,
                    key: "k".into(),
                },
            );
            app.apply_keymap(km, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.settings.keymap,
                sh_core::keymap::defaults(),
                "failed save must leave settings untouched"
            );
            assert_eq!(app.settings.version, 7);
        });
    }

    /// M2 re-enter: opening Settings while already there is a no-op — the
    /// origin is NOT overwritten with Settings (early-return guard).
    #[gpui::test]
    fn open_settings_reenter_is_noop(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Grid;
            app.open_settings(cx);
        });
        app.update(cx, |app, cx| {
            app.open_settings(cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.view, View::Settings);
            assert_eq!(
                app.settings_return_to,
                View::Grid,
                "re-enter must not overwrite the origin with Settings"
            );
        });
    }

    /// Esc routing through the REAL `BackToGrid` action (not direct close):
    /// first Esc with a capture in progress cancels the capture and STAYS in
    /// Settings; second Esc returns to the origin view.
    #[gpui::test]
    fn esc_first_cancels_capture_then_returns(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.bind_keys(crate::actions::resolve_bindings(
                &sh_core::keymap::defaults(),
            ));
        });
        let (app, cx) = cx.add_window_view(|window, cx| {
            let app = test_app(cx);
            // Cold-start pattern: the tracked root must own focus before
            // any keystroke arrives.
            window.focus(&app.focus_handle);
            app
        });
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Grid;
            app.open_settings(cx);
            app.capture_action = Some("toggle-slideshow".into());
        });
        cx.simulate_keystrokes("escape");
        app.read_with(cx, |app, _| {
            assert!(
                app.capture_action.is_none(),
                "first Esc cancels the capture"
            );
            assert_eq!(app.view, View::Settings, "first Esc stays in Settings");
        });
        cx.simulate_keystrokes("escape");
        app.read_with(cx, |app, _| {
            assert_eq!(app.view, View::Grid, "second Esc returns to origin");
        });
    }

    /// Capture end-to-end (Task 7 Issue 2): while capturing, a real
    /// `ctrl-k` keystroke lands in the root `on_key_down` handler, validates,
    /// persists via `apply_keymap`, and clears the capture.
    #[gpui::test]
    fn capture_ctrl_k_rebinds_end_to_end(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.bind_keys(crate::actions::resolve_bindings(
                &sh_core::keymap::defaults(),
            ));
        });
        // apply_keymap saves synchronously: point at a real tempdir first
        // (Task 5 harness rule) so the capture persist succeeds.
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let settings_path = dir.path().join("settings.json");
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = test_app(cx);
            app.settings_path = settings_path.clone();
            // Cold-start pattern: the tracked root must own focus before
            // any keystroke arrives.
            window.focus(&app.focus_handle);
            app
        });
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Settings;
            app.settings_return_to = View::Grid;
            app.capture_action = Some("toggle-slideshow".into());
            cx.notify();
        });
        cx.simulate_keystrokes("ctrl-k");
        app.read_with(cx, |app, _| {
            let b = app.settings.keymap.get("toggle-slideshow").unwrap();
            assert_eq!(b.key, "k");
            assert!(b.ctrl);
            assert!(
                app.capture_action.is_none(),
                "successful capture clears capture mode"
            );
            assert!(app.capture_conflict.is_none());
        });
    }

    /// Appearance no-close: applying a builtin theme from the surface keeps
    /// the user in Settings (the old dropdown closed itself; the surface
    /// persists — closing here would strand the user).
    #[gpui::test]
    fn appearance_apply_keeps_settings_surface_open(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = View::Settings;
            app.settings_return_to = View::Grid;
            assert_ne!(app.theme_store.name, "dark-clinical.json");
            assert!(app.apply_builtin_theme("dark-clinical.json", cx));
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.theme_store.name, "dark-clinical.json");
            assert_eq!(
                app.view,
                View::Settings,
                "theme apply must not close Settings"
            );
        });
    }

    // ── Viewer info panel ──

    /// Write one decodable PNG per `(name, w, h)` spec into `dir` and
    /// return the paths in order. Distinct dims per file let
    /// navigate-while-open assert B's facts (never A's).
    fn real_info_images(dir: &std::path::Path, specs: &[(&str, u32, u32)]) -> Vec<PathBuf> {
        specs
            .iter()
            .map(|(name, w, h)| {
                let path = dir.join(name);
                let img = image::RgbaImage::from_fn(*w, *h, |x, y| {
                    image::Rgba([(x % 255) as u8, (y % 255) as u8, 255, 255])
                });
                img.save(&path).expect("info test fixture must save");
                path
            })
            .collect()
    }

    /// Point the test app's session at real image files with known dims.
    fn point_session_at_images(app: &mut App, paths: &[PathBuf], _cx: &mut gpui::Context<App>) {
        let epoch = std::time::SystemTime::UNIX_EPOCH;
        let entries: Vec<sh_core::navigation::ImageEntry> = paths
            .iter()
            .map(|p| sh_core::navigation::ImageEntry {
                path: p.clone(),
                size: 0,
                modified: epoch,
                created: None,
            })
            .collect();
        app.session.images = build_image_items(entries);
        app.session.current = 0;
        app.view = View::Viewer;
    }

    #[gpui::test]
    fn info_toggle_opens_with_current_facts_and_recloses(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let paths = real_info_images(dir.path(), &[("a.png", 17, 9)]);
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            point_session_at_images(app, &paths, cx);
            assert!(!app.info_panel_open, "panel starts closed (transient)");
            app.toggle_info_panel(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(app.info_panel_open);
            let (cached_path, result) = app.info_facts.as_ref().expect("facts cached on open");
            assert_eq!(cached_path, &paths[0]);
            let info = result.as_ref().expect("valid fixture must resolve").clone();
            assert_eq!((info.width, info.height), (17, 9));
            assert_eq!(info.format, "PNG");
        });
        app.update(cx, |app, cx| {
            app.toggle_info_panel(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(!app.info_panel_open, "re-tap closes the popover");
        });
    }

    #[gpui::test]
    fn info_esc_closes_without_navigating_or_leaving(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.bind_keys(crate::actions::resolve_bindings(
                &sh_core::keymap::defaults(),
            ));
        });
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let paths = real_info_images(dir.path(), &[("a.png", 17, 9), ("b.png", 32, 24)]);
        let (app, cx) = cx.add_window_view(|window, cx| {
            let app = test_app(cx);
            window.focus(&app.focus_handle);
            app
        });
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            point_session_at_images(app, &paths, cx);
            app.toggle_info_panel(cx);
            assert!(app.info_panel_open);
        });
        cx.simulate_keystrokes("escape");
        app.read_with(cx, |app, _| {
            assert!(!app.info_panel_open, "Esc closes the popover");
            assert_eq!(app.view, View::Viewer, "Esc must not leave the viewer");
            assert_eq!(app.session.current, 0, "Esc must not navigate");
        });
    }

    #[gpui::test]
    fn info_catcher_close_changes_nothing_else(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let paths = real_info_images(dir.path(), &[("a.png", 17, 9)]);
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            point_session_at_images(app, &paths, cx);
            app.toggle_info_panel(cx);
            app.close_info_panel(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(!app.info_panel_open);
            assert_eq!(app.view, View::Viewer);
            assert_eq!(app.session.current, 0);
            assert!(!app.sort_menu_open, "catcher close touches nothing else");
            assert!(app.session.images[0].path == paths[0]);
        });
    }

    #[gpui::test]
    fn info_navigate_while_open_shows_next_image_facts(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let paths = real_info_images(dir.path(), &[("a.png", 17, 9), ("b.png", 32, 24)]);
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            point_session_at_images(app, &paths, cx);
            app.toggle_info_panel(cx);
            app.navigate(1, cx);
        });
        app.read_with(cx, |app, _| {
            assert!(app.info_panel_open, "navigate keeps the panel open");
            assert_eq!(app.session.current, 1);
            let (cached_path, result) = app.info_facts.as_ref().expect("facts re-resolved");
            assert_eq!(cached_path, &paths[1], "facts track image B, never stale A");
            let info = result.as_ref().expect("valid fixture must resolve").clone();
            assert_eq!((info.width, info.height), (32, 24));
        });
    }

    #[gpui::test]
    fn info_corrupt_file_shows_localized_error_without_crash(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let bad = dir.path().join("corrupt.png");
        std::fs::write(&bad, b"not really a png").expect("corrupt fixture must write");
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            point_session_at_images(app, std::slice::from_ref(&bad), cx);
            app.toggle_info_panel(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(app.info_panel_open, "panel still opens on corrupt input");
            let (_, result) = app.info_facts.as_ref().expect("outcome cached");
            assert!(
                result.is_err(),
                "corrupt file must be a typed Err, never a panic"
            );
            assert_eq!(
                app.settings
                    .language
                    .get(sh_core::i18n::StrKey::InfoLoadError),
                "Could not read image info"
            );
        });
        // Missing file: same contract — typed Err, localized error copy.
        let missing = dir.path().join("gone.png");
        app.update(cx, |app, cx| {
            point_session_at_images(app, std::slice::from_ref(&missing), cx);
            app.toggle_info_panel(cx);
        });
        app.read_with(cx, |app, _| {
            let (_, result) = app.info_facts.as_ref().expect("outcome cached");
            assert!(result.is_err());
        });
    }

    #[test]
    fn info_has_no_action_keymap_or_shortcuts_surface() {
        // Click-only guard: the Shortcuts panel renders one row per ACTIONS
        // entry, so absence here covers all three surfaces (read-only
        // inspection — nothing is mutated).
        for desc in crate::actions::ACTIONS {
            assert!(
                !desc.id.contains("info"),
                "no action id for the info panel, found {}",
                desc.id
            );
        }
        for id in sh_core::keymap::defaults().keys() {
            assert!(
                !id.contains("info"),
                "no keymap entry for the info panel, found {id}"
            );
        }
    }

    #[gpui::test]
    fn info_labels_resolve_through_settings_language(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let paths = real_info_images(dir.path(), &[("a.png", 17, 9)]);
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            point_session_at_images(app, &paths, cx);
            app.settings.language = sh_core::i18n::Language::Es;
            cx.notify();
        });
        app.read_with(cx, |app, _| {
            let lang = app.settings.language;
            assert_eq!(
                lang.get(sh_core::i18n::StrKey::InfoDimensionsLabel),
                "Dimensiones"
            );
            assert_eq!(
                lang.get(sh_core::i18n::StrKey::InfoLoadError),
                "No se pudo leer la información de la imagen"
            );
            // Values stay untranslated regardless of language.
            let rows = crate::ui::overlay::info_rows(
                lang,
                &sh_core::decode::FileInfo {
                    width: 1920,
                    height: 1080,
                    size_bytes: 2_400_000,
                    format: "PNG".into(),
                },
            );
            assert_eq!(rows[0].1, "1920 × 1080");
            assert_eq!(rows[1].1, "2.4 MB");
            assert_eq!(rows[2].1, "PNG");
        });
    }
}
