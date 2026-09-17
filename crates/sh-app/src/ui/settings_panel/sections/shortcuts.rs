//! Shortcuts section: binding-chip text, capture helpers.

use sh_core::keymap::KeyBinding;

/// Chip text for a stored binding: the keymap string itself
/// (`"ctrl-shift-o"`). One format everywhere — no display/parse drift.
pub fn chip_text(binding: &KeyBinding) -> String {
    binding.to_keymap_string()
}

/// Text shown while a row awaits a keypress.
pub const CAPTURE_PROMPT: &str = "Press keys… (Esc to cancel)";

/// Conflict message for a rejected combo. `label` is the incumbent action's
/// display label resolved via ACTIONS.
pub fn conflict_text(label: &str) -> String {
    format!("Already used by {label}")
}

/// Whether a keydown is a bare modifier (capture keeps waiting).
/// gpui reports lone-modifier presses with the modifier as the key token.
pub fn is_modifier_only(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "shift"
            | "control"
            | "ctrl"
            | "alt"
            | "option"
            | "cmd"
            | "super"
            | "win"
            | "function"
            | "fn"
    )
}

/// Human label for an action id via the ACTIONS table; falls back to the id.
pub fn action_label(id: &str) -> &str {
    crate::actions::ACTIONS
        .iter()
        .find(|a| a.id == id)
        .map(|a| a.label)
        .unwrap_or(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chip_shows_keymap_string() {
        let b = KeyBinding {
            ctrl: true,
            shift: true,
            alt: false,
            platform: false,
            key: "o".into(),
        };
        assert_eq!(chip_text(&b), "ctrl-shift-o");
    }

    #[test]
    fn capture_prompt_and_conflict_text() {
        assert!(CAPTURE_PROMPT.contains("Esc"));
        assert_eq!(conflict_text("Close"), "Already used by Close");
    }

    #[test]
    fn modifier_only_detection() {
        assert!(is_modifier_only("shift"));
        assert!(is_modifier_only("Control"));
        assert!(is_modifier_only("cmd"));
        assert!(!is_modifier_only("o"));
        assert!(!is_modifier_only("escape"));
        assert!(!is_modifier_only(","));
    }

    #[test]
    fn action_label_resolves_or_falls_back() {
        assert_eq!(action_label("next-image"), "Next image");
        assert_eq!(action_label("no-such-action"), "no-such-action");
    }
}
