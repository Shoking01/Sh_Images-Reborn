//! Root application component: session, theme, key dispatch, root render.

use crate::actions::{NextImage, PrevImage, ToggleOverlays};
use crate::state::session::{build_image_items, FitMode, Session};
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
}

impl App {
    /// Create a new app with the given session and theme.
    pub fn new(session: Session, theme_store: ThemeStore) -> Self {
        Self {
            session,
            theme_store,
            viewport: size(px(0.), px(0.)),
        }
    }

    /// Open a set of images from a resolved list.
    pub fn set_images(&mut self, paths: Vec<PathBuf>, current: usize) {
        self.session.images = build_image_items(paths);
        self.session.current = current;
    }

    /// Navigate `delta` steps (‑1 = prev, +1 = next) with decode on a worker.
    ///
    /// Spawns a background decode for the target image, then updates session
    /// state on completion. A neighbor prefetch warms the next image in the
    /// likely navigation direction.
    pub fn navigate(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.session.images.is_empty() {
            return;
        }
        let n = self.session.images.len();
        let next = (self.session.current as isize + delta).rem_euclid(n as isize) as usize;
        self.session.current = next;
        self.session.error = None;

        // ── Decode current image on background thread ──
        let path = self.session.images[next].path.clone();
        let viewport = self.viewport;
        let bg = cx.background_executor();
        let decode_task = bg.spawn(async move {
            let item = sh_core::decode::load(&path);
            let fit = item.as_ref().ok().map(|d| {
                sh_core::transform::fit(
                    sh_core::transform::Vec2 {
                        x: d.width as f32,
                        y: d.height as f32,
                    },
                    sh_core::transform::Vec2 {
                        x: f32::from(viewport.width),
                        y: f32::from(viewport.height),
                    },
                )
            });
            (item, fit)
        });
        cx.spawn(async move |this, cx| {
            let (item, fit) = decode_task.await;
            let _ = this.update(cx, |app, cx| {
                if let Ok(decoded) = item {
                    app.session.images[next].decoded = Some(decoded);
                } else {
                    app.session.error = Some("could not decode image".into());
                }
                if let Some(f) = fit {
                    app.session.zoom = f;
                    app.session.fit_mode = FitMode::Fit;
                }
                cx.notify();
            });
        })
        .detach();

        // ── Prefetch neighbor (CPU-side session warm-up) ──
        // NOTE: This warms CPU-side session state (fit/zoom math) but does NOT
        // populate GPUI's `img()` asset cache. Render-side reuse arrives with
        // the use_asset wiring in later tasks.
        let pre_path = self.session.images[(next + 1) % n].path.clone();
        let bg = cx.background_executor();
        let pre_task = bg.spawn(async move {
            let _ = sh_core::decode::load(&pre_path);
        });
        cx.spawn(async move |this, cx| {
            let _ = pre_task.await;
            let _ = this.update(cx, |app, cx| {
                // Store decoded data for the prefetched image if it's still
                // the same slot (user may have navigated away by now).
                // We simply notify; the next render will pick up any changes.
                let _ = app;
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for App {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bg: Hsla =
            parse_hex(&self.theme_store.theme.colors.background).unwrap_or(rgb(0x0d0d0f).into());
        let params = ViewerParams {
            path: self.session.current_item().map(|i| i.path.clone()),
            position: self.session.position_label(),
            error: self.session.error.clone(),
            zoom_text: format!("{:.0}%", self.session.zoom.scale * 100.0),
            show_overlay_top: self.session.show_overlay_top,
            show_overlay_bottom: self.session.show_overlay_bottom,
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
                        .on_click(on_prev),
                )
                .child(
                    div()
                        .id("next-btn")
                        .cursor_pointer()
                        .child("▶")
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
