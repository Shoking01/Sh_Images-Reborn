//! Ephemeral overlays: top (name + position) and bottom (zoom + controls).

use crate::app::parse_hex;
use gpui::prelude::*;
use gpui::*;

/// Idle time after which overlays fade out.
pub const OVERLAY_IDLE: std::time::Duration = std::time::Duration::from_millis(1500);

/// Data needed to render overlays.
#[derive(Debug, Clone)]
pub struct OverlayData {
    /// Current image file name.
    pub name: String,
    /// Position label like "3/12".
    pub position: String,
    /// Zoom level text like "100%".
    pub zoom_text: String,
    /// Text color from the active theme.
    pub theme_text: Hsla,
    /// Surface color from the active theme (overlay chip background).
    pub theme_surface: Hsla,
}

impl OverlayData {
    /// Build overlay data from theme color strings, with sensible fallbacks
    /// for missing/invalid entries.
    pub fn from_theme(
        name: String,
        position: String,
        zoom_text: String,
        text_hex: &str,
        surface_hex: &str,
    ) -> Self {
        Self {
            name,
            position,
            zoom_text,
            theme_text: parse_hex(text_hex).unwrap_or(rgb(0xe8e8ee).into()),
            theme_surface: parse_hex(surface_hex).unwrap_or(rgb(0x121218).into()),
        }
    }
}

/// Render the top overlay (filename + position).
///
/// Always renders the element (visibility via opacity, not tree-shape) so the
/// layout is stable and focus is not reset on toggle.
pub fn top(overlay: &OverlayData, visible: bool) -> impl IntoElement {
    div()
        .id("overlay-top")
        .absolute()
        .top(px(12.0))
        .left(px(14.0))
        .flex()
        .gap(px(10.0))
        .opacity(if visible { 1.0 } else { 0.0 })
        .child(div().child(overlay.name.clone()))
        .child(div().child(overlay.position.clone()))
        .text_color(overlay.theme_text)
        .into_any_element()
}

/// Render the bottom overlay (zoom + prev/next).
///
/// `prev`/`next` are pre-built arrow elements (constructed with `cx.listener`
/// at the App::render call site — same pattern as Tasks 7/8, including the
/// mouse-down swallowing on the buttons).
pub fn bottom(
    overlay: &OverlayData,
    visible: bool,
    prev: Option<AnyElement>,
    next: Option<AnyElement>,
) -> impl IntoElement {
    let mut bar = div()
        .id("overlay-bottom")
        .absolute()
        .bottom(px(12.0))
        .left_0()
        .w_full()
        .flex()
        .justify_center()
        .gap(px(10.0))
        .opacity(if visible { 1.0 } else { 0.0 })
        .child(div().child(overlay.zoom_text.clone()));
    if let Some(p) = prev {
        bar = bar.child(p);
    }
    if let Some(n) = next {
        bar = bar.child(n);
    }
    bar.text_color(overlay.theme_text).into_any_element()
}

#[cfg(test)]
mod tests {
    // NOTE: explicit imports instead of `use super::*` — gpui's glob re-exports
    // the `test` proc macro, which blows the recursion limit under `use super::*`.
    use super::{OverlayData, OVERLAY_IDLE};
    use crate::app::parse_hex;

    #[test]
    fn overlay_data_from_theme_parses_colors() {
        let d = OverlayData::from_theme(
            "cat.png".into(),
            "1/3".into(),
            "100%".into(),
            "#e8e8ee",
            "#121218",
        );
        assert_eq!(d.name, "cat.png");
        assert_eq!(d.position, "1/3");
        assert_eq!(d.zoom_text, "100%");
        assert!(parse_hex("#e8e8ee").is_some());
        // Fallback colors for invalid input.
        let d2 = OverlayData::from_theme("a".into(), "b".into(), "c".into(), "zzz", "nope");
        // Fallback must still be a valid color (no panic).
        assert!((d2.theme_text.a - 1.0).abs() < 1e-5);
        assert!((d2.theme_surface.a - 1.0).abs() < 1e-5);
    }

    #[test]
    fn overlay_idle_constant_is_1500ms() {
        assert_eq!(OVERLAY_IDLE, std::time::Duration::from_millis(1500));
    }
}
