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

/// Render the welcome screen. `continue_btn`/`open_btn` are pre-built
/// elements (same `cx.listener` call-site pattern as the overlay arrows).
pub fn welcome(
    data: &WelcomeData,
    continue_btn: Option<AnyElement>,
    open_btn: AnyElement,
) -> impl IntoElement {
    let chip = match &data.last_dir_text {
        Some(dir) => div()
            .child(format!("Last folder: {dir}"))
            .bg(data.theme_surface)
            .text_color(data.theme_text)
            .rounded(px(8.0))
            .px(px(10.0))
            .py(px(6.0)),
        None => div()
            .child("Open a folder to get started")
            .text_color(data.theme_text),
    };
    let mut row = div().flex().justify_center().gap(px(12.0)).mt(px(16.0));
    if let Some(btn) = continue_btn {
        row = row.child(btn);
    }
    row = row.child(open_btn);
    div()
        .id("welcome")
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(14.0))
                .child(div().child("SH_IMAGES").text_color(data.theme_accent))
                .child(chip)
                .child(row)
                .text_color(data.theme_text),
        )
}

#[cfg(test)]
mod tests {
    // NOTE: explicit imports instead of `use super::*` — gpui's glob re-exports
    // the `test` proc macro, which blows the recursion limit under `use super::*`.
    use super::WelcomeData;
    use crate::app::parse_hex;

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
