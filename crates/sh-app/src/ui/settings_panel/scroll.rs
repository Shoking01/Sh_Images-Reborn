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
/// Gap between Theme Editor children — must match its render `gap()`.
pub const THEME_EDITOR_GAP_PX: f32 = 2.0;

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

/// Section headers in the Theme Editor column — just the one column header.
///
/// Every row below it carries its own label, so per-group headers would add
/// three more fixed-height children to assert for nothing.
pub const THEME_EDITOR_HEADER_COUNT: usize = 1;

/// Interactive rows in the Theme Editor column: the ten color slots plus the
/// two structural fields [`sh_core::theme::validate`] rejects when empty.
///
/// Derived from the rendered [`FIELDS`] list rather than written as `12`, so a
/// row added there without a matching constant — or a color added to
/// `ThemeColors` without a row — fails the arithmetic loudly instead of
/// stranding the row past the scroll clamp.
pub const THEME_EDITOR_ROW_COUNT: usize = super::sections::theme_editor::FIELDS.len();

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

/// Exact Appearance content height for the current theme-row count.
///
/// `theme_count` is `App::theme_entries.len()` — built-ins PLUS every user
/// theme discovered in the config `themes/` directory, so it is bounded
/// only by how many files the user has dropped there. That is fine, and
/// deliberately not clamped or capped:
///
/// * Clamping the HEIGHT would strand rows: the clamp is what decides
///   whether the last theme is reachable, and a height that stops growing
///   while rows keep rendering is precisely the "changing a row height
///   without updating its constant" failure this module's header warns
///   about, just in the other direction.
/// * Capping the LIST would need a "more" affordance, which needs a new
///   i18n string, a new interactive element, and a new focus target in
///   [`super::AppearanceControl`] — all to bound a list that is a handful
///   of ~500-byte JSON files in normal use.
///
/// The arithmetic is already parameterized by the count, and both the
/// render and the wheel handler's clamp read the same `theme_entries.len()`,
/// so a tall list scrolls correctly for ANY count. Pinned by
/// `appearance_geometry_stays_exact_for_an_unbounded_theme_count`.
pub fn appearance_content_h(theme_count: usize) -> f32 {
    let setting_row_count = theme_count + super::sections::appearance::APPEARANCE_SETTING_ROW_COUNT;
    column_content_h(1, setting_row_count, APPEARANCE_GAP_PX)
}

/// Exact content height for the active Settings section.
pub fn section_content_h(section: SettingsSection, recent_count: usize, theme_count: usize) -> f32 {
    match section {
        SettingsSection::General => general_content_h(recent_count),
        SettingsSection::Appearance => appearance_content_h(theme_count),
        SettingsSection::ThemeEditor => theme_editor_content_h(),
        SettingsSection::Shortcuts => shortcuts_content_h(),
    }
}

/// Exact Theme Editor content height (no outer padding).
///
/// Unlike the other sections this one takes no count: every row is a fixed
/// member of the schema, so the height is a compile-time constant rather than
/// a function of anything the user did. That is what makes it the one section
/// whose arithmetic can be pinned against the render exactly — see
/// `theme_editor_content_height_matches_the_rendered_column` in `app.rs`,
/// which is the assertion this constant exists to satisfy.
pub fn theme_editor_content_h() -> f32 {
    column_content_h(
        THEME_EDITOR_HEADER_COUNT,
        THEME_EDITOR_ROW_COUNT,
        THEME_EDITOR_GAP_PX,
    )
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

    /// The theme count is user-unbounded (one row per file in the config
    /// themes directory), so the height must keep tracking it exactly
    /// instead of saturating. Proves both halves of the contract: every
    /// added row is 40px + a 2px gap, and the scroll clamp stays
    /// `content - visible` so the LAST row is reachable at any count.
    #[test]
    fn appearance_geometry_stays_exact_for_an_unbounded_theme_count() {
        let visible_h = 320.0 - crate::ui::topbar::TOPBAR_H_PX - 2.0 * SETTINGS_PAD_PX;
        let base = appearance_content_h(crate::theme_builtins::BUILTIN_THEMES.len());

        for extra in [0usize, 1, 12, 500, 5_000] {
            let count = crate::theme_builtins::BUILTIN_THEMES.len() + extra;
            let content_h = appearance_content_h(count);
            assert!(
                (content_h - base - extra as f32 * (SETTINGS_ROW_H_PX + APPEARANCE_GAP_PX)).abs()
                    < 0.001,
                "{count} themes broke the per-row geometry: {content_h} vs {base}"
            );
            // Scrollable, and the clamp reaches the very bottom of the
            // content — that is what keeps the last user theme clickable.
            assert!(
                content_h > visible_h,
                "{count} themes fit without scrolling"
            );
            assert_eq!(
                settings_max_scroll(content_h, visible_h),
                content_h - visible_h
            );
            // Bottom of the scroll window lines up with the bottom of the
            // content: the last row cannot be stranded past the clamp.
            assert!(
                (content_h - settings_max_scroll(content_h, visible_h) - visible_h).abs() < 0.001
            );
        }
    }

    #[test]
    fn settings_appearance_rows_remain_reachable_at_minimum_window() {
        // The built-ins alone are the minimum viable list (what renders
        // before the first discovery completes), so this is the floor.
        let content_h = appearance_content_h(crate::theme_builtins::BUILTIN_THEMES.len());
        let visible_h = 320.0 - crate::ui::topbar::TOPBAR_H_PX - 2.0 * SETTINGS_PAD_PX;
        let max_scroll = settings_max_scroll(content_h, visible_h);

        assert!(content_h > visible_h);
        assert_eq!(max_scroll, content_h - visible_h);
    }

    #[test]
    fn theme_editor_content_height_is_exact() {
        // 1 header + 12 rows + 12 gaps between 13 children:
        //   1 * 32          =  32   (SETTINGS_HEADER_H_PX)
        // + 12 * 40         = 480   (SETTINGS_ROW_H_PX: ten slots + name + family)
        // + 12 * 2          =  24   (THEME_EDITOR_GAP_PX between all 13 children)
        // = 536
        assert_eq!(theme_editor_content_h(), 536.0);
    }

    /// The row count is derived from the schema, not written down. If a color
    /// is added to `ThemeColors` without a row here, this fails with the
    /// arithmetic rather than stranding the row below the scroll clamp.
    #[test]
    fn theme_editor_row_count_follows_the_field_schema() {
        assert_eq!(THEME_EDITOR_ROW_COUNT, 12);
        assert_eq!(
            THEME_EDITOR_ROW_COUNT,
            sh_core::theme_draft::SLOTS.len() + 2
        );
        // 12 rows must still exceed the visible area at the minimum window, or
        // the section would fit without ever scrolling — which would make the
        // clamp arithmetic vacuous.
        let visible_h = 320.0 - crate::ui::topbar::TOPBAR_H_PX - 2.0 * SETTINGS_PAD_PX;
        let content_h = theme_editor_content_h();
        assert!(
            content_h > visible_h,
            "the Theme Editor must scroll at the minimum window: {content_h} vs {visible_h}"
        );
        assert_eq!(
            settings_max_scroll(content_h, visible_h),
            content_h - visible_h
        );
        // Bottom of the scroll window lands exactly on the bottom of content.
        assert!((content_h - settings_max_scroll(content_h, visible_h) - visible_h).abs() < 0.001);
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
            section_content_h(SettingsSection::ThemeEditor, 0, 3),
            theme_editor_content_h()
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
