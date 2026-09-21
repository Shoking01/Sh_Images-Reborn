//! Viewer filmstrip: bottom-docked thumbnail navigator (Slice A: constants).
//!
//! The strip is layout chrome, never overlay chrome: a fixed-height
//! in-flow row below the image area showing a windowed range of cached
//! thumbnails with click-to-navigate. Slice A establishes only the height
//! constant consumed by the viewport carve
//! ([`crate::app::stable_filmstrip_viewport`]); the window helper, cell
//! builder, and click wiring land in Slice B.

/// Fixed strip height in px: layout chrome, never content-sized.
///
/// Subtracted by [`crate::app::stable_filmstrip_viewport`] and enforced in
/// layout by `#filmstrip.h(px(STRIP_H_PX))`. Same discipline as
/// `TOPBAR_H_PX` / `BOTTOM_BAR_H_PX`: fixed constants keep carve
/// arithmetic exact instead of depending on measured content height.
pub const STRIP_H_PX: f32 = 104.0;
