//! Settings content scroll geometry for every section.
//!
//! Scroll note: gpui 0.2.2 has a native `overflow_y_scroll`, but a scroll
//! container inside a flex layout needs `min-height: 0` to actually bound
//! itself (flex auto-minimums grow it to its content instead), and `Div`
//! exposes no `min-height` setter — so the native path can never engage
//! here (verified against gpui + taffy 0.9.0 sources; an `overflow_y_scroll`
//! attempt shipped dead scroll). This module follows the grid precedent
//! (`ui/grid.rs`): the render wraps content in a `relative` div shifted by
//! `-scroll_px` (driven by the root wheel handler in Settings view), clamped
//! by [`settings_max_scroll`]. Row heights below are EXPLICIT in render
//! (`.h()`, never content-sized) so the clamp arithmetic stays exact —
//! changing a row height without updating its constant strands rows out of
//! reach, exactly like `GRID_ROW_H_PX`.

use super::SettingsSection;

/// Gap between General section children — must match its render `gap()`.
pub const GENERAL_GAP_PX: f32 = 8.0;
/// Gap between Appearance section children — must match its render `gap()`.
pub const APPEARANCE_GAP_PX: f32 = 2.0;
/// Gap between rows inside the Shortcuts column — must match its `gap()`.
pub const SHORTCUT_GAP_PX: f32 = 2.0;

/// Content padding (each side) — must match the `p()` in the render.
pub const SETTINGS_PAD_PX: f32 = 16.0;
/// General/Appearance section header height — must match render `.h()`.
pub const SETTINGS_HEADER_H_PX: f32 = 32.0;
/// General/Appearance interactive row height — must match render `.h()`.
pub const SETTINGS_ROW_H_PX: f32 = 40.0;
/// Language options rendered in General — must match `language_options()`.
pub const LANGUAGE_OPTION_COUNT: usize = 2;

/// Reset-all button row height — must match its `.h()` in the render.
pub const SHORTCUT_RESET_H_PX: f32 = 40.0;
/// Context group header height — must match its `.h()` in the render.
pub const SHORTCUT_GROUP_H_PX: f32 = 32.0;
/// Shortcut row height — must match its `.h()` in the render. Rows stay
/// single-line; wrapping would silently break [`shortcuts_content_h`].
pub const SHORTCUT_ROW_H_PX: f32 = 40.0;
/// Shortcut actions rendered (reset button excluded) — must match the
/// `ACTIONS` table length consumed by the Shortcuts section.
pub const SHORTCUT_ROW_COUNT: usize = 18;
/// Context groups rendered — Viewer, Grid, Global, in that order.
pub const SHORTCUT_GROUP_COUNT: usize = 3;

fn column_content_h(header_count: usize, row_count: usize, gap: f32) -> f32 {
    let child_count = header_count + row_count;
    if child_count == 0 {
        return 0.0;
    }
    header_count as f32 * SETTINGS_HEADER_H_PX
        + row_count as f32 * SETTINGS_ROW_H_PX
        + (child_count - 1) as f32 * gap
}

/// Exact General content height for the current recent-folder count.
pub fn general_content_h(recent_count: usize) -> f32 {
    let clear_row_count = usize::from(recent_count > 0);
    let row_count = LANGUAGE_OPTION_COUNT + 1 + recent_count + clear_row_count;
    column_content_h(2, row_count, GENERAL_GAP_PX)
}

/// Exact Appearance content height for the current builtin-theme count.
pub fn appearance_content_h(theme_count: usize) -> f32 {
    let setting_row_count = theme_count + super::sections::appearance::APPEARANCE_SETTING_ROW_COUNT;
    column_content_h(1, setting_row_count, APPEARANCE_GAP_PX)
}

/// Exact content height for the active Settings section.
pub fn section_content_h(section: SettingsSection, recent_count: usize, theme_count: usize) -> f32 {
    match section {
        SettingsSection::General => general_content_h(recent_count),
        SettingsSection::Appearance => appearance_content_h(theme_count),
        SettingsSection::Shortcuts => shortcuts_content_h(),
    }
}

/// Inner content height of the Shortcuts section (no outer padding):
/// reset + group headers + rows + inter-child gaps. Exact by construction
/// (all heights explicit in render).
pub fn shortcuts_content_h() -> f32 {
    let rows = SHORTCUT_ROW_COUNT as f32;
    let groups = SHORTCUT_GROUP_COUNT as f32;
    let children = 1.0 + groups + rows;
    SHORTCUT_RESET_H_PX
        + groups * SHORTCUT_GROUP_H_PX
        + rows * SHORTCUT_ROW_H_PX
        + (children - 1.0).max(0.0) * SHORTCUT_GAP_PX
}

/// Max scroll offset: content height minus visible height, floored at zero
/// (nothing to scroll). Same shape as [`crate::ui::grid::grid_max_scroll`].
pub fn settings_max_scroll(content_h: f32, visible_h: f32) -> f32 {
    (content_h - visible_h).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::settings_panel::SettingsSection;

    #[test]
    fn shortcuts_content_height_is_exact() {
        // 40 + 3*32 + 18*40 + 21*2 = 40 + 96 + 720 + 42 = 898.
        assert_eq!(shortcuts_content_h(), 898.0);
    }

    #[test]
    fn settings_appearance_content_height_includes_every_row() {
        // Header + three themes + filmstrip + checkerboard + interval + reduced motion.
        assert_eq!(appearance_content_h(3), 326.0);
    }

    #[test]
    fn settings_appearance_rows_remain_reachable_at_minimum_window() {
        let content_h = appearance_content_h(crate::theme_builtins::BUILTIN_THEMES.len());
        let visible_h = 320.0 - crate::ui::topbar::TOPBAR_H_PX - 2.0 * SETTINGS_PAD_PX;
        let max_scroll = settings_max_scroll(content_h, visible_h);

        assert!(content_h > visible_h);
        assert_eq!(max_scroll, content_h - visible_h);
    }

    #[test]
    fn settings_section_geometry_dispatches_to_the_active_column() {
        assert_eq!(
            section_content_h(SettingsSection::General, 0, 3),
            general_content_h(0)
        );
        assert_eq!(
            section_content_h(SettingsSection::Appearance, 0, 3),
            appearance_content_h(3)
        );
        assert_eq!(
            section_content_h(SettingsSection::Shortcuts, 0, 3),
            shortcuts_content_h()
        );
    }

    #[test]
    fn max_scroll_is_content_minus_visible_floored() {
        assert_eq!(settings_max_scroll(898.0, 1200.0), 0.0);
        assert_eq!(settings_max_scroll(898.0, 498.0), 400.0);
        assert_eq!(settings_max_scroll(0.0, 0.0), 0.0);
    }
}
