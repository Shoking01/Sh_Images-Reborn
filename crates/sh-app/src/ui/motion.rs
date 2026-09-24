//! Restrained background-hover motion for persistent controls.

use gpui::prelude::*;
use gpui::{Animation, AnimationExt, App, Div, Hsla, Rgba, Stateful, Window};
use std::collections::BTreeMap;
use std::time::Duration;

/// Duration of an enabled background-hover transition.
pub(crate) const HOVER_MOTION_DURATION: Duration = Duration::from_millis(150);

/// Stable identity for one hover-animated element.
///
/// GPUI 0.2.2 identifies `with_animation` wrappers with [`gpui::ElementId`]
/// rather than a dedicated animation-id type. This small adapter keeps the
/// caller's stable element identity explicit and derives direction-specific
/// wrapper IDs so entering and leaving start independent animations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct AnimationId(&'static str);

impl AnimationId {
    /// Create a stable animation identity from a static element name.
    pub(crate) const fn new(id: &'static str) -> Self {
        Self(id)
    }

    const fn key(self, direction: HoverDirection) -> (&'static str, u32) {
        (
            self.0,
            match direction {
                HoverDirection::Enter => 0,
                HoverDirection::Leave => 1,
            },
        )
    }
}

/// Direction of a background-hover transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HoverDirection {
    /// Move from the idle background to the hover background.
    Enter,
    /// Move from the hover background to the idle background.
    Leave,
}

/// Easing policies supported by the motion layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnimationEasing {
    /// Quintic ease-out: immediate response with a soft, controlled settle.
    ///
    /// GPUI applies this curve once before invoking the animator. The animator
    /// must interpolate colors directly with the resulting progress rather
    /// than applying another easing curve.
    EaseOutQuint,
}

impl AnimationEasing {
    pub(crate) fn configure(self, animation: Animation) -> Animation {
        match self {
            Self::EaseOutQuint => animation.with_easing(gpui::ease_out_quint()),
        }
    }
}

/// Current hover phase for one stable animation identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HoverPhase {
    /// The pointer has not entered this element yet; render its idle state.
    Idle,
    /// The pointer entered; the target is the hover background.
    Entering,
    /// The pointer left after an observed enter; the target is idle.
    Leaving,
}

impl HoverPhase {
    pub(crate) const fn is_hovered(self) -> bool {
        matches!(self, Self::Entering)
    }
}

/// Pure decision made before constructing an animated element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MotionDecision {
    /// Paint the target background immediately.
    Instant,
    /// Run a bounded background interpolation.
    Animate(HoverDirection),
}

pub(crate) const fn background_motion_decision(
    phase: HoverPhase,
    reduce_motion: bool,
) -> MotionDecision {
    if reduce_motion || matches!(phase, HoverPhase::Idle) {
        MotionDecision::Instant
    } else if matches!(phase, HoverPhase::Entering) {
        MotionDecision::Animate(HoverDirection::Enter)
    } else {
        MotionDecision::Animate(HoverDirection::Leave)
    }
}

/// Hover phases for the bounded set of app-owned animated controls.
#[derive(Debug, Default)]
pub(crate) struct HoverMotionState {
    hovered: BTreeMap<AnimationId, bool>,
}

impl HoverMotionState {
    pub(crate) fn phase(&self, id: AnimationId) -> HoverPhase {
        match self.hovered.get(&id) {
            None => HoverPhase::Idle,
            Some(true) => HoverPhase::Entering,
            Some(false) => HoverPhase::Leaving,
        }
    }

    /// Record a pointer transition, returning whether render state changed.
    pub(crate) fn set_hovered(&mut self, id: AnimationId, hovered: bool) -> bool {
        if hovered {
            return match self.hovered.get_mut(&id) {
                Some(current) if *current => false,
                Some(current) => {
                    *current = true;
                    true
                }
                None => {
                    self.hovered.insert(id, true);
                    true
                }
            };
        }

        match self.hovered.get_mut(&id) {
            Some(current) if *current => {
                *current = false;
                true
            }
            Some(_) | None => false,
        }
    }

    /// Forget transitions when a control surface unmounts.
    pub(crate) fn clear(&mut self) {
        self.hovered.clear();
    }
}

fn interpolate_background(from: Hsla, to: Hsla, progress: f32) -> Hsla {
    let from: Rgba = from.into();
    let to: Rgba = to.into();
    let progress = progress.clamp(0.0, 1.0);
    let mix = |start: f32, end: f32| start + (end - start) * progress;
    Rgba {
        r: mix(from.r, to.r),
        g: mix(from.g, to.g),
        b: mix(from.b, to.b),
        a: mix(from.a, to.a),
    }
    .into()
}

/// Attach hover behavior to a fully interactive element and return the
/// type-erased boundary required by GPUI's `AnimationElement` wrapper.
///
/// Callers must attach stable IDs and existing mouse/click listeners before
/// calling this function because `with_animation` returns an
/// `AnimationElement`, not the original `Stateful<Div>`.
pub(crate) fn hover_background(
    mut element: Stateful<Div>,
    animation_id: AnimationId,
    phase: HoverPhase,
    reduce_motion: bool,
    idle_bg: Hsla,
    hover_bg: Hsla,
    on_hover: impl Fn(&bool, &mut Window, &mut App) + 'static,
) -> gpui::AnyElement {
    element = element.on_hover(on_hover);

    let instant_bg = if phase.is_hovered() {
        hover_bg
    } else {
        idle_bg
    };
    if reduce_motion {
        return element
            .bg(instant_bg)
            .hover(move |style| style.bg(hover_bg))
            .into_any_element();
    }

    match background_motion_decision(phase, reduce_motion) {
        MotionDecision::Instant => element.bg(instant_bg).into_any_element(),
        MotionDecision::Animate(direction) => {
            let (from, to) = match direction {
                HoverDirection::Enter => (idle_bg, hover_bg),
                HoverDirection::Leave => (hover_bg, idle_bg),
            };
            let animation =
                AnimationEasing::EaseOutQuint.configure(Animation::new(HOVER_MOTION_DURATION));
            element
                .bg(idle_bg)
                .with_animation(
                    animation_id.key(direction),
                    animation,
                    move |element, progress| {
                        // `progress` is already eased by GPUI. Interpolating
                        // directly avoids double-easing the same transition.
                        element.bg(interpolate_background(from, to, progress))
                    },
                )
                .into_any_element()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        background_motion_decision, AnimationEasing, AnimationId, HoverDirection, HoverMotionState,
        HoverPhase, MotionDecision, HOVER_MOTION_DURATION,
    };
    use gpui::Animation;
    use std::time::Duration;

    const STABLE_ID: &str = "settings-motion-row";

    #[test]
    fn motion_duration_is_150ms() {
        assert_eq!(HOVER_MOTION_DURATION, Duration::from_millis(150));
    }

    #[test]
    fn motion_animation_ids_are_stable_and_direction_specific() {
        let id = AnimationId::new(STABLE_ID);

        assert_eq!(id.key(HoverDirection::Enter), (STABLE_ID, 0));
        assert_eq!(id.key(HoverDirection::Leave), (STABLE_ID, 1));
        assert_ne!(id.key(HoverDirection::Enter), id.key(HoverDirection::Leave));
    }

    #[test]
    fn motion_easing_uses_one_ease_out_quint_curve() {
        let easing = AnimationEasing::EaseOutQuint;
        let animation = easing.configure(Animation::new(HOVER_MOTION_DURATION));

        assert_eq!(animation.duration, HOVER_MOTION_DURATION);
        assert!(animation.oneshot);
        assert!(((animation.easing)(0.0) - 0.0).abs() < f32::EPSILON);
        assert!(((animation.easing)(0.5) - 0.96875).abs() < f32::EPSILON);
        assert!(((animation.easing)(1.0) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn motion_reduced_motion_bypasses_animation_for_every_phase() {
        for phase in [HoverPhase::Idle, HoverPhase::Entering, HoverPhase::Leaving] {
            assert_eq!(
                background_motion_decision(phase, true),
                MotionDecision::Instant
            );
        }

        assert_eq!(
            background_motion_decision(HoverPhase::Idle, false),
            MotionDecision::Instant
        );
        assert_eq!(
            background_motion_decision(HoverPhase::Entering, false),
            MotionDecision::Animate(HoverDirection::Enter)
        );
        assert_eq!(
            background_motion_decision(HoverPhase::Leaving, false),
            MotionDecision::Animate(HoverDirection::Leave)
        );
        assert!(HoverPhase::Entering.is_hovered());
        assert!(!HoverPhase::Idle.is_hovered());
        assert!(!HoverPhase::Leaving.is_hovered());
    }

    #[test]
    fn motion_state_tracks_stable_hover_transitions_without_spurious_exit() {
        let id = AnimationId::new(STABLE_ID);
        let mut state = HoverMotionState::default();

        assert_eq!(state.phase(id), HoverPhase::Idle);
        assert!(!state.set_hovered(id, false));
        assert_eq!(state.phase(id), HoverPhase::Idle);
        assert!(state.set_hovered(id, true));
        assert_eq!(state.phase(id), HoverPhase::Entering);
        assert!(state.set_hovered(id, false));
        assert_eq!(state.phase(id), HoverPhase::Leaving);
        assert!(!state.set_hovered(id, false));
    }
}
