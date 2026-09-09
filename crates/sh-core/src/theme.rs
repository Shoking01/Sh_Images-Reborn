//! Theme parsing, validation, and discovery (AGENTS.md §10).
//!
//! Note (approved design deviation): on invalid theme hot-reload input, the app
//! keeps the last valid theme instead of falling back to the default per
//! AGENTS.md §10. Hot-reload orchestration lives in a later task; this module
//! only parses, validates, and discovers themes.

use crate::errors::{Result, ShImagesError};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A validated theme.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Theme {
    pub name: String,
    pub author: String,
    pub version: u32,
    pub colors: ThemeColors,
    pub spacing: ThemeSpacing,
    pub radii: ThemeRadii,
    pub typography: ThemeTypography,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThemeColors {
    pub background: String,
    pub surface: String,
    pub text: String,
    pub accent: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThemeSpacing {
    pub xs: u32,
    pub sm: u32,
    pub md: u32,
    pub lg: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThemeRadii {
    pub sm: u32,
    pub md: u32,
    pub lg: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThemeTypography {
    pub family: String,
    pub sizes: ThemeSizes,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThemeSizes {
    pub caption: u32,
    pub body: u32,
    pub title: u32,
}

/// Image extensions recognized by the viewer.
///
/// Temporarily duplicated from `navigation`'s `is_supported` logic; a later
/// refactor makes navigation reuse this list.
const SUPPORTED_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff"];

/// Parse and validate a theme from JSON.
pub fn parse(json: &str) -> Result<Theme> {
    let theme: Theme =
        serde_json::from_str(json).map_err(|e| ShImagesError::Theme(e.to_string()))?;
    validate(&theme)?;
    Ok(theme)
}

/// Validate a theme's values, returning `Theme` error on any violation.
pub fn validate(theme: &Theme) -> Result<()> {
    if theme.name.trim().is_empty() {
        return Err(ShImagesError::Theme("name must not be empty".into()));
    }
    validate_color(&theme.colors.background)?;
    validate_color(&theme.colors.surface)?;
    validate_color(&theme.colors.text)?;
    validate_color(&theme.colors.accent)?;
    if theme.typography.family.trim().is_empty() {
        return Err(ShImagesError::Theme(
            "typography.family must not be empty".into(),
        ));
    }
    Ok(())
}

/// Returns true if `hex` is a valid 3, 6, or 8-digit hex color (with optional leading `#`).
pub fn validate_color(hex: &str) -> Result<()> {
    let trimmed = hex.trim_start_matches('#');
    let valid =
        matches!(trimmed.len(), 3 | 6 | 8) && trimmed.chars().all(|c| c.is_ascii_hexdigit());
    if valid {
        Ok(())
    } else {
        Err(ShImagesError::Theme(format!("invalid color: {hex}")))
    }
}

/// Discover theme files in a config directory (`.json` extension, sorted).
pub fn discover(config_dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(config_dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// Supported extensions, public for navigation reuse.
pub fn supported_extensions() -> &'static [&'static str] {
    SUPPORTED_EXTENSIONS
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r##"{
        "name": "Neon Nights",
        "author": "username",
        "version": 1,
        "colors": { "background": "#0a0a1a", "surface": "#12122a", "text": "#e0e0ff", "accent": "#00ffff" },
        "spacing": { "xs": 4, "sm": 8, "md": 16, "lg": 24 },
        "radii": { "sm": 2, "md": 6, "lg": 12 },
        "typography": { "family": "Inter", "sizes": { "caption": 11, "body": 14, "title": 18 } }
    }"##;

    #[test]
    fn parses_valid_theme() {
        let t = parse(VALID).unwrap();
        assert_eq!(t.name, "Neon Nights");
        assert_eq!(t.colors.background, "#0a0a1a");
        assert_eq!(t.spacing.md, 16);
    }

    #[test]
    fn rejects_malformed_json() {
        let err = parse("{ not json").unwrap_err();
        assert!(matches!(err, ShImagesError::Theme(_)));
    }

    #[test]
    fn rejects_missing_required_field() {
        let json = r##"{"name":"X","author":"a","version":1,"colors":{"background":"#000","surface":"#111","text":"#fff","accent":"#0f0"},"spacing":{"xs":1,"sm":2,"md":3,"lg":4},"radii":{"sm":1,"md":2,"lg":3},"typography":{"family":"Arial","sizes":{"caption":1,"body":2,"title":3}}}"##;
        // Remove a required field (spacing) to prove missing-field mismatches fail:
        let arr = serde_json::from_str::<serde_json::Value>(json).unwrap();
        let mut obj = arr.as_object().unwrap().clone();
        obj.remove("spacing");
        let bad = serde_json::to_string(&obj).unwrap();
        assert!(parse(&bad).is_err());
    }

    #[test]
    fn rejects_invalid_color_formats() {
        let bad = VALID.replace("#0a0a1a", "#12");
        assert!(parse(&bad).is_err());
        let bad2 = VALID.replace("#0a0a1a", "red");
        assert!(parse(&bad2).is_err());
    }

    #[test]
    fn accepts_three_six_eight_digit_colors() {
        assert!(validate_color("#abc").is_ok());
        assert!(validate_color("#aabbcc").is_ok());
        assert!(validate_color("#aabbccdd").is_ok());
        assert!(validate_color("abc").is_ok());
        assert!(validate_color("#ab").is_err());
        assert!(validate_color("#zzz").is_err());
    }

    #[test]
    fn rejects_empty_name() {
        let json = VALID.replace("Neon Nights", "");
        assert!(parse(&json).is_err());
    }

    #[test]
    fn discover_finds_theme_files_sorted() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b.json"), VALID).unwrap();
        std::fs::write(dir.path().join("a.json"), VALID).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
        let found = discover(dir.path());
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].file_name().unwrap(), "a.json");
        assert_eq!(found[1].file_name().unwrap(), "b.json");
    }

    #[test]
    fn discover_missing_dir_returns_empty() {
        assert!(discover(Path::new("Z:/definitely/not/here")).is_empty());
    }

    #[test]
    fn parses_builtin_default_theme() {
        let json = include_str!("../../../themes/deep-neutral.json");
        let t = parse(json).unwrap();
        assert_eq!(t.name, "Deep Neutral");
        assert_eq!(t.colors.background, "#0d0d0f");
    }
}
