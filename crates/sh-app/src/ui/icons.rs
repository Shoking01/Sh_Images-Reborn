//! Icon registry: stable asset keys + a builder for themed SVG icons.
//!
//! Icons recolor via `.text_color()` because GPUI paints svg strokes with
//! the element's text color (`Svg::paint` uses `style.text.color`).

use gpui::{svg, Hsla, Pixels, Styled, Svg};

/// Every icon the app renders. `asset_path()` strings MUST match the keys
/// in `crate::assets::ICONS` exactly — locked by the tests below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconName {
    Gear,
    Scissors,
    ChevronLeft,
    ChevronRight,
    BackArrow,
    Expand,
    FolderOpen,
    Image,
    Eye,
    Close,
}

impl IconName {
    /// Asset path passed to `svg().path(...)`.
    pub fn asset_path(self) -> &'static str {
        match self {
            IconName::Gear => "icons/gear.svg",
            IconName::Scissors => "icons/scissors.svg",
            IconName::ChevronLeft => "icons/chevron-left.svg",
            IconName::ChevronRight => "icons/chevron-right.svg",
            IconName::BackArrow => "icons/back-arrow.svg",
            IconName::Expand => "icons/expand.svg",
            IconName::FolderOpen => "icons/folder-open.svg",
            IconName::Image => "icons/image.svg",
            IconName::Eye => "icons/eye.svg",
            IconName::Close => "icons/close.svg",
        }
    }
}

/// All icons — registry-completeness tests iterate this.
pub const ALL: [IconName; 10] = [
    IconName::Gear,
    IconName::Scissors,
    IconName::ChevronLeft,
    IconName::ChevronRight,
    IconName::BackArrow,
    IconName::Expand,
    IconName::FolderOpen,
    IconName::Image,
    IconName::Eye,
    IconName::Close,
];

/// Build a themed icon element. Size is square; color comes from the active
/// theme so all four built-in themes work without per-color variants.
pub fn icon(name: IconName, size: Pixels, color: Hsla) -> Svg {
    svg().size(size).path(name.asset_path()).text_color(color)
}

#[cfg(test)]
mod tests {
    // NOTE: explicit imports — gpui's glob re-exports the `test` proc macro
    // (recursion-limit hazard under `use super::*`, same as sibling modules).
    use super::ALL;
    use crate::assets::{AppAssets, ICONS};
    use gpui::AssetSource;

    #[test]
    fn every_icon_maps_to_a_registered_asset() {
        for name in ALL {
            let path = name.asset_path();
            let loaded = AppAssets.load(path).unwrap();
            assert!(loaded.is_some(), "{path} must be a registered asset");
        }
    }

    #[test]
    fn asset_paths_are_unique() {
        let mut paths: Vec<&str> = ALL.iter().map(|i| i.asset_path()).collect();
        paths.sort();
        paths.dedup();
        assert_eq!(paths.len(), ICONS.len());
    }
}
