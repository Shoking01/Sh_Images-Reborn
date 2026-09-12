//! Root application component: session, theme, key dispatch, root render.

use crate::actions::{NextImage, OpenFile, PrevImage, ToggleFullscreen, ToggleOverlays};
use crate::state::session::{build_image_items, next_index, FitMode, Session};
use crate::state::theme_store::{hot_reload_decision, HotReloadDecision, ThemeStore};
use crate::state::view::View;
use crate::ui::overlay::{self, OverlayData};
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
    /// Last folder known to exist, for the Welcome "Continue" affordance.
    pub last_dir_available: Option<PathBuf>,
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
            last_dir_available: None,
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
                    let overlays_on =
                        app.session.show_overlay_top || app.session.show_overlay_bottom;
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
                self.session.show_overlay_top = true;
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
        self.last_dir_available = Some(dir);
        cx.notify();
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
                            viewport_vec(app.viewport),
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
            self.session.refit_for_viewport(viewport_vec(new_viewport));
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

        // ── Task 9: overlay visibility = Tab-toggled && not idle ──
        let idle = self.last_interaction.elapsed() > overlay::OVERLAY_IDLE;
        let top_visible = self.session.show_overlay_top && !idle;
        let bottom_visible = self.session.show_overlay_bottom && !idle;

        let overlay_data = OverlayData::from_theme(
            self.session
                .current_item()
                .map(|i| i.name.clone())
                .unwrap_or_default(),
            self.session.position_label(),
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

        div()
            .id("app-root")
            .size_full()
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
                this.note_interaction(cx);
                this.navigate(1, cx);
            }))
            .on_action(cx.listener(|this: &mut App, _: &PrevImage, _window, cx| {
                this.note_interaction(cx);
                this.navigate(-1, cx);
            }))
            .on_action(
                cx.listener(|this: &mut App, _: &ToggleOverlays, _window, cx| {
                    this.note_interaction(cx);
                    this.session.show_overlay_top = !this.session.show_overlay_top;
                    this.session.show_overlay_bottom = !this.session.show_overlay_bottom;
                    cx.notify();
                }),
            )
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
                        });
                    }
                })
                .detach();
            }))
            // ── B3: wheel zoom anchored at cursor ──
            .on_scroll_wheel(
                cx.listener(|this: &mut App, ev: &ScrollWheelEvent, _window, cx| {
                    this.note_interaction(cx);
                    let view = viewport_vec(this.viewport);
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
            // ── B3: double-click toggles fit/100%; single press starts pan drag ──
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this: &mut App, ev: &MouseDownEvent, _window, cx| {
                    if ev.click_count == 2 {
                        this.session.toggle_fit_100(viewport_vec(this.viewport));
                        this.drag_last = None;
                    } else {
                        this.drag_last = Some(ev.position);
                    }
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this: &mut App, _ev: &MouseUpEvent, _window, _cx| {
                    this.drag_last = None;
                }),
            )
            .on_mouse_move(
                cx.listener(|this: &mut App, ev: &MouseMoveEvent, _window, cx| {
                    if ev.dragging() {
                        // Drag pan: a real gesture, always resets the clock.
                        this.note_interaction(cx);
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
            // ── Task 10: drag & drop opens the first dropped image's folder ──
            .on_drop(
                cx.listener(|this: &mut App, paths: &ExternalPaths, _window, cx| {
                    this.note_interaction(cx);
                    if let Some(path) = paths.paths().first() {
                        this.open_path(path.clone(), cx);
                    }
                }),
            )
            .child(viewer)
            // ── Task 9: ephemeral overlays (app-level, over the viewer) ──
            .child(overlay::top(&overlay_data, top_visible))
            .child(overlay::bottom(
                &overlay_data,
                bottom_visible,
                Some(prev_btn),
                Some(next_btn),
            ))
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
    use super::{parse_hex, App};
    use crate::actions::{NextImage, PrevImage};
    use crate::state::session::{build_image_items, Session};
    use crate::state::theme_store::ThemeStore;
    use std::path::PathBuf;

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
}
