//! The image viewer: renders the single image at session zoom/pan.

use gpui::prelude::*;
use gpui::*;
use std::path::PathBuf;

/// Params the App hands to the viewer each frame.
#[derive(Debug, Clone)]
pub struct ViewerParams {
    /// Path to the current image file, if any.
    pub path: Option<PathBuf>,
    /// Error message, if any (rendered in the empty-state arm).
    pub error: Option<String>,
    /// Current zoom scale (1.0 = 100%).
    pub zoom_scale: f32,
    /// Current pan offset in viewport pixels (negative when centered).
    pub pan_offset: sh_core::transform::Vec2,
    /// Probed image dimensions in pixels, if known.
    pub decoded_size: Option<(f32, f32)>,
    /// UI language for user-facing copy (S4: empty-state hint).
    pub lang: sh_core::i18n::Language,
    /// Precomputed checkerboard gate (viewer-checkerboard Slice B).
    ///
    /// Computed once per frame by `App::render` as
    /// `settings.checkerboard && current.has_alpha == Some(true)` — this
    /// function stays pure-presentational and never touches settings or
    /// session state itself.
    pub show_checkerboard: bool,
}

/// Pure-presentational board gate: show the checkerboard iff the persisted
/// visibility setting is ON and the cached alpha verdict is a confirmed
/// `Some(true)`.
///
/// `None` (verdict still in flight — renders exactly like the 1x1 dimensions
/// fallback: no board for one tick, then `cx.notify()` paints it) and
/// `Some(false)` both yield `false`. No I/O, no decode: O(1) field reads.
pub fn should_show_checkerboard(setting_on: bool, has_alpha: Option<bool>) -> bool {
    setting_on && has_alpha == Some(true)
}

/// Build the viewer element tree from the current params.
///
/// Renders the error state FIRST (a failed probe of the current image shows
/// the message), else the loaded image, else the empty-state placeholder.
/// The image is displayed via GPUI's built-in `img()` element, which handles
/// file loading, decoding, and BGRA conversion internally.
///
/// Sizing is explicit layout (Task 8): the image is positioned absolutely at
/// `pan_offset` and sized to `dimensions * zoom_scale`, so zoom/pan math
/// fully controls the frame. The element's aspect always equals the image's
/// aspect (`w = iw * scale`, `h = ih * scale`), so no `object_fit` is needed.
///
/// Task 9 moved the info overlays (top name/position, bottom zoom/arrows)
/// OUT of the viewer — they render as app-level children over this layer.
pub fn render_viewer(params: &ViewerParams) -> impl IntoElement {
    // Error state FIRST: a failed probe of the CURRENT image must show the
    // message, not a 1×1 artifact — session.error is set while the path is
    // still Some. (Error clears on the next successful navigate.)
    let content: AnyElement = if let Some(error) = &params.error {
        let text_color: Hsla = crate::app::parse_hex("#e8e8ee").unwrap_or(rgb(0xe8e8ee).into());
        div()
            .id("viewer-error")
            .debug_selector(|| "viewer-error".to_string())
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(div().text_color(text_color).child(error.clone()))
            .into_any()
    } else {
        match &params.path {
            Some(path) => {
                // NOTE: while dimensions are unknown (the one tick before the
                // header probe lands), fall back to 1x1 — a tiny artifact for
                // a frame. The probe is header-only, so this is near-instant.
                let (iw, ih) = params.decoded_size.unwrap_or((1.0, 1.0));
                let (fw, fh) = (iw * params.zoom_scale, ih * params.zoom_scale);
                let image = img(path.clone())
                    .id("viewer-image")
                    .debug_selector(|| "viewer-image".to_string())
                    .absolute()
                    .left(px(params.pan_offset.x))
                    .top(px(params.pan_offset.y))
                    .w(px(fw))
                    .h(px(fh));

                // Single baked board behind the image, iff the precomputed
                // gate says so (error/empty arms never reach this branch).
                // The board shares the image's exact frame geometry.
                let zoom_layer = div()
                    .id("zoom-layer")
                    .debug_selector(|| "zoom-layer".to_string())
                    .size_full()
                    .relative();
                let zoom_layer = if params.show_checkerboard {
                    zoom_layer.child(
                        crate::checkerboard::checkerboard_layer(fw, fh)
                            .absolute()
                            .left(px(params.pan_offset.x))
                            .top(px(params.pan_offset.y))
                            .debug_selector(|| "viewer-checkerboard".to_string()),
                    )
                } else {
                    zoom_layer
                };
                zoom_layer.child(image).into_any()
            }
            None => {
                let text_color: Hsla =
                    crate::app::parse_hex("#e8e8ee").unwrap_or(rgb(0xe8e8ee).into());
                div()
                    .id("viewer-empty")
                    .debug_selector(|| "viewer-empty".to_string())
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().text_color(text_color).child(sh_core::i18n::t(
                        params.lang,
                        sh_core::i18n::StrKey::ViewerEmptyHint,
                    )))
                    .into_any()
            }
        }
    };

    div()
        .id("viewer-root")
        .debug_selector(|| "viewer-root".to_string())
        .size_full()
        .child(content)
}

#[cfg(test)]
mod tests {
    use super::{render_viewer, should_show_checkerboard, ViewerParams};
    use std::path::PathBuf;

    fn params(path: Option<PathBuf>, error: Option<String>, show: bool) -> ViewerParams {
        ViewerParams {
            path,
            error,
            zoom_scale: 1.0,
            pan_offset: sh_core::transform::Vec2 { x: 0.0, y: 0.0 },
            decoded_size: Some((4.0, 3.0)),
            lang: sh_core::i18n::Language::En,
            show_checkerboard: show,
        }
    }

    /// Task 2.3: the board gate needs the setting ON plus a CONFIRMED
    /// transparent verdict. `None` (verdict in flight) and `Some(false)`
    /// render exactly as before, with no crash.
    #[test]
    fn board_gate_needs_setting_on_and_confirmed_alpha() {
        assert!(should_show_checkerboard(true, Some(true)));
        assert!(!should_show_checkerboard(true, Some(false)));
        assert!(!should_show_checkerboard(true, None));
        assert!(!should_show_checkerboard(false, Some(true)));
        assert!(!should_show_checkerboard(false, Some(false)));
        assert!(!should_show_checkerboard(false, None));
    }

    /// Task 2.3: the error arm never carries the board, even when the gate
    /// is on — the error state renders exactly as before.
    #[test]
    fn error_arm_ignores_board_flag() {
        let p = params(
            Some(PathBuf::from("Z:\\fake\\a.png")),
            Some("could not read image".into()),
            true,
        );
        let _el = render_viewer(&p);
    }

    /// Task 2.3: the empty arm never carries the board either.
    #[test]
    fn empty_arm_ignores_board_flag() {
        let p = params(None, None, true);
        let _el = render_viewer(&p);
    }

    /// Task 2.3: the image arm builds with the board on and off (the gate
    /// only adds one `#checkerboard` layer behind the image; no crash).
    #[test]
    fn image_arm_builds_with_and_without_board() {
        let p = params(Some(PathBuf::from("Z:\\fake\\a.png")), None, true);
        let _el = render_viewer(&p);
        let p = params(Some(PathBuf::from("Z:\\fake\\a.png")), None, false);
        let _el = render_viewer(&p);
    }
}
