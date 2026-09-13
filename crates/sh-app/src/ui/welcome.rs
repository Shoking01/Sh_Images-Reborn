//! Welcome screen: startup landing with Continue / Open-folder affordances.

use crate::app::parse_hex;
use gpui::prelude::*;
use gpui::*;

/// Data needed to render the welcome screen.
#[derive(Debug, Clone)]
pub struct WelcomeData {
    /// Display text of the last folder, if one is still available.
    pub last_dir_text: Option<String>,
    /// Text color from the active theme.
    pub theme_text: Hsla,
    /// Surface color from the active theme (chip background).
    pub theme_surface: Hsla,
    /// Accent color from the active theme (title).
    pub theme_accent: Hsla,
}

impl WelcomeData {
    /// Build welcome data from theme color strings, with sensible fallbacks
    /// for missing/invalid entries.
    pub fn from_theme(
        last_dir: Option<String>,
        text_hex: &str,
        surface_hex: &str,
        accent_hex: &str,
    ) -> Self {
        Self {
            last_dir_text: last_dir,
            theme_text: parse_hex(text_hex).unwrap_or(rgb(0xe8e8ee).into()),
            theme_surface: parse_hex(surface_hex).unwrap_or(rgb(0x121218).into()),
            theme_accent: parse_hex(accent_hex).unwrap_or(rgb(0x00ffff).into()),
        }
    }
}

/// Secondary text color: theme text at 60% alpha (editorial hierarchy).
pub fn dimmed(text: Hsla) -> Hsla {
    Hsla { a: 0.6, ..text }
}

/// Render the welcome screen: editorial asymmetric layout.
/// Left: hero title + tagline + actions. Right: drop zone. The root
/// `on_drop` in app.rs already handles file/folder drops app-wide.
pub fn welcome(
    data: &WelcomeData,
    continue_btn: Option<AnyElement>,
    open_btn: AnyElement,
) -> impl IntoElement {
    let secondary = dimmed(data.theme_text);

    // ── Left column: hero + actions ──
    let last_dir_chip = data.last_dir_text.as_ref().map(|dir| {
        div()
            .child(format!("Last folder: {dir}"))
            .bg(data.theme_surface)
            .text_color(data.theme_text)
            .rounded(px(8.0))
            .px(px(10.0))
            .py(px(6.0))
    });

    let mut actions = div().flex().flex_col().gap(px(10.0)).mt(px(20.0));
    if let Some(btn) = continue_btn {
        actions = actions.child(btn);
    }
    actions = actions.child(open_btn);

    let mut left = div()
        .flex()
        .flex_col()
        .gap(px(14.0))
        .child(
            div()
                .child("SH_IMAGES")
                .text_size(px(28.0))
                .text_color(data.theme_text),
        )
        .child(
            div()
                .child("A native, GPU-accelerated image viewer")
                .text_size(px(13.0))
                .text_color(secondary),
        );
    if let Some(chip) = last_dir_chip {
        left = left.child(chip);
    }
    left = left.child(actions);

    // ── Right column: drop zone ──
    // Solid low-alpha border per plan (BorderStyle::Dashed exists in the
    // scene API, but 0.2.2's fluent builders expose no dashed helper;
    // the plan ships the solid border and notes dashed for V3 polish).
    let mut zone_border = data.theme_surface;
    zone_border.a = 0.4;

    let drop_zone = div()
        .id("welcome-dropzone")
        .flex_1()
        .h_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(10.0))
        .border(px(1.0))
        .border_color(zone_border)
        .rounded(px(12.0))
        .child(crate::ui::icons::icon(
            crate::ui::icons::IconName::Image,
            px(48.0),
            secondary,
        ))
        .child(
            div()
                .child("Drop images or a folder here")
                .text_size(px(12.0))
                .text_color(secondary),
        );

    div()
        .id("welcome")
        .size_full()
        .flex()
        .items_center()
        .gap(px(48.0))
        .p(px(48.0))
        .child(left)
        .child(drop_zone)
        .text_color(data.theme_text)
}

#[cfg(test)]
mod tests {
    // NOTE: explicit imports instead of `use super::*` — gpui's glob re-exports
    // the `test` proc macro, which blows the recursion limit under `use super::*`.
    use super::{dimmed, WelcomeData};
    use crate::app::parse_hex;

    #[test]
    fn dimmed_text_is_60_percent_alpha() {
        let t: gpui::Hsla = gpui::rgb(0xe8e8ee).into();
        let d = dimmed(t);
        assert!((d.a - 0.6).abs() < 1e-5);
        assert_eq!(d.h, t.h);
        assert_eq!(d.s, t.s);
        assert_eq!(d.l, t.l);
    }

    #[test]
    fn welcome_data_from_theme_parses_colors() {
        let d = WelcomeData::from_theme(Some("C:\\pics".into()), "#e8e8ee", "#121218", "#00ffff");
        assert_eq!(d.last_dir_text.as_deref(), Some("C:\\pics"));
        assert!(parse_hex("#e8e8ee").is_some());
        assert!(parse_hex("#00ffff").is_some());
        // Fallback colors for invalid input.
        let d2 = WelcomeData::from_theme(None, "zzz", "nope", "bad");
        assert_eq!(d2.last_dir_text, None);
        // Fallbacks must still be valid colors (no panic).
        assert!((d2.theme_text.a - 1.0).abs() < 1e-5);
        assert!((d2.theme_surface.a - 1.0).abs() < 1e-5);
        assert!((d2.theme_accent.a - 1.0).abs() < 1e-5);
    }
}
