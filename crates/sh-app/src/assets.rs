//! Embedded asset source: serves icon SVGs to GPUI's `svg()` element.
//!
//! License: icon path data derived from Lucide (ISC License,
//! https://lucide.dev). GPUI's `svg().path()` takes an ASSET PATH resolved
//! by `AssetSource` — not raw path data — so icons are embedded as bytes.

use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

/// Asset key → embedded SVG bytes. Keys are exact `svg().path()` strings.
/// `pub(crate)`: the icon-registry tests cross-check against this table.
pub(crate) static ICONS: &[(&str, &[u8])] = &[
    (
        "icons/gear.svg",
        include_bytes!("../assets/icons/gear.svg") as &[u8],
    ),
    (
        "icons/scissors.svg",
        include_bytes!("../assets/icons/scissors.svg") as &[u8],
    ),
    (
        "icons/chevron-left.svg",
        include_bytes!("../assets/icons/chevron-left.svg") as &[u8],
    ),
    (
        "icons/chevron-right.svg",
        include_bytes!("../assets/icons/chevron-right.svg") as &[u8],
    ),
    (
        "icons/back-arrow.svg",
        include_bytes!("../assets/icons/back-arrow.svg") as &[u8],
    ),
    (
        "icons/expand.svg",
        include_bytes!("../assets/icons/expand.svg") as &[u8],
    ),
    (
        "icons/folder-open.svg",
        include_bytes!("../assets/icons/folder-open.svg") as &[u8],
    ),
    (
        "icons/image.svg",
        include_bytes!("../assets/icons/image.svg") as &[u8],
    ),
    (
        "icons/eye.svg",
        include_bytes!("../assets/icons/eye.svg") as &[u8],
    ),
    (
        "icons/close.svg",
        include_bytes!("../assets/icons/close.svg") as &[u8],
    ),
    (
        "icons/play.svg",
        include_bytes!("../assets/icons/play.svg") as &[u8],
    ),
    (
        "icons/pause.svg",
        include_bytes!("../assets/icons/pause.svg") as &[u8],
    ),
];

/// The app's asset source, registered once at boot.
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(key, _)| *key == path)
            .map(|(_, bytes)| Cow::Borrowed(bytes as &'static [u8])))
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS.iter().map(|(key, _)| (*key).into()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::{AppAssets, ICONS};
    use gpui::AssetSource;

    #[test]
    fn every_icon_key_loads_bytes() {
        for (key, bytes) in ICONS {
            let loaded = AppAssets.load(key).expect("load never errors");
            let data = loaded.expect("every key must resolve");
            assert!(!data.is_empty(), "{key} must not be empty");
            assert_eq!(data.len(), bytes.len());
        }
    }

    #[test]
    fn unknown_path_returns_none() {
        let loaded = AppAssets.load("icons/does-not-exist.svg").unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn icon_count_is_twelve() {
        // 10 originals + Play/Pause (V3 slideshow). The companion test
        // `every_icon_maps_to_a_registered_asset` in icons.rs keeps the
        // registry locked to the enum — this one only pins the count.
        assert_eq!(ICONS.len(), 12);
    }
}
