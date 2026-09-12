//! Built-in theme JSON sources for bootstrap and fallback.
//!
//! V1 ships a single built-in theme; the map shape keeps resolution total
//! (unknown names fall back to the default) so startup code never has to
//! handle a missing builtin as an error path.

/// The default built-in theme's settings-file name.
pub const DEFAULT_THEME_NAME: &str = "noir-gallery.json";

/// All built-in themes as (settings-file name, JSON source). Single source
/// of truth for the settings dropdown; every entry must parse (tested).
pub const BUILTIN_THEMES: &[(&str, &str)] = &[
    (
        "noir-gallery.json",
        include_str!("../../../themes/noir-gallery.json"),
    ),
    (
        "deep-neutral.json",
        include_str!("../../../themes/deep-neutral.json"),
    ),
    (
        "dark-clinical.json",
        include_str!("../../../themes/dark-clinical.json"),
    ),
    (
        "light-clean.json",
        include_str!("../../../themes/light-clean.json"),
    ),
];

/// Resolve a settings-file theme name to its built-in JSON source.
///
/// Unknown names fall back to the default theme; the returned string is
/// always valid theme JSON (proven by sh-core's `parses_all_builtin_themes`
/// test plus the `startup_missing_file_bootstraps_builtin` app test).
pub fn builtin_theme_json(name: &str) -> &'static str {
    BUILTIN_THEMES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, json)| *json)
        .unwrap_or(BUILTIN_THEMES[0].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_name_resolves_to_default_json() {
        let json = builtin_theme_json(DEFAULT_THEME_NAME);
        assert!(sh_core::theme::parse(json).is_ok());
        assert_eq!(sh_core::theme::parse(json).unwrap().name, "Noir Gallery");
    }

    #[test]
    fn unknown_name_falls_back_to_default_json() {
        let json = builtin_theme_json("no-such-theme.json");
        assert!(sh_core::theme::parse(json).is_ok());
    }

    #[test]
    fn builtin_list_has_four_distinct_parsing_themes() {
        assert_eq!(BUILTIN_THEMES.len(), 4);
        let mut names = std::collections::HashSet::new();
        for (file, json) in BUILTIN_THEMES {
            assert!(names.insert(*file), "duplicate builtin entry");
            let theme = sh_core::theme::parse(json).expect("builtin must parse");
            assert!(!theme.name.is_empty());
        }
    }
}
