//! Root application component: session, theme, key dispatch, root render.

use crate::actions::{NextImage, PrevImage, ToggleOverlays};
use crate::state::session::{build_image_items, next_index, FitMode, Session};
use crate::state::theme_store::ThemeStore;
use crate::viewer::{render_viewer, ViewerParams};
use gpui::prelude::*;
use gpui::*;
use std::path::PathBuf;

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
}

impl App {
    /// Create a new app with the given session and theme.
    pub fn new(session: Session, theme_store: ThemeStore) -> Self {
        Self {
            session,
            theme_store,
            viewport: size(px(0.), px(0.)),
            navigation_seq: 0,
            drag_last: None,
        }
    }

    /// Open a set of images from a resolved list.
    pub fn set_images(&mut self, paths: Vec<PathBuf>, current: usize) {
        self.session.images = build_image_items(paths);
        self.session.current = current;
    }

    /// Navigate `delta` steps (‑1 = prev, +1 = next) with a header probe on a worker.
    ///
    /// Spawns a background dimension probe for the target image (never a full
    /// pixel decode), then updates session state on completion. A neighbor
    /// prefetch warms the next image's dimensions.
    ///
    /// Completions carry a monotonic sequence number: only the completion
    /// belonging to the *latest* navigation may commit global state
    /// (`zoom`/`fit_mode`/`error`). Stale completions still warm their slot's
    /// `dimensions` but leave global state untouched.
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
                // The slot write is always safe: `next` is the right index for
                // this image even if the user has since navigated away.
                if let Ok(d) = dims {
                    app.session.images[next].dimensions = Some(d);
                }
                // Only the latest navigation commits global state.
                if seq == app.navigation_seq {
                    match app.session.images[next].dimensions {
                        Some((w, h)) => {
                            app.session.zoom = sh_core::transform::fit(
                                sh_core::transform::Vec2 {
                                    x: w as f32,
                                    y: h as f32,
                                },
                                viewport_vec(app.viewport),
                            );
                            app.session.fit_mode = FitMode::Fit;
                        }
                        None => {
                            app.session.error = Some("could not read image".into());
                        }
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

impl Render for App {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Live viewport: GPUI re-renders on window resize (`on_resize` →
        // `bounds_changed` → `refresh`), so reading the drawable size here
        // keeps `App.viewport` current on every frame.
        self.viewport = window.viewport_size();

        let bg: Hsla =
            parse_hex(&self.theme_store.theme.colors.background).unwrap_or(rgb(0x0d0d0f).into());
        let params = ViewerParams {
            path: self.session.current_item().map(|i| i.path.clone()),
            position: self.session.position_label(),
            error: self.session.error.clone(),
            zoom_text: format!("{:.0}%", self.session.zoom.scale * 100.0),
            show_overlay_top: self.session.show_overlay_top,
            show_overlay_bottom: self.session.show_overlay_bottom,
            zoom_scale: self.session.zoom.scale,
            pan_offset: self.session.zoom.offset,
            decoded_size: self
                .session
                .current_dimensions()
                .map(|(w, h)| (w as f32, h as f32)),
        };

        // Build nav arrow elements for the viewer. Gated by show_overlay_bottom
        // so Task 9's overlay fade supersedes cleanly (Tab enables them).
        let viewer = if self.session.show_overlay_bottom {
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
            let arrows: AnyElement = div()
                .id("nav-bar")
                .absolute()
                .bottom_0()
                .w_full()
                .flex()
                .justify_center()
                .gap_2()
                .child(
                    div()
                        .id("prev-btn")
                        .cursor_pointer()
                        .child("◀")
                        .on_mouse_down(MouseButton::Left, swallow_prev)
                        .on_click(on_prev),
                )
                .child(
                    div()
                        .id("next-btn")
                        .cursor_pointer()
                        .child("▶")
                        .on_mouse_down(MouseButton::Left, swallow_next)
                        .on_click(on_next),
                )
                .into_any();
            render_viewer(&params, Some(arrows))
        } else {
            render_viewer(&params, None)
        };

        div()
            .id("app-root")
            .size_full()
            .key_context("image_view")
            .on_action(cx.listener(|this: &mut App, _: &NextImage, _window, cx| {
                this.navigate(1, cx);
            }))
            .on_action(cx.listener(|this: &mut App, _: &PrevImage, _window, cx| {
                this.navigate(-1, cx);
            }))
            .on_action(
                cx.listener(|this: &mut App, _: &ToggleOverlays, _window, cx| {
                    this.session.show_overlay_top = !this.session.show_overlay_top;
                    this.session.show_overlay_bottom = !this.session.show_overlay_bottom;
                    cx.notify();
                }),
            )
            // ── B3: wheel zoom anchored at cursor ──
            .on_scroll_wheel(
                cx.listener(|this: &mut App, ev: &ScrollWheelEvent, _window, cx| {
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
                        if let Some(last) = this.drag_last {
                            this.session.pan(sh_core::transform::Vec2 {
                                x: f32::from(ev.position.x - last.x),
                                y: f32::from(ev.position.y - last.y),
                            });
                            cx.notify();
                        }
                        this.drag_last = Some(ev.position);
                    } else {
                        // Not dragging: clear arming. This also self-heals a
                        // drag whose button was released outside the window
                        // (the mouse-up there never reaches `on_mouse_up`
                        // because bubble listeners filter by hitbox).
                        this.drag_last = None;
                    }
                }),
            )
            .bg(bg)
            .on_drop(cx.listener(|_this, _paths: &ExternalPaths, _window, _cx| {
                // Task 10 wires full open flow
            }))
            .child(viewer)
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
    use super::parse_hex;

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
}
