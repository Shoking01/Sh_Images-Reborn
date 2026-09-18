//! General section rows: language picker, hidden-files toggle,
//! recent folders + clear.
//!
//! Display strings resolve through the `sh-core` string table threaded
//! from `Settings.language`; nothing here owns English literals.

use sh_core::i18n::{t, Language, StrKey};

/// Picker rows in order: `(language, autonym)`. Autonyms are proper nouns
/// (design open question, assumed yes): identical in both languages.
pub fn language_options() -> [(Language, &'static str); 2] {
    [
        (Language::En, t(Language::En, StrKey::LanguageEnglish)),
        (Language::Es, t(Language::Es, StrKey::LanguageSpanish)),
    ]
}

/// Language to show before the user picks: the stored pref, defaulting to
/// English. Never reads the OS locale (binding decision) — a Spanish OS
/// with no stored pref still renders English.
pub fn initial_language(stored: Option<Language>) -> Language {
    stored.unwrap_or_default()
}

/// Label for the recents header, e.g. `"Recent folders (N)"`.
/// Pure wrapper over the table template (kept for call-site readability).
pub fn recents_header(lang: Language, count: usize) -> String {
    sh_core::i18n::recents_header(lang, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_offers_english_default_and_spanish_only_alternative() {
        let opts = language_options();
        assert_eq!(opts.len(), 2, "only English + Spanish exist");
        assert_eq!(opts[0].0, Language::En);
        assert_eq!(opts[0].1, "English");
        assert_eq!(opts[1].0, Language::Es);
        assert_eq!(opts[1].1, "Español");
        // English default: the stored-pref default selects the first row.
        assert_eq!(initial_language(None), opts[0].0);
    }

    #[test]
    fn picker_autonyms_are_identical_in_both_languages() {
        assert_eq!(
            t(Language::En, StrKey::LanguageEnglish),
            t(Language::Es, StrKey::LanguageEnglish),
        );
        assert_eq!(
            t(Language::En, StrKey::LanguageSpanish),
            t(Language::Es, StrKey::LanguageSpanish),
        );
    }

    #[test]
    fn initial_language_ignores_os_locale_and_defaults_to_english() {
        // No env read anywhere on this path: the choice is a pure function
        // of the stored pref, so a Spanish OS + no stored pref is English.
        assert_eq!(Language::default(), Language::En);
        assert_eq!(initial_language(None), Language::En);
        assert_eq!(initial_language(Some(Language::Es)), Language::Es);
        assert_eq!(initial_language(Some(Language::En)), Language::En);
    }

    #[test]
    fn recents_header_counts_in_both_languages() {
        // Relocated behind the table (S2): English asserts via `t(En, …)`
        // semantics, Spanish as complete sentences.
        assert_eq!(recents_header(Language::En, 0), "Recent folders (0)");
        assert_eq!(recents_header(Language::En, 3), "Recent folders (3)");
        assert_eq!(recents_header(Language::Es, 0), "Carpetas recientes (0)");
        assert_eq!(recents_header(Language::Es, 3), "Carpetas recientes (3)");
    }
}
