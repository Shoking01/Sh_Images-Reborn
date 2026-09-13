//! Root application component: session, theme, key dispatch, root render.

use crate::actions::{
    BackToGrid, CropCancel, CropCopy, CropSave, NextImage, OpenFile, OpenFolder, OpenSelected,
    PrevImage, ToggleCrop, ToggleFullscreen, ToggleOverlays,
};
use crate::state::session::{build_image_items, next_index, FitMode, Session};
use crate::state::theme_store::{hot_reload_decision, HotReloadDecision, ThemeStore};
use crate::state::view::View;
use crate::ui::grid;
use crate::ui::overlay::{self, OverlayData};
use crate::ui::topbar;
use crate::ui::welcome;
use crate::viewer::{render_viewer, ViewerParams};
use gpui::prelude::*;
use gpui::*;
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
    /// Manual grid scroll offset in px (wheel-driven, clamped).
    pub grid_scroll_px: f32,
    /// Decoded 256px thumbnails by path (grid cells). Cleared on every
    /// folder open; filled by one background task per open (seq-guarded).
    /// Full-resolution images NEVER live here — that was the 984MB grid.
    pub thumbs: std::collections::HashMap<PathBuf, std::sync::Arc<gpui::RenderImage>>,
    /// Sequence guarding thumb decode tasks against folder switches.
    pub thumb_seq: u64,
    /// Last folder known to exist, for the Welcome "Continue" affordance.
    pub last_dir_available: Option<PathBuf>,
    /// Settings dropdown open (gear button in the top bar).
    pub settings_open: bool,
    /// Crop mode: drag selects a region instead of panning.
    pub crop_mode: bool,
    /// Current selection in viewport px (drag order; normalized on confirm).
    pub crop_rect: Option<sh_core::crop::CropRect>,
    /// Confirm bar visible (a finished drag left a non-degenerate rect).
    pub crop_bar_visible: bool,
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
            grid_scroll_px: 0.0,
            thumbs: std::collections::HashMap::new(),
            thumb_seq: 0,
            last_dir_available: None,
            settings_open: false,
            crop_mode: false,
            crop_rect: None,
            crop_bar_visible: false,
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

    /// Open a path: resolve the sibling image list, show the first image,
    /// reset state, persist, and kick off the initial probe/fit.
    ///
    /// A resolve error (e.g. dropping an unsupported file) surfaces in the
    /// session error slot instead of panicking.
    pub fn open_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        match sh_core::navigation::resolve(&path) {
            Ok(list) => {
                self.session.error = None;
                self.session.current = list.current;
                self.session.images = build_image_items(list.paths);
                self.session.show_overlay_bottom = true;
                cx.notify();
                self.persist(cx);
                self.navigate(0, cx);
            }
            Err(e) => {
                self.session.error = Some(e.to_string());
                cx.notify();
            }
        }
    }

    /// Open a folder: resolve its first image (or surface "no images" in the
    /// session error slot), enter the Grid view, persist, and probe.
    /// A resolve error surfaces in `session.error` instead of panicking;
    /// the view still switches to Grid so the empty-state renders with context.
    pub fn open_folder(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        match sh_core::navigation::first_supported(&dir) {
            Some(first) => self.open_path(first, cx),
            None => {
                self.session.images = Vec::new();
                self.session.current = 0;
                self.session.error = Some(format!("No images in {}", dir.display()));
                cx.notify();
            }
        }
        self.view = View::Grid;
        self.grid_selected = 0;
        self.grid_scroll_px = 0.0;
        self.last_dir_available = Some(dir);
        // Thumbnails: one background task decodes every image at 256px
        // (sequential — each decode is milliseconds) and commits the batch
        // under a seq guard, so a folder switch mid-decode drops stale work
        // instead of polluting the new folder's map. The map is cleared
        // first so no previous folder's thumbs linger.
        self.thumbs.clear();
        self.thumb_seq = self.thumb_seq.wrapping_add(1);
        let seq = self.thumb_seq;
        let paths: Vec<PathBuf> = self.session.images.iter().map(|i| i.path.clone()).collect();
        // Parallel chunks: 8 concurrent decodes trade a brief, bounded
        // transient peak (~8 full images) for ~8x load speed, then settle to
        // thumbs-only. Commits stay progressive (first paint ~200ms).
        // Stale folders drop work via the seq guard per commit.
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
        cx.notify();
    }

    /// Viewport available to the image: full window minus the persistent top
    /// bar. ALL fit math (navigate completion, toggle, clamp, wheel) must use
    /// this, never the raw window viewport.
    pub fn viewer_viewport(&self) -> sh_core::transform::Vec2 {
        let v = viewport_vec(self.viewport);
        sh_core::transform::Vec2 {
            x: v.x,
            y: (v.y - topbar::TOPBAR_H_PX).max(1.0),
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

    /// Enter crop mode: drag will select a region instead of panning.
    pub fn enter_crop(&mut self, cx: &mut Context<Self>) {
        self.crop_mode = true;
        self.crop_rect = None;
        self.crop_bar_visible = false;
        self.drag_last = None;
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

    /// Toggle crop mode (the `C` key / ✂ button).
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

    /// Confirm-bar "Copiar": crop the ORIGINAL file at the converted rect,
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

    /// Confirm-bar "Guardar…": show the save dialog (rfd async, PNG filter,
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
        let start_dir = self.last_dir_available.clone();
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
        let last_dir = self
            .session
            .current_item()
            .and_then(|i| i.path.parent().map(std::path::Path::to_path_buf));
        // Keep the in-memory copy truthful for the next persist.
        self.settings.last_dir = last_dir.clone();
        // `last_dir_available` is refreshed here (every persist: file dialog,
        // drag&drop, navigate-folder-change) and in `open_folder` (covers the
        // empty-folder arm that skips persist); `open_path` alone never
        // touches it directly.
        self.last_dir_available = last_dir.clone();
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

/// Expose the root focus handle so external code (and GPUI's
/// `window.focus_view`) can focus the app's `image_view` subtree.
impl Focusable for App {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// Cadence of the idle watcher poll. Independent of [`overlay::OVERLAY_IDLE`]
/// (the actual hide threshold) — a short tick keeps the hide within ~500ms
/// of the deadline without notifying more than once.
const IDLE_TICK: std::time::Duration = std::time::Duration::from_millis(500);

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
            .child("◀")
            .on_mouse_down(MouseButton::Left, swallow_prev)
            .on_click(on_prev)
            .into_any();
        let next_btn: AnyElement = div()
            .id("next-btn")
            .cursor_pointer()
            .child("▶")
            .on_mouse_down(MouseButton::Left, swallow_next)
            .on_click(on_next)
            .into_any();

        let viewer = render_viewer(&params);

        // ── V2 Task 6: top bar data (grid: folder name; viewer: name — pos).
        // Built for every frame; only attached outside Welcome below.
        let topbar_data = topbar::TopbarData {
            left: self
                .last_dir_available
                .as_ref()
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
        let swallow_crop_btn = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
            cx.stop_propagation();
        });
        let topbar_el = if self.view != View::Welcome {
            // Button chips sit on the surface bar, so they use the app
            // background for contrast (same text color as the bar).
            let btn_bg = parse_hex(&self.theme_store.theme.colors.background)
                .unwrap_or(rgb(0x0d0d0f).into());
            let btn_hover =
                parse_hex(&self.theme_store.theme.colors.accent).unwrap_or(rgb(0x00ffff).into());
            let back_btn: AnyElement = div()
                .id("topbar-back")
                .cursor_pointer()
                .bg(btn_bg)
                .border(px(1.0))
                .border_color(btn_bg)
                .hover(move |s| s.border_color(btn_hover))
                .text_color(topbar_data.theme_text)
                .rounded(px(6.0))
                .px(px(12.0))
                .py(px(4.0))
                .child("← Atrás")
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
                .border(px(1.0))
                .border_color(btn_bg)
                .hover(move |s| s.border_color(btn_hover))
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
                .border(px(1.0))
                .border_color(btn_bg)
                .hover(move |s| s.border_color(btn_hover))
                .text_color(topbar_data.theme_text)
                .rounded(px(6.0))
                .px(px(10.0))
                .py(px(4.0))
                .child("⚙")
                .on_mouse_down(MouseButton::Left, swallow_gear_btn)
                .on_click(
                    cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                        this.note_interaction(cx);
                        this.settings_open = !this.settings_open;
                        cx.notify();
                    }),
                )
                .into_any();
            // ✂ enters crop mode (Viewer only); active mode shows pressed.
            let crop_btn = if self.view == View::Viewer {
                let label = if self.crop_mode { "✂ ✓" } else { "✂" };
                Some(
                    div()
                        .id("topbar-crop")
                        .cursor_pointer()
                        .bg(btn_bg)
                        .border(px(1.0))
                        .border_color(if self.crop_mode {
                            topbar_data.theme_text
                        } else {
                            btn_bg
                        })
                        .hover(move |s| s.border_color(btn_hover))
                        .text_color(topbar_data.theme_text)
                        .rounded(px(6.0))
                        .px(px(10.0))
                        .py(px(4.0))
                        .child(label)
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
            Some(
                topbar::topbar(&topbar_data, back, open_btn, gear_btn, crop_btn).into_any_element(),
            )
        } else {
            None
        };

        // ── V2: view-specific content ──
        // Welcome: startup screen with Continue / Open-folder. Buttons are
        // built here (cx.listener call-site pattern, same as overlay arrows).
        let welcome_el = if self.view == View::Welcome {
            let welcome_data = welcome::WelcomeData::from_theme(
                self.last_dir_available
                    .as_ref()
                    .map(|d| d.display().to_string()),
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
            let continue_btn = self.last_dir_available.clone().map(|dir| {
                let btn = div()
                    .id("welcome-continue")
                    .cursor_pointer()
                    .bg(welcome_data.theme_surface)
                    .border(px(1.0))
                    .border_color(welcome_data.theme_surface)
                    .hover(|s| s.border_color(welcome_data.theme_accent))
                    .text_color(welcome_data.theme_text)
                    .rounded(px(6.0))
                    .px(px(16.0))
                    .py(px(8.0))
                    .child("Continue →")
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
                .border(px(1.0))
                .border_color(welcome_data.theme_surface)
                .hover(|s| s.border_color(welcome_data.theme_accent))
                .text_color(welcome_data.theme_text)
                .rounded(px(6.0))
                .px(px(16.0))
                .py(px(8.0))
                .child("Open folder…")
                .on_mouse_down(MouseButton::Left, swallow_open)
                .on_click(
                    cx.listener(|this: &mut App, _ev: &ClickEvent, _window, cx| {
                        this.pick_folder(cx);
                    }),
                )
                .into_any();
            Some(
                welcome::welcome(&welcome_data, continue_btn.map(|b| b.into_any()), open_btn)
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
            let mut cells: Vec<AnyElement> = Vec::with_capacity(self.session.images.len());
            for (idx, item) in self.session.images.iter().enumerate() {
                let selected = idx == self.grid_selected;
                let swallow_cell =
                    cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                        cx.stop_propagation();
                    });
                // Thumb or placeholder: a missing decode (batch still
                // running, slow/corrupt file) shows the themed chip
                // until the background batch lands.
                let thumb: AnyElement = match self.thumbs.get(&item.path) {
                    Some(arc) => img(arc.clone())
                        .id(("grid-thumb", idx))
                        .w(px(160.0))
                        .h(px(120.0))
                        .rounded(px(8.0))
                        .into_any(),
                    None => div()
                        .id(("grid-thumb-empty", idx))
                        .w(px(160.0))
                        .h(px(120.0))
                        .bg(topbar_data.theme_surface)
                        .rounded(px(8.0))
                        .into_any(),
                };
                let mut cell = div()
                    .id(("grid-cell", idx))
                    .w(px(grid::GRID_CELL_PX))
                    .cursor_pointer()
                    .child(thumb)
                    // Single-line ellipsis: a wrapped label grows the row
                    // and breaks the scroll math (see GRID_ROW_H_PX).
                    .child(
                        div()
                            .w(px(160.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_color(text)
                            .child(item.name.clone()),
                    )
                    .on_mouse_down(MouseButton::Left, swallow_cell)
                    .on_click(
                        cx.listener(move |this: &mut App, _ev: &ClickEvent, _window, cx| {
                            this.note_interaction(cx);
                            this.enter_viewer(idx, cx);
                        }),
                    );
                if selected {
                    cell = cell.border(px(2.0)).border_color(accent);
                }
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
        // ── Task 5: crop confirm bar (Copiar / Guardar / Cancelar) ──
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
            let bar_btn = |id: &'static str,
                           label: &'static str,
                           on_click: fn(&mut App, &ClickEvent, &mut Window, &mut Context<App>),
                           hover: Hsla|
             -> AnyElement {
                let swallow = cx.listener(|_this: &mut App, _ev: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                });
                div()
                    .id(id)
                    .cursor_pointer()
                    .bg(surface)
                    .border(px(1.0))
                    .border_color(surface)
                    .hover(move |s| s.border_color(hover))
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
                "Copiar",
                |this: &mut App, _ev: &ClickEvent, _window, cx| {
                    this.confirm_crop_copy(cx);
                },
                accent,
            );
            let save_btn = bar_btn(
                "crop-save",
                "Guardar…",
                |this: &mut App, _ev: &ClickEvent, _window, cx| {
                    this.confirm_crop_save(cx);
                },
                accent,
            );
            let cancel_btn = bar_btn(
                "crop-cancel",
                "Cancelar",
                |this: &mut App, _ev: &ClickEvent, _window, cx| {
                    this.cancel_crop(cx);
                },
                accent,
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
                        Some(prev_btn),
                        Some(next_btn),
                    ))
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
    use super::{parse_hex, topbar_hidden, viewer_fit_height, App};
    use crate::actions::{NextImage, PrevImage};
    use crate::state::session::{build_image_items, Session};
    use crate::state::theme_store::ThemeStore;
    use std::path::PathBuf;

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

    /// Build a minimal App for focus-dispatch tests: two fake images, the
    /// built-in theme, default settings. Paths don't need to exist — the
    /// dimension probe failing merely sets the session error slot, which
    /// these tests never assert on.
    fn test_app(cx: &mut gpui::Context<App>) -> App {
        let session = Session {
            images: build_image_items(vec![
                PathBuf::from("Z:\\fake\\a.png"),
                PathBuf::from("Z:\\fake\\b.png"),
                PathBuf::from("Z:\\fake\\c.png"),
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
        App::new(
            session,
            theme_store,
            PathBuf::from("Z:\\fake\\settings.json"),
            sh_core::settings::Settings::default(),
            theme_text,
            cx,
        )
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
