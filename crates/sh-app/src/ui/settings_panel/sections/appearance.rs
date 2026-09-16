//! Appearance section: theme picker (moved here from the old topbar
//! dropdown) + display toggles already present in settings.

/// Display name for a builtin theme file: parsed theme name, file fallback.
pub fn theme_display_name(file: &str, json: &str) -> String {
    sh_core::theme::parse(json)
        .map(|t| t.name)
        .unwrap_or_else(|_| file.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_display_prefers_parsed_name() {
        let json = crate::theme_builtins::builtin_theme_json("dark-clinical.json");
        assert_eq!(
            theme_display_name("dark-clinical.json", json),
            "Dark Clinical"
        );
        assert_eq!(theme_display_name("x.json", "{ broken"), "x.json");
    }
}
