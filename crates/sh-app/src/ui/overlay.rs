//! Ephemeral overlay: bottom bar (zoom + preset chips + slideshow + prev/next).

use crate::app::parse_hex;
use gpui::prelude::*;
use gpui::*;

/// Idle time after which overlays fade out.
pub const OVERLAY_IDLE: std::time::Duration = std::time::Duration::from_millis(1500);

/// Fixed chrome budget for the bottom overlay: one row at the topbar scale.
/// The bar never grows with content — fixed height + clipped overflow cap
/// it even if children (zoom text, chips, arrows) measure taller.
pub const BOTTOM_BAR_H_PX: f32 = 40.0;

/// Private Styled extension: gate interactivity + paint via `display: none`.
///
/// gpui 0.2.2 has no fluent `.visibility()` (the `Style::visibility` field is
/// not refineable) and `.opacity(0.0)` is paint-only — invisible overlays
/// would remain clickable. `Display::None` skips child prepaint/paint
/// entirely (div.rs:1409/1454), so hidden overlays get no hitboxes. The
/// element stays in the tree either way, keeping IDs stable.
///
/// NOTE: a future fade animation can layer `.opacity()` on top of this gate;
/// opacity alone would leave hidden overlays interactive.
trait VisibilityGate: Styled + Sized {
    /// Hide (and de-activate) this element unless `visible`.
    fn visibility_gate(self, visible: bool) -> Self {
        if visible {
            self
        } else {
            self.hidden()
        }
    }
}

impl VisibilityGate for Div {}
impl VisibilityGate for Stateful<Div> {}

/// Data needed to render overlays.
#[derive(Debug, Clone)]
pub struct OverlayData {
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
    pub fn from_theme(zoom_text: String, text_hex: &str, surface_hex: &str) -> Self {
        Self {
            zoom_text,
            theme_text: parse_hex(text_hex).unwrap_or(rgb(0xe8e8ee).into()),
            theme_surface: parse_hex(surface_hex).unwrap_or(rgb(0x121218).into()),
        }
    }
}

// The old top overlay (floating name + position chip) was removed: the
// persistent topbar already shows that information, so the chip duplicated
// it while covering part of the image. The bottom overlay (zoom + presets +
// arrows) is the only ephemeral overlay left.

/// Colors for overlay action chrome, resolved from the active theme.
///
/// Pill chrome applies these only when the call site opts in
/// ([`ActionButtonOpts::chrome`]); bare call sites (prev/next/slideshow)
/// render without background, hover, radius, or text-color paint.
pub struct ActionButtonStyle {
    /// Overlay text color (content + chrome text).
    pub text: Hsla,
    /// Background of a chrome'd idle button.
    pub idle_bg: Hsla,
    /// Background of a chrome'd hovered button.
    pub hover_bg: Hsla,
    /// Background of a chrome'd active (pressed-look) button.
    pub active_bg: Hsla,
}

/// Per-call-site options for [`action_button`].
///
/// * `chrome: false` reproduces the bare overlay-arrow shape (cursor +
///   content only) so migrated controls stay pixel-identical by construction.
/// * `chrome: true` renders the topbar density-control pill idiom (bg,
///   hover tint, pressed tint when `active`, 6px radius, overlay text color)
///   — the zoom-preset chips opt into it.
pub struct ActionButtonOpts {
    /// Opt into the pill chrome (bg/hover/active/radius/text color).
    pub chrome: bool,
    /// Pressed-look tint (chrome'd buttons only).
    pub active: bool,
    /// Horizontal padding (chips use the density-control metrics; arrows 0).
    pub pad_x: f32,
    /// Vertical padding (chips use the density-control metrics; arrows 0).
    pub pad_y: f32,
}

/// Shared overlay action-button chrome.
///
/// Call sites attach `.id(...)` + mousedown-swallow + `on_click` (the
/// pre-built `cx.listener` elements pattern) and pass through their own
/// padding, so bare controls render pixel-identically and chips share the
/// exact same construction.
pub fn action_button(
    content: impl IntoElement,
    style: &ActionButtonStyle,
    opts: &ActionButtonOpts,
) -> Div {
    let div = div().cursor_pointer();
    if opts.chrome {
        div.bg(if opts.active {
            style.active_bg
        } else {
            style.idle_bg
        })
        .hover(move |s| s.bg(style.hover_bg))
        .text_color(style.text)
        .rounded(px(6.0))
        .px(px(opts.pad_x))
        .py(px(opts.pad_y))
        .child(content)
    } else {
        div.child(content)
    }
}

/// One info-popover row: localized label + untranslated value.
pub fn info_rows(
    lang: sh_core::i18n::Language,
    facts: &sh_core::decode::FileInfo,
) -> Vec<(String, String)> {
    use sh_core::i18n::StrKey;
    vec![
        (
            lang.get(StrKey::InfoDimensionsLabel).to_string(),
            format!("{} × {}", facts.width, facts.height),
        ),
        (
            lang.get(StrKey::InfoFileSizeLabel).to_string(),
            sh_core::decode::format_file_size(facts.size_bytes),
        ),
        (
            lang.get(StrKey::InfoFormatLabel).to_string(),
            facts.format.clone(),
        ),
    ]
}

/// Render the viewer info popover: three labeled rows from `facts`, or the
/// localized `error` string when facts failed. Caller passes
/// `visible = info_panel_open`; hidden renders `Display::None` (no hitboxes,
/// ADR-008 precedent). Numeric/unit tokens never pass through the string
/// table — labels resolve via `lang.get`, values come from core formatters.
///
/// The popover swallows its own mousedown (sort-menu precedent) so clicks
/// inside do not reach the outside-click catcher. It is a fixed top-right
/// dropdown card below the top chrome: bottom-anchoring proved unreliable
/// in this tree (the popover never painted there), while top-anchored
/// floats (info button, chips row) paint fine — so it anchors like they do.
pub fn info_popover(
    lang: sh_core::i18n::Language,
    facts: Option<&sh_core::decode::FileInfo>,
    error: &str,
    text: Hsla,
    surface: Hsla,
    visible: bool,
) -> AnyElement {
    let mut border = text;
    border.a = 0.22;
    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .bg(surface)
        .border(px(1.0))
        .border_color(border)
        .rounded(px(8.0))
        .px(px(12.0))
        .py(px(8.0))
        .min_w(px(260.0));
    match facts {
        Some(facts) => {
            for (label, value) in info_rows(lang, facts) {
                col = col.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(12.0))
                        .child(div().text_color(text).child(label))
                        .child(div().text_color(text).child(value)),
                );
            }
        }
        None => {
            col = col.child(div().text_color(text).child(error.to_string()));
        }
    }
    div()
        .id("info-popover")
        .absolute()
        // Fixed dropdown spot: below the 40px top chrome + 12px gap, right
        // aligned — identical in every Tab/idle state.
        .top(px(52.0))
        .right(px(12.0))
        .visibility_gate(visible)
        .child(col)
        .into_any_element()
}

/// Render the bottom overlay (zoom + preset chips + slideshow + prev/next).
///
/// One compact fixed-height row ([`BOTTOM_BAR_H_PX`], the topbar scale):
/// no wrap, tighter gaps/padding, clipped overflow — the bar never grows
/// with content and never stacks a second solid bar under the topbar (the
/// topbar dissolves while this bar is armed; see
/// `crate::app::topbar_dissolved_for_viewer`).
///
/// `chips`, `slideshow`, `prev`/`next` are pre-built elements (constructed
/// with `cx.listener` at the App::render call site — same pattern as Tasks
/// 7/8, including the mouse-down swallowing on the buttons). Same visibility
/// gate as [`top`]: hidden ⇒ `Display::None` ⇒ no hitboxes, chips included.
///
/// The info button is intentionally NOT a slot here (B3): it used to ride
/// this auto-hiding bar and lost its hitbox with Tab OFF or after idle.
/// It now floats above the viewer area; see `crate::app::info_button_visible`.
pub fn bottom(
    overlay: &OverlayData,
    visible: bool,
    chips: Vec<AnyElement>,
    slideshow: Option<AnyElement>,
    prev: Option<AnyElement>,
    next: Option<AnyElement>,
) -> impl IntoElement {
    let mut bar = div()
        .id("overlay-bottom")
        .debug_selector(|| "overlay-bottom".to_string())
        .absolute()
        .bottom(px(12.0))
        .left_0()
        .w_full()
        .h(px(BOTTOM_BAR_H_PX))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .overflow_hidden()
        .bg(overlay.theme_surface)
        .px(px(8.0))
        .py(px(4.0))
        .rounded(px(8.0))
        .visibility_gate(visible)
        .child(div().child(overlay.zoom_text.clone()));
    for chip in chips {
        bar = bar.child(chip);
    }
    if let Some(s) = slideshow {
        bar = bar.child(s);
    }
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
    use super::{info_popover, info_rows, OverlayData, OVERLAY_IDLE};
    use crate::app::parse_hex;

    #[test]
    fn overlay_data_from_theme_parses_colors() {
        let d = OverlayData::from_theme("100%".into(), "#e8e8ee", "#121218");
        assert_eq!(d.zoom_text, "100%");
        assert!(parse_hex("#e8e8ee").is_some());
        // Fallback colors for invalid input.
        let d2 = OverlayData::from_theme("c".into(), "zzz", "nope");
        // Fallback must still be a valid color (no panic).
        assert!((d2.theme_text.a - 1.0).abs() < 1e-5);
        assert!((d2.theme_surface.a - 1.0).abs() < 1e-5);
    }

    #[test]
    fn overlay_idle_constant_is_1500ms() {
        assert_eq!(OVERLAY_IDLE, std::time::Duration::from_millis(1500));
    }

    #[test]
    fn bottom_bar_height_matches_topbar_scale() {
        use super::BOTTOM_BAR_H_PX;
        // One-row chrome budget: the bottom bar caps at the topbar scale
        // so it can never grow into a second stacked bar.
        assert_eq!(BOTTOM_BAR_H_PX, crate::ui::topbar::TOPBAR_H_PX);
    }

    fn test_facts() -> sh_core::decode::FileInfo {
        sh_core::decode::FileInfo {
            width: 1920,
            height: 1080,
            size_bytes: 2_400_000,
            format: "PNG".into(),
        }
    }

    #[test]
    fn info_rows_render_three_labeled_facts_in_english() {
        let rows = info_rows(sh_core::i18n::Language::En, &test_facts());
        assert_eq!(rows.len(), 3, "exactly dimensions/size/format, no EXIF");
        assert_eq!(rows[0], ("Dimensions".into(), "1920 × 1080".into()));
        assert_eq!(rows[1], ("Size".into(), "2.4 MB".into()));
        assert_eq!(rows[2], ("Format".into(), "PNG".into()));
    }

    #[test]
    fn info_rows_render_spanish_labels_with_unchanged_values() {
        let rows = info_rows(sh_core::i18n::Language::Es, &test_facts());
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], ("Dimensiones".into(), "1920 × 1080".into()));
        assert_eq!(rows[1], ("Tamaño".into(), "2.4 MB".into()));
        assert_eq!(rows[2], ("Formato".into(), "PNG".into()));
    }

    #[test]
    fn info_popover_builds_for_facts_error_and_hidden_states() {
        // Element construction is pure (no window needed); this pins the
        // call contract — three labeled rows, the localized error path,
        // and the hidden gate — while dismissal/interaction is covered by
        // the headless App regression tests.
        let text = parse_hex("#e8e8ee").unwrap();
        let surface = parse_hex("#121218").unwrap();
        let lang = sh_core::i18n::Language::En;
        let facts = test_facts();
        let _ = info_popover(lang, Some(&facts), "", text, surface, true);
        let _ = info_popover(lang, None, "Could not read image info", text, surface, true);
        // Hidden ⇒ gated to Display::None (no hitboxes, ADR-008 precedent).
        let _ = info_popover(lang, Some(&facts), "", text, surface, false);
    }
}
