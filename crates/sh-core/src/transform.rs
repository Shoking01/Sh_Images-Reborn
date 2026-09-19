//! Pure zoom/pan/fit math. No GPU types — the UI maps this to pixel layout.

/// 2D vector in abstract (image) units, `f32` for precision.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
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
#[derive(Debug, Clone, Copy, Default, PartialEq)]
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
/// clamped to `[0.01, MAX_SCALE]`. Degenerate (non-positive) inputs return 1.0.
pub fn fit_scale(image_w: f32, image_h: f32, viewport_w: f32, viewport_h: f32) -> f32 {
    if image_w <= 0.0 || image_h <= 0.0 || viewport_w <= 0.0 || viewport_h <= 0.0 {
        return 1.0;
    }
    (viewport_w / image_w)
        .min(viewport_h / image_h)
        .clamp(0.01, MAX_SCALE)
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
/// The image point under the cursor stays fixed exactly, even when the scale
/// clamps. The scale multiplies by `delta`, clamped to `[0.01, MAX_SCALE]`; the
/// 0.01 floor deliberately permits zooming below fit during a gesture —
/// [`clamp_scale`] re-asserts fit as the minimum when called.
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

/// Clamp scale into the allowed range, re-asserting fit as the minimum scale
/// and re-centering when the scale was clamped.
///
/// The floor is the image's current fit scale, which [`fit_scale`] caps at
/// [`MAX_SCALE`] so the clamp range is always valid. When the scale was
/// clamped, the state is re-centered on the viewport center at that scale.
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
    fn fit_scale_guards_non_positive_inputs() {
        assert_eq!(fit_scale(0.0, 100.0, 800.0, 600.0), 1.0);
        assert_eq!(fit_scale(100.0, 0.0, 800.0, 600.0), 1.0);
        assert_eq!(fit_scale(100.0, 100.0, 0.0, 600.0), 1.0);
        assert_eq!(fit_scale(100.0, 100.0, 800.0, 0.0), 1.0);
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
    fn zoom_at_floors_at_minimum() {
        let st = ZoomState {
            scale: 2.0,
            offset: Vec2 { x: 0.0, y: 0.0 },
        };
        // 2.0 * 0.0001 -> clamps to the 0.01 floor.
        let z = zoom_at(st, Vec2 { x: 400.0, y: 300.0 }, 0.0001);
        assert_eq!(z.scale, 0.01);
        assert!(z.scale > 0.0);
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
    fn clamp_scale_small_image_in_large_viewport_does_not_panic() {
        // 100x100 image in 1920x1080: fit was 10.8 > MAX_SCALE before the fix,
        // which made clamp_scale's `clamp(min, MAX_SCALE)` panic (min > max).
        let st = fit(
            Vec2 { x: 100.0, y: 100.0 },
            Vec2 {
                x: 1920.0,
                y: 1080.0,
            },
        );
        let c = clamp_scale(
            st,
            Vec2 { x: 100.0, y: 100.0 },
            Vec2 {
                x: 1920.0,
                y: 1080.0,
            },
        );
        assert!(c.scale <= MAX_SCALE);
        assert!(c.scale > 0.0);
        // Zooming in must never shrink the image below the pre-zoom scale.
        let z = zoom_at(st, Vec2 { x: 960.0, y: 540.0 }, 1.1);
        assert!(z.scale >= st.scale);
    }

    #[test]
    fn clamp_scale_noop_when_in_range() {
        let st = ZoomState {
            scale: 2.0,
            offset: Vec2 { x: 10.0, y: -5.0 },
        };
        let c = clamp_scale(st, IMG, VIEW);
        assert_eq!(c.scale, 2.0);
        assert_eq!(c.offset.x, 10.0);
        assert_eq!(c.offset.y, -5.0);
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
            sx in 0.1f32..8.0,
            ox in -1000.0f32..1000.0,
            oy in -1000.0f32..1000.0,
            cx in 0.0f32..800.0,
            cy in 0.0f32..600.0,
            mult in 0.2f32..5.0,
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

    // ── Zoom-preset clamp contract (viewer-zoom-presets, Phase 1) ──
    //
    // These tests pin the CONTRACT every preset activation relies on, not
    // new behavior: preset scales (0.5 / 1.0 / 2.0) routed through
    // `zoom_at` → `clamp_scale` keep their scale when above the fit floor,
    // snap back to fit at/below it, and stay inside `MAX_SCALE`. The
    // `Session::set_zoom_preset` funnel (sh-app) depends on each of these
    // invariants; a change here that breaks them must break these tests
    // first.

    /// Drive one preset-scale request through the same funnel the session
    /// helper uses: `zoom_at` with a multiplicative delta at the viewport
    /// center, then `clamp_scale`. Mirrors `Session::set_zoom_preset`'s
    /// scale path so the contract is pinned at the pure-math layer.
    fn preset_scale_through_clamp(
        start: ZoomState,
        target: f32,
        image_size: Vec2,
        viewport: Vec2,
    ) -> ZoomState {
        // Seed a degenerate (zero/negative) start scale via fit, exactly like
        // the session helper's fresh-session guard.
        let start = if start.scale <= 0.0 {
            fit(image_size, viewport)
        } else {
            start
        };
        let center = Vec2 {
            x: viewport.x / 2.0,
            y: viewport.y / 2.0,
        };
        let delta = target / start.scale;
        let zoomed = zoom_at(start, center, delta);
        clamp_scale(zoomed, image_size, viewport)
    }

    const SMALL_IMG: Vec2 = Vec2 { x: 100.0, y: 100.0 };
    const LARGE_VIEW: Vec2 = Vec2 {
        x: 1920.0,
        y: 1080.0,
    };
    /// 4:2 image larger than the viewport in both dims: fit floor 0.2, so
    /// every preset scale (0.5 / 1.0 / 2.0) sits above the floor.
    const LARGE_IMG: Vec2 = Vec2 {
        x: 4000.0,
        y: 2000.0,
    };

    #[test]
    fn preset_scales_above_floor_keep_exact_scale_and_center() {
        // LARGE_IMG/VIEW: fit floor 0.2 → all three preset scales are above
        // it and below MAX_SCALE, so clamping must keep the scale (and the
        // centered anchor) untouched.
        let fit_state = fit(LARGE_IMG, VIEW);
        for target in [0.5f32, 1.0, 2.0] {
            let out = preset_scale_through_clamp(fit_state, target, LARGE_IMG, VIEW);
            assert!(
                (out.scale - target).abs() < 1e-5,
                "preset {target} drifted to {}",
                out.scale
            );
            // Viewport-center anchoring: the image point at the center
            // before equals the one after (same tolerance as the
            // cursor-anchor tests above).
            let center = Vec2 {
                x: VIEW.x / 2.0,
                y: VIEW.y / 2.0,
            };
            let img_before_x = (center.x - fit_state.offset.x) / fit_state.scale;
            let img_after_x = (center.x - out.offset.x) / out.scale;
            assert!((img_before_x - img_after_x).abs() < 1e-2);
        }
    }

    #[test]
    fn preset_scale_exactly_at_floor_is_kept_by_clamp_scale() {
        // Boundary between the above-floor and sub-floor scenarios: a target
        // exactly equal to the fit floor survives `clamp_scale` unchanged
        // (the session-level snap decision belongs to `clamp_zoom`'s epsilon
        // predicate and is pinned in sh-app). Geometry: 300x100 in a 600x200
        // view fits exactly at 2.0 in both axes → floor = min(2.0, 2.0) = 2.0.
        let img = Vec2 { x: 300.0, y: 100.0 };
        let view = Vec2 { x: 600.0, y: 200.0 };
        let floor = fit_scale(img.x, img.y, view.x, view.y);
        assert!((floor - 2.0).abs() < 1e-5);
        let out = preset_scale_through_clamp(fit(img, view), 2.0, img, view);
        assert!(
            (out.scale - 2.0).abs() < 1e-5,
            "target == floor must stay 2.0"
        );
    }

    #[test]
    fn preset_sub_floor_50_snaps_back_to_fit_scale_instead_of_sticking() {
        // 100x100 image in 1920x1080: fit floor = 10.8 → capped to MAX_SCALE.
        // A 0.5 preset request is deep sub-floor; clamp_scale must lift it to
        // the floor (and re-center), NOT leave it sticking at 0.5. The
        // session-level snap-back to FitMode::Fit is pinned in sh-app.
        let fit_state = fit(SMALL_IMG, LARGE_VIEW);
        let out = preset_scale_through_clamp(fit_state, 0.5, SMALL_IMG, LARGE_VIEW);
        assert!(
            out.scale >= fit_scale(SMALL_IMG.x, SMALL_IMG.y, LARGE_VIEW.x, LARGE_VIEW.y) - 1e-5,
            "sub-floor preset scale must never stick below the floor"
        );
        assert!((out.scale - MAX_SCALE).abs() < 1e-5);
        assert!((out.scale - 0.5).abs() > 1.0);
    }

    #[test]
    fn preset_at_floor_scale_returns_the_fit_state() {
        // A target exactly at the fit floor comes out of the funnel as the
        // fit state (scale AND centered offset): `clamp_scale` keeps it and
        // the session-level epsilon then snaps the mode back to Fit.
        let fit_state = fit(IMG, VIEW); // floor 0.8
        let out = preset_scale_through_clamp(fit_state, 0.8, IMG, VIEW);
        let expected = fit(IMG, VIEW);
        assert!((out.scale - expected.scale).abs() < 1e-5);
        assert!((out.offset.x - expected.offset.x).abs() < 1e-4);
        assert!((out.offset.y - expected.offset.y).abs() < 1e-4);
    }

    #[test]
    fn preset_degenerate_start_scale_seeds_from_fit() {
        // A fresh Session::default() carries scale 0.0; the seed guard must
        // route through fit BEFORE the delta divides by the start scale.
        let degenerate = ZoomState {
            scale: 0.0,
            offset: Vec2 { x: 0.0, y: 0.0 },
        };
        let out = preset_scale_through_clamp(degenerate, 1.0, IMG, VIEW);
        assert!((out.scale - 1.0).abs() < 1e-5);
        // And without the guard this would divide by zero (NaN), so also
        // assert sanity explicitly:
        assert!(out.scale.is_finite());
    }
}
