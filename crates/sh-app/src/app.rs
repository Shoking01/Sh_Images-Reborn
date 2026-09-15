//! Root application component: session, theme, key dispatch, root render.

use crate::actions::{
    BackToGrid, CopySelected, CropCancel, CropCopy, CropSave, NextImage, OpenFile, OpenFolder,
    OpenSelected, PrevImage, SelectAll, SelectNext, SelectPrev, ToggleCrop, ToggleFullscreen,
    ToggleOverlays, ToggleSelected, ToggleSlideshow,
};
use crate::state::session::{build_image_items, next_index, FitMode, Session};
use crate::state::theme_store::{hot_reload_decision, HotReloadDecision, ThemeStore};
use crate::state::view::View;
use crate::ui::grid;
use crate::ui::icons::{icon, IconName};
use crate::ui::overlay::{self, OverlayData};
use crate::ui::topbar;
use crate::ui::welcome;
use crate::viewer::{render_viewer, ViewerParams};
use gpui::prelude::*;
use gpui::*;
use sh_core::navigation::{SortBy, SortDir};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Instant;

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
    /// Multi-selection set (V3): grid indices, ascending + deduped by
    /// construction. Cursor + range anchor stays `grid_selected`. Cleared
    /// on folder swap and sort change; survives viewer round-trips.
    pub selected: BTreeSet<usize>,
    /// Manual grid scroll offset in px (wheel-driven, clamped).
    pub grid_scroll_px: f32,
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
    /// Settings dropdown open (gear button in the top bar).
    pub settings_open: bool,
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
            selected: BTreeSet::new(),
            grid_scroll_px: 0.0,
            thumbs: std::collections::HashMap::new(),
            thumb_seq: 0,
            recent_dirs_available: Vec::new(),
            settings_open: false,
            sort_menu_open: false,
            crop_mode: false,
            crop_rect: None,
            crop_bar_visible: false,
            topbar_was_hidden: false,
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
            self.session.error = Some(format!("No images in {}", dir.display()));
            // Empty folder: still clear + invalidate any previous
            // folder's thumb map (zero-path batch = clear + seq bump).
            self.spawn_thumb_batch(cx);
            cx.notify();
        }
        self.view = View::Grid;
        self.grid_selected = 0;
        // Work-in-progress selection never crosses folders.
        self.selected.clear();
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

    /// Enter the viewer at `idx`: set current + selection, switch view,
    /// probe/fit. Reuses [`Self::navigate`] so probe, seq-guard, fit, and
    /// persist all behave exactly like keyboard navigation.
    pub fn enter_viewer(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx < self.session.images.len() {
            self.session.current = idx;
            self.grid_selected = idx;
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
        // behavior); Shift+arrows go through `extend_selection_to` instead.
        self.selected.clear();
        // Scroll the selected row into view.
        let v = viewport_vec(self.viewport);
        let visible_h = (v.y - topbar::TOPBAR_H_PX).max(1.0);
        let cols = grid::grid_columns(v.x);
        let row_top = (self.grid_selected / cols) as f32 * grid::GRID_ROW_H_PX;
        let row_bottom = row_top + grid::GRID_ROW_H_PX;
        if row_top < self.grid_scroll_px {
            self.grid_scroll_px = row_top;
        } else if row_bottom > self.grid_scroll_px + visible_h {
            self.grid_scroll_px = row_bottom - visible_h;
        }
        let max = grid::grid_max_scroll(len, v.x, visible_h);
        self.grid_scroll_px = self.grid_scroll_px.clamp(0.0, max);
        self.note_interaction(cx);
        cx.notify();
    }

    /// Toggle one cell in the selection set (Ctrl+click / Ctrl+Space).
    /// The cursor follows the toggled cell (it becomes the range anchor).
    pub fn toggle_selected(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.session.images.len() {
            return;
        }
        if !self.selected.remove(&idx) {
            self.selected.insert(idx);
        }
        self.grid_selected = idx;
        self.note_interaction(cx);
        cx.notify();
    }

    /// Union the anchor→target range into the set (Shift+click /
    /// Shift+arrows). Union-only by design: narrowing needs Ctrl+click.
    pub fn extend_selection_to(&mut self, idx: usize, cx: &mut Context<Self>) {
        let len = self.session.images.len();
        if len == 0 {
            return;
        }
        let target = idx.min(len - 1);
        self.selected
            .extend(grid::selection_range(self.grid_selected, target));
        self.grid_selected = target;
        self.note_interaction(cx);
        cx.notify();
    }

    /// Fill the set (Ctrl+A). No-op on an empty grid.
    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.selected = (0..self.session.images.len()).collect();
        self.note_interaction(cx);
        cx.notify();
    }

    /// Empty the set (Escape in grid). Always notifies — callers use it as
    /// the consumed-gesture path.
    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.selected.clear();
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

    /// Apply a built-in theme by settings-file name: swap the store
    /// (theme + name + `%APPDATA%/themes` path), reset the hot-reload
    /// baseline and warn state, persist, and close the settings panel.
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
        self.settings.theme = file_name.to_string();
        self.settings_open = false;
        self.persist(cx);
        cx.notify();
        true
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

        // ── Probe current image dimensions on background thread ──
        // NOTE: fit is computed at COMPLETION time against the live viewport,
        // so a resize mid-probe picks up the current size.
        let path = self.session.images[next].path.clone();
        let bg = cx.background_executor();
        let probe_task = bg.spawn(async move { sh_core::decode::probe_dimensions(&path) });
        cx.spawn(async move |this, cx| {
            let dims = probe_task.await;
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
            let pre_task = bg.spawn(async move { sh_core::decode::probe_dimensions(&pre_path) });
            cx.spawn(async move |this, cx| {
                let pre = pre_task.await;
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
const SORT_MENU_ITEMS: &[(SortBy, &str)] = &[
    (SortBy::Name, "Name"),
    (SortBy::Created, "Created"),
    (SortBy::Modified, "Modified"),
    (SortBy::Size, "Size"),
    (SortBy::Type, "Type"),
];
const SORT_DIR_ITEMS: &[(SortDir, &str)] =
    &[(SortDir::Asc, "Ascending"), (SortDir::Desc, "Descending")];

/// Human label for the sort chip: criterion + direction arrow.
pub fn sort_chip_label(by: SortBy, dir: SortDir) -> String {
    let name = SORT_MENU_ITEMS
        .iter()
        .find(|(by2, _)| *by2 == by)
        .map(|(_, label)| *label)
        .unwrap_or("Name");
    let arrow = match dir {
        SortDir::Asc => "↑",
        SortDir::Desc => "↓",
    };
    format!("{name} {arrow}")
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
        let prev_btn: AnyElement = div()
            .id("prev-btn")
            .cursor_pointer()
            .child(icon(
                IconName::ChevronLeft,
                px(14.0),
                overlay_data.theme_text,
            ))
            .on_mouse_down(MouseButton::Left, swallow_prev)
            .on_click(on_prev)
            .into_any();
        let next_btn: AnyElement = div()
            .id("next-btn")
            .cursor_pointer()
            .child(icon(
                IconName::ChevronRight,
                px(14.0),
                overlay_data.theme_text,
            ))
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
        let slideshow_btn: AnyElement = div()
            .id("slideshow-btn")
            .cursor_pointer()
            .child(icon(
                slideshow_icon(self.session.slideshow_active),
                px(14.0),
                overlay_data.theme_text,
            ))
            .on_mouse_down(MouseButton::Left, swallow_slide)
            .on_click(on_toggle_slide)
            .into_any();

        let viewer = render_viewer(&params);

        // ── V2 Task 6: top bar data (grid: folder name; viewer: name — pos).
        // Built for every frame; only attached outside Welcome below.
        let topbar_data = topbar::TopbarData {
            left: self
                .recent_dirs_available
                .first()
                .and_then(|d| d.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string(),
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
                String::new()
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
        let topbar_el = if self.view != View::Welcome {
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
                .child("Back")
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
                .child("Open folder")
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
                        this.note_interaction(cx);
                        // Only one dropdown at a time: opening settings
                        // closes the sort menu (the sort chip mirrors this).
                        this.sort_menu_open = false;
                        this.settings_open = !this.settings_open;
                        cx.notify();
                    }),
                )
                .into_any();
            // V3 sort chip: shows the active criterion + direction; click
            // toggles the sort dropdown (mirrors the gear/settings pattern).
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
                .child(sort_chip_label(self.session.sort_by, self.session.sort_dir))
                .on_mouse_down(MouseButton::Left, swallow_sort_btn)
                .on_click(
                    cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                        this.note_interaction(cx);
                        // Only one dropdown at a time (same as gear button,
                        // which closes the sort menu symmetrically below).
                        this.settings_open = false;
                        this.sort_menu_open = !this.sort_menu_open;
                        cx.notify();
                    }),
                )
                .into_any();
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
            let bar = topbar::topbar(&topbar_data, back, open_btn, sort_btn, gear_btn, crop_btn);
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
        let welcome_el = if self.view == View::Welcome {
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
                            .child("Continue"),
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
                        .child("Open folder…"),
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
            // Cell hover plate: the SAME tint as the buttons (hover_fill over
            // the app background) — one hover language across the whole app,
            // dark and light themes alike.
            let cell_hover = hover_fill(bg, text);
            let mut cells: Vec<AnyElement> = Vec::with_capacity(self.session.images.len());
            for (idx, item) in self.session.images.iter().enumerate() {
                let selected = idx == self.grid_selected;
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
                        .w(px(160.0))
                        .h(px(120.0))
                        .rounded(px(10.0)) // 8 → 10 per spec
                        .into_any(),
                    None => div()
                        .id(("grid-thumb-empty", idx))
                        .w(px(160.0))
                        .h(px(120.0))
                        .bg(topbar_data.theme_surface)
                        .rounded(px(10.0))
                        .into_any(),
                };
                // Minimal active marker: a slim accent bar laid over the
                // thumbnail's bottom edge (streaming-app active pattern) —
                // no border box around the cell. The relative frame anchors
                // the absolutely-positioned bar to the thumb itself.
                let mut thumb_frame = div().relative().child(thumb);
                if selected {
                    // Inset 10px horizontally so the bar clears the thumb's
                    // rounded corners; 3px tall, pill-shaped.
                    thumb_frame = thumb_frame.child(
                        div()
                            .id(("grid-active-bar", idx))
                            .absolute()
                            .bottom(px(0.0))
                            .left(px(10.0))
                            .w(px(140.0))
                            .h(px(3.0))
                            .rounded(px(2.0))
                            .bg(accent),
                    );
                }
                let cell = div()
                    .id(("grid-cell", idx))
                    .w(px(grid::GRID_CELL_PX))
                    .cursor_pointer()
                    // Centered flex column: the cell is 180px while
                    // thumb+label are 160px — without centering the content
                    // sat flush left and the plate jutted 20px to the right.
                    // Centered, the plate reads as a symmetric card around
                    // the image. (Cell height = thumb + label, unchanged —
                    // GRID_ROW_H_PX math intact.)
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
                            .w(px(160.0))
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
        let grid_empty_el = if self.view == View::Grid && self.session.images.is_empty() {
            let text =
                parse_hex(&self.theme_store.theme.colors.text).unwrap_or(rgb(0xe8e8ee).into());
            let msg = self
                .session
                .error
                .clone()
                .unwrap_or_else(|| "No images in this folder".to_string());
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

        // ── Settings dropdown (Grid + Viewer): full-window click catcher
        // closes on outside click; the panel lists built-in themes with the
        // active one checked. Rendered last so both float above content.
        let (settings_catcher_el, settings_panel_el) = if self.settings_open
            && self.view != View::Welcome
        {
            let surface =
                parse_hex(&self.theme_store.theme.colors.surface).unwrap_or(rgb(0x121218).into());
            let text =
                parse_hex(&self.theme_store.theme.colors.text).unwrap_or(rgb(0xe8e8ee).into());
            let accent =
                parse_hex(&self.theme_store.theme.colors.accent).unwrap_or(rgb(0x00ffff).into());
            let row_bg = parse_hex(&self.theme_store.theme.colors.background)
                .unwrap_or(rgb(0x0d0d0f).into());
            let catcher: AnyElement = div()
                .id("settings-catcher")
                .absolute()
                .top(px(0.0))
                .left(px(0.0))
                .right(px(0.0))
                .bottom(px(0.0))
                .cursor_default()
                .on_mouse_down(
                    // Swallow the press so the grid cell / viewer gesture
                    // behind the catcher never arms (its mousedown handler
                    // sits further down the bubble chain).
                    MouseButton::Left,
                    cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                        cx.stop_propagation();
                    }),
                )
                .on_click(
                    cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                        this.settings_open = false;
                        cx.notify();
                    }),
                )
                .into_any();
            let mut list = div().flex().flex_col().gap(px(2.0));
            for (row_idx, (file, json)) in crate::theme_builtins::BUILTIN_THEMES.iter().enumerate()
            {
                let display = sh_core::theme::parse(json)
                    .map(|t| t.name)
                    .unwrap_or_else(|_| file.to_string());
                let active = *file == self.theme_store.name;
                let name = file.to_string();
                // Same swallow pattern as every other button: a row press
                // must not reach the grid cell behind the panel (two-click
                // bug — theme applied AND photo opened).
                let swallow_row =
                    cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                        cx.stop_propagation();
                    });
                let mut row = div()
                    .id(("theme-row", row_idx))
                    .flex()
                    .items_center()
                    .justify_between()
                    .cursor_pointer()
                    .rounded(px(6.0))
                    .px(px(10.0))
                    .py(px(6.0))
                    .hover(move |s| s.bg(row_bg))
                    .child(div().text_color(text).child(display))
                    .child(
                        div()
                            .text_color(if active { accent } else { text })
                            .child(if active { "✓" } else { "" }),
                    )
                    .on_mouse_down(MouseButton::Left, swallow_row)
                    .on_click(
                        cx.listener(move |this: &mut App, _ev: &ClickEvent, _window, cx| {
                            this.note_interaction(cx);
                            this.apply_builtin_theme(&name, cx);
                        }),
                    );
                // Active row keeps its check readable without hover.
                if active {
                    row = row.bg(row_bg);
                }
                list = list.child(row);
            }
            // The panel itself also swallows presses on its padding/header
            // — a click on "Theme" or the gaps between rows must only close
            // nothing and open nothing.
            let swallow_panel =
                cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                });
            let panel: AnyElement = div()
                .id("settings-panel")
                .absolute()
                .top(px(topbar::TOPBAR_H_PX + 8.0))
                .right(px(12.0))
                .bg(surface)
                .text_color(text)
                .rounded(px(8.0))
                .p(px(8.0))
                .on_mouse_down(MouseButton::Left, swallow_panel)
                .child(div().px(px(10.0)).py(px(4.0)).child("Theme"))
                .child(list)
                .into_any();
            (Some(catcher), Some(panel))
        } else {
            (None, None)
        };

        // ── Sort dropdown (Grid + Viewer): mirrors the settings catcher
        // pattern — full-window click catcher closes on outside click; the
        // menu lists criteria + directions with the active one checked.
        // Rendered last so both float above content.
        let (sort_catcher_el, sort_menu_el) = if self.sort_menu_open && self.view != View::Welcome {
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
            for (row_idx, (by, label)) in SORT_MENU_ITEMS.iter().enumerate() {
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
                    .child(div().text_color(text).child(*label))
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
            for (row_idx, (dir, label)) in SORT_DIR_ITEMS.iter().enumerate() {
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
                    .child(div().text_color(text).child(*label))
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
                .child(div().px(px(10.0)).py(px(4.0)).child("Sort by"))
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
                "Copy",
                |this: &mut App, _ev: &ClickEvent, _window, cx| {
                    this.confirm_crop_copy(cx);
                },
            );
            let save_btn = bar_btn(
                "crop-save",
                "Save…",
                |this: &mut App, _ev: &ClickEvent, _window, cx| {
                    this.confirm_crop_save(cx);
                },
            );
            let cancel_btn = bar_btn(
                "crop-cancel",
                "Cancel",
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
                            this.note_interaction(cx);
                            this.settings_open = !this.settings_open;
                            cx.notify();
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
                        Some(slideshow_btn),
                        Some(prev_btn),
                        Some(next_btn),
                    ))
                    .children(chips_el)
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
                this.extend_selection_to(this.grid_selected.saturating_add(1).min(len - 1), cx);
            }))
            .on_action(cx.listener(|this: &mut App, _: &SelectPrev, _window, cx| {
                if this.view != View::Grid {
                    return;
                }
                if this.session.images.is_empty() {
                    return;
                }
                this.extend_selection_to(this.grid_selected.saturating_sub(1), cx);
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
                // Settings panel intercepts Esc first (close it); crop mode
                // second (leave it); only then does Esc mean "back to grid".
                if this.settings_open {
                    this.settings_open = false;
                    cx.notify();
                    return;
                }
                if this.crop_mode {
                    this.cancel_crop(cx);
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
            // ── B3: wheel zoom anchored at cursor (Viewer only). In Grid the
            // wheel scrolls the thumbnail list instead (manual offset, see
            // ui::grid — gpui 0.2.2 has no scrollable plain div).
            .on_scroll_wheel(
                cx.listener(|this: &mut App, ev: &ScrollWheelEvent, _window, cx| {
                    if this.view == View::Welcome {
                        // Nothing to zoom or scroll yet; keep the idle clock.
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
            // ── Settings dropdown: click-catcher (closes on outside click)
            // under the panel, both absolute so they float over content. ──
            .children(settings_catcher_el)
            .children(settings_panel_el)
            // ── Sort dropdown: same catcher pattern as settings (V3). ──
            .children(sort_catcher_el)
            .children(sort_menu_el)
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
        hover_fill, hover_fill_strong, hover_tint, parse_hex, slideshow_icon, sort_chip_label,
        topbar_hidden, viewer_fit_height, App, SLIDESHOW_INTERVAL,
    };
    use crate::actions::{NextImage, PrevImage};
    use crate::state::session::{build_image_items, Session};
    use crate::state::theme_store::ThemeStore;
    use crate::state::view::View;
    use crate::ui::icons::IconName;
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

    #[test]
    fn sort_chip_label_formats_criterion_and_direction() {
        assert_eq!(
            sort_chip_label(SortBy::Name, SortDir::Asc),
            "Name ↑",
            "defaults must render as Name ↑"
        );
        assert_eq!(sort_chip_label(SortBy::Created, SortDir::Desc), "Created ↓");
        assert_eq!(
            sort_chip_label(SortBy::Modified, SortDir::Asc),
            "Modified ↑"
        );
        assert_eq!(sort_chip_label(SortBy::Size, SortDir::Desc), "Size ↓");
        assert_eq!(sort_chip_label(SortBy::Type, SortDir::Asc), "Type ↑");
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
            // Same bindings as production main.rs.
            cx.bind_keys([
                gpui::KeyBinding::new("right", NextImage, Some("image_view")),
                gpui::KeyBinding::new("left", PrevImage, Some("image_view")),
            ]);
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

    /// Toggle adds then removes; the cursor follows the toggled cell.
    #[gpui::test]
    fn toggle_select_adds_removes_and_moves_cursor(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.toggle_selected(2, cx);
        });
        app.read_with(cx, |app, _| {
            assert!(app.selected.contains(&2));
            assert_eq!(app.grid_selected, 2);
        });
        app.update(cx, |app, cx| {
            app.toggle_selected(2, cx);
        });
        app.read_with(cx, |app, _| {
            assert!(!app.selected.contains(&2));
            assert!(app.selected.is_empty());
        });
    }

    /// Shift-extend unions the anchor→target range; chaining extends further
    /// (union semantics — no separate anchor field needed).
    #[gpui::test]
    fn extend_unions_range_and_chains(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|_window, cx| test_app(cx));
        let cx = cx as &mut gpui::VisualTestContext;
        app.update(cx, |app, cx| {
            app.view = crate::state::view::View::Grid;
            app.grid_selected = 0;
            app.extend_selection_to(2, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.selected.iter().copied().collect::<Vec<_>>(),
                vec![0, 1, 2]
            );
            assert_eq!(app.grid_selected, 2);
        });
        // Reversed direction unions too.
        app.update(cx, |app, cx| {
            app.extend_selection_to(1, cx);
        });
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.selected.iter().copied().collect::<Vec<_>>(),
                vec![0, 1, 2]
            );
            assert_eq!(app.grid_selected, 1);
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
            app.extend_selection_to(1, cx);
            app.move_selection(1, cx);
        });
        app.read_with(cx, |app, _| {
            assert!(app.selected.is_empty());
            assert_eq!(app.grid_selected, 2);
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
            assert!(!app.settings_open);
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
}
