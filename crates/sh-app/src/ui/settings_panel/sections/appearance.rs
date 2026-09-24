//! Appearance section: theme picker (moved here from the old topbar
//! dropdown) and persisted viewer display controls.

use sh_core::settings::{SLIDESHOW_INTERVAL_MAX_SECS, SLIDESHOW_INTERVAL_MIN_SECS};

/// Stable element ID for the filmstrip toggle row.
pub const FILMSTRIP_TOGGLE_ID: &str = "settings-filmstrip-toggle";
/// Stable element ID for the transparency checkerboard toggle row.
pub const CHECKERBOARD_TOGGLE_ID: &str = "settings-checkerboard-toggle";
/// Stable element ID for the reduced-motion toggle row.
pub const REDUCE_MOTION_TOGGLE_ID: &str = "settings-reduce-motion-toggle";
/// Stable element ID for the slideshow interval row.
pub const SLIDESHOW_INTERVAL_ROW_ID: &str = "settings-slideshow-interval";
/// Stable element ID for the interval decrement button.
pub const SLIDESHOW_INTERVAL_DECREMENT_ID: &str = "settings-slideshow-interval-decrement";
/// Stable element ID for the persisted interval value.
pub const SLIDESHOW_INTERVAL_VALUE_ID: &str = "settings-slideshow-interval-value";
/// Stable element ID for the interval increment button.
pub const SLIDESHOW_INTERVAL_INCREMENT_ID: &str = "settings-slideshow-interval-increment";
/// Number of persisted controls below the theme picker.
pub const APPEARANCE_SETTING_ROW_COUNT: usize = 4;

/// Adjust a slideshow interval while keeping the persisted inclusive bounds.
pub fn adjust_slideshow_interval(current: u32, delta: i32) -> u32 {
    let adjusted = if delta < 0 {
        current.saturating_sub(delta.unsigned_abs())
    } else {
        current.saturating_add(delta as u32)
    };
    adjusted.clamp(SLIDESHOW_INTERVAL_MIN_SECS, SLIDESHOW_INTERVAL_MAX_SECS)
}

/// Display name for a builtin theme file: parsed theme name, file fallback.
pub fn theme_display_name(file: &str, json: &str) -> String {
    sh_core::theme::parse(json)
        .map(|t| t.name)
        .unwrap_or_else(|_| file.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_display_prefers_parsed_name() {
        let json = crate::theme_builtins::builtin_theme_json("dark-clinical.json");
        assert_eq!(
            theme_display_name("dark-clinical.json", json),
            "Dark Clinical"
        );
        assert_eq!(theme_display_name("x.json", "{ broken"), "x.json");
    }

    #[test]
    fn settings_rows_use_stable_unique_element_ids() {
        let ids = [
            FILMSTRIP_TOGGLE_ID,
            CHECKERBOARD_TOGGLE_ID,
            REDUCE_MOTION_TOGGLE_ID,
            SLIDESHOW_INTERVAL_ROW_ID,
            SLIDESHOW_INTERVAL_DECREMENT_ID,
            SLIDESHOW_INTERVAL_VALUE_ID,
            SLIDESHOW_INTERVAL_INCREMENT_ID,
        ];

        for (index, id) in ids.iter().enumerate() {
            assert!(id.starts_with("settings-"));
            assert!(!ids[index + 1..].contains(id), "duplicate element id: {id}");
        }
    }

    #[test]
    fn settings_slideshow_interval_adjustment_stays_within_bounds() {
        assert_eq!(adjust_slideshow_interval(3, -1), 2);
        assert_eq!(adjust_slideshow_interval(3, 1), 4);
        assert_eq!(adjust_slideshow_interval(1, -1), 1);
        assert_eq!(adjust_slideshow_interval(60, 1), 60);
        assert_eq!(adjust_slideshow_interval(30, i32::MIN), 1);
        assert_eq!(adjust_slideshow_interval(30, i32::MAX), 60);
    }

    #[test]
    fn reduce_motion_row_has_a_stable_id_and_counts_as_a_setting_row() {
        assert_eq!(REDUCE_MOTION_TOGGLE_ID, "settings-reduce-motion-toggle");
        assert_eq!(APPEARANCE_SETTING_ROW_COUNT, 4);
    }
}
