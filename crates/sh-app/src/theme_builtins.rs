//! Built-in theme JSON sources for bootstrap and fallback.
//!
//! V1 ships a single built-in theme; the map shape keeps resolution total
//! (unknown names fall back to the default) so startup code never has to
//! handle a missing builtin as an error path.

/// The default built-in theme's settings-file name.
pub const DEFAULT_THEME_NAME: &str = "deep-neutral.json";

/// Resolve a settings-file theme name to its built-in JSON source.
///
/// Unknown names fall back to the default theme; the returned string is
/// always valid theme JSON (proven by sh-core's `parses_all_builtin_themes`
/// test plus the `startup_missing_file_bootstraps_builtin` app test).
pub fn builtin_theme_json(name: &str) -> &'static str {
    match name {
        DEFAULT_THEME_NAME => include_str!("../../../themes/deep-neutral.json"),
        _ => include_str!("../../../themes/deep-neutral.json"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_name_resolves_to_default_json() {
        let json = builtin_theme_json(DEFAULT_THEME_NAME);
        assert!(sh_core::theme::parse(json).is_ok());
        assert_eq!(sh_core::theme::parse(json).unwrap().name, "Deep Neutral");
    }

    #[test]
    fn unknown_name_falls_back_to_default_json() {
        let json = builtin_theme_json("no-such-theme.json");
        assert!(sh_core::theme::parse(json).is_ok());
    }
}
