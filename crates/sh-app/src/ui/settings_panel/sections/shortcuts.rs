//! Shortcuts section: binding-chip text, capture helpers.
//!
//! Display strings resolve through the `sh-core` string table threaded
//! from `Settings.language`; nothing here owns English literals.

use sh_core::i18n::{t, Language, StrKey};
use sh_core::keymap::KeyBinding;

/// Chip text for a stored binding: the keymap string itself
/// (`"ctrl-shift-o"`). One format everywhere — no display/parse drift.
pub fn chip_text(binding: &KeyBinding) -> String {
    binding.to_keymap_string()
}

/// Text shown while a row awaits a keypress (`Esc` is a key chip,
/// untranslated). Resolves via the table so it follows the UI language.
pub fn capture_prompt(lang: Language) -> &'static str {
    t(lang, StrKey::CapturePrompt)
}

/// Conflict message for a rejected combo. `incumbent` is the already
/// localized label of the holding action (resolved by the caller via
/// [`action_label`]), passed through as a proper noun.
pub fn conflict_text(lang: Language, incumbent: &str) -> String {
    sh_core::i18n::conflict_text(lang, incumbent)
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

/// Human label for an action id via the ACTIONS table, rendered in `lang`;
/// falls back to the id itself for unknown (future) actions.
pub fn action_label(id: &str, lang: Language) -> &str {
    crate::actions::ACTIONS
        .iter()
        .find(|a| a.id == id)
        .map(|a| lang.get(a.label_key))
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
    fn capture_prompt_and_conflict_come_from_the_table() {
        use sh_core::i18n::{t, Language, StrKey};
        // English assertions resolve through the table (S2 relocation).
        assert_eq!(
            t(Language::En, StrKey::CapturePrompt),
            "Press keys… (Esc to cancel)"
        );
        assert_eq!(
            t(Language::Es, StrKey::CapturePrompt),
            "Pulse teclas… (Esc para cancelar)"
        );
        assert_eq!(
            conflict_text(Language::En, "Close"),
            "Already used by Close"
        );
        assert_eq!(
            conflict_text(Language::Es, "Imagen siguiente"),
            "Ya en uso por Imagen siguiente"
        );
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
    fn action_label_resolves_in_both_languages_or_falls_back() {
        use sh_core::i18n::Language;
        assert_eq!(action_label("next-image", Language::En), "Next image");
        assert_eq!(action_label("next-image", Language::Es), "Imagen siguiente");
        assert_eq!(action_label("open-settings", Language::Es), "Abrir ajustes");
        assert_eq!(
            action_label("no-such-action", Language::En),
            "no-such-action"
        );
        assert_eq!(
            action_label("no-such-action", Language::Es),
            "no-such-action"
        );
    }
}
