//! Pure zoom/pan/fit math. No GPU types — the UI maps this to pixel layout.

/// 2D vector in abstract (image) units, `f32` for precision.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec2 {
    /// Horizontal component.
    pub x: f32,
    /// Vertical component.
    pub y: f32,
}

/// Zoom/pan state for one image.
///
/// Invariant: `scale` is always positive; [`fit`], [`zoom_at`], and
/// [`clamp_scale`] never produce a non-positive scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoomState {
    /// Scale factor relative to the image's natural size (1.0 = 100%).
    pub scale: f32,
    /// Offset in *scaled image units* from the top-left of the viewport.
    pub offset: Vec2,
}

/// Maximum zoom allowed (8x).
pub const MAX_SCALE: f32 = 8.0;

/// Scale used for "fit to window".
///
/// Returns the largest scale that shows the whole image inside the viewport,
/// clamped to a 0.01 floor. Degenerate (non-positive) inputs return 1.0.
pub fn fit_scale(image_w: f32, image_h: f32, viewport_w: f32, viewport_h: f32) -> f32 {
    if image_w <= 0.0 || image_h <= 0.0 || viewport_w <= 0.0 || viewport_h <= 0.0 {
        return 1.0;
    }
    (viewport_w / image_w).min(viewport_h / image_h).max(0.01)
}

/// Fit the image centered in the viewport. Returns a state at scale `fit`.
pub fn fit_to_window(image_size: Vec2, viewport: Vec2, fit: f32) -> ZoomState {
    ZoomState {
        scale: fit,
        offset: Vec2 {
            x: (viewport.x - image_size.x * fit) / 2.0,
            y: (viewport.y - image_size.y * fit) / 2.0,
        },
    }
}

/// Single-frame helper: fit with the computed natural fit scale.
pub fn fit(image_size: Vec2, viewport: Vec2) -> ZoomState {
    fit_to_window(
        image_size,
        viewport,
        fit_scale(image_size.x, image_size.y, viewport.x, viewport.y),
    )
}

/// Anchor zoom at a cursor position in viewport units (pixels).
///
/// The image point under the cursor stays fixed while the scale multiplies by
/// `delta`. The new scale is clamped to `[0.01, MAX_SCALE]`; when the clamp
/// engages the anchor drifts slightly — [`clamp_scale`] is the API that
/// re-centers instead.
pub fn zoom_at(state: ZoomState, cursor: Vec2, delta: f32) -> ZoomState {
    let new_scale = (state.scale * delta).clamp(0.01, MAX_SCALE);
    // The image point under the cursor must stay fixed:
    // img_point = (cursor - offset) / old_scale
    // offset' = cursor - img_point * new_scale
    let img_x = (cursor.x - state.offset.x) / state.scale;
    let img_y = (cursor.y - state.offset.y) / state.scale;
    ZoomState {
        scale: new_scale,
        offset: Vec2 {
            x: cursor.x - img_x * new_scale,
            y: cursor.y - img_y * new_scale,
        },
    }
}

/// Pan by a delta in viewport units.
pub fn pan(state: ZoomState, delta: Vec2) -> ZoomState {
    ZoomState {
        offset: Vec2 {
            x: state.offset.x + delta.x,
            y: state.offset.y + delta.y,
        },
        ..state
    }
}

/// Clamp scale into the allowed range preserving anchor by re-centering.
///
/// The floor is the image's current fit scale; when a clamp applies, the state
/// is re-centered on the viewport center at the clamped scale.
pub fn clamp_scale(state: ZoomState, image_size: Vec2, viewport: Vec2) -> ZoomState {
    let min = fit_scale(image_size.x, image_size.y, viewport.x, viewport.y);
    let s = state.scale.clamp(min, MAX_SCALE);
    if (s - state.scale).abs() < f32::EPSILON {
        state
    } else {
        // Re-center on the viewport center when the scale is clamped.
        fit_to_window(image_size, viewport, s)
    }
}

/// The visible image bounds inside the viewport (left, top, right, bottom), in viewport units.
pub fn visible_bounds(state: ZoomState, image_size: Vec2) -> (f32, f32, f32, f32) {
    let w = image_size.x * state.scale;
    let h = image_size.y * state.scale;
    (
        state.offset.x,
        state.offset.y,
        state.offset.x + w,
        state.offset.y + h,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::strategy::Strategy;

    const IMG: Vec2 = Vec2 {
        x: 1000.0,
        y: 500.0,
    };
    const VIEW: Vec2 = Vec2 { x: 800.0, y: 600.0 };

    #[test]
    fn fit_scales_to_contain() {
        let f = fit_scale(IMG.x, IMG.y, VIEW.x, VIEW.y);
        assert!((f - 0.8).abs() < 1e-5); // 800/1000
        let st = fit(IMG, VIEW);
        assert!((st.offset.x - 0.0).abs() < 1e-4);
        assert!((st.offset.y - (600.0 - 500.0 * 0.8) / 2.0).abs() < 1e-3);
    }

    #[test]
    fn zoom_anchors_cursor() {
        let st = fit(IMG, VIEW);
        // cursor at image center on screen
        let cursor = Vec2 { x: 400.0, y: 300.0 };
        let z = zoom_at(st, cursor, 2.0);
        // The image point under the cursor before = after:
        let img_before_x = (cursor.x - st.offset.x) / st.scale;
        let img_after_x = (cursor.x - z.offset.x) / z.scale;
        assert!((img_before_x - img_after_x).abs() < 1e-2);
        assert!((z.scale - st.scale * 2.0).abs() < 1e-5);
    }

    #[test]
    fn zoom_clamped_to_max() {
        let st = fit(IMG, VIEW);
        let z = zoom_at(st, Vec2 { x: 400.0, y: 300.0 }, 100.0);
        assert!(z.scale <= MAX_SCALE + 1e-5);
    }

    #[test]
    fn pan_moves_offset() {
        let st = fit(IMG, VIEW);
        let p = pan(st, Vec2 { x: 10.0, y: -5.0 });
        assert!((p.offset.x - st.offset.x - 10.0).abs() < 1e-5);
        assert!((p.offset.y - st.offset.y + 5.0).abs() < 1e-5);
    }

    #[test]
    fn clamp_scale_stays_within_bounds() {
        let big = ZoomState {
            scale: 1000.0,
            offset: Vec2 { x: 0.0, y: 0.0 },
        };
        let c = clamp_scale(big, IMG, VIEW);
        assert!(c.scale <= MAX_SCALE);
        let small = ZoomState {
            scale: 0.0,
            offset: Vec2 { x: 0.0, y: 0.0 },
        };
        let c2 = clamp_scale(small, IMG, VIEW);
        assert!(c2.scale >= fit_scale(IMG.x, IMG.y, VIEW.x, VIEW.y));
    }

    #[test]
    fn visible_bounds_are_positive() {
        let st = fit(IMG, VIEW);
        let (l, t, r, b) = visible_bounds(st, IMG);
        assert!(r > l);
        assert!(b > t);
    }

    proptest::proptest! {
        #[test]
        fn zoom_keeps_cursor_point_fixed(
            (sx, mult) in (0.1f32..8.0, 0.2f32..5.0).prop_filter(
                "zoom product stays within [0.01, MAX_SCALE] so no clamp shifts the anchor",
                |(s, m)| *s * *m <= MAX_SCALE,
            ),
            ox in -1000.0f32..1000.0,
            oy in -1000.0f32..1000.0,
            cx in 0.0f32..800.0,
            cy in 0.0f32..600.0,
        ) {
            let st = ZoomState { scale: sx, offset: Vec2 { x: ox, y: oy } };
            let z = zoom_at(st, Vec2 { x: cx, y: cy }, mult);
            let before_x = (cx - st.offset.x) / st.scale;
            let after_x = (cx - z.offset.x) / z.scale;
            let before_y = (cy - st.offset.y) / st.scale;
            let after_y = (cy - z.offset.y) / z.scale;
            assert!((before_x - after_x).abs() < 0.05);
            assert!((before_y - after_y).abs() < 0.05);
        }
    }
}
