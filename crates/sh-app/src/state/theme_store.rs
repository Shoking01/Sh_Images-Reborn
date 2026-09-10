//! Active theme state; UI maps theme tokens to colors.

use sh_core::theme::Theme;
use std::path::PathBuf;

/// The active theme plus its source path (for hot reload identity).
#[derive(Debug, Clone)]
pub struct ThemeStore {
    /// The currently active validated theme.
    pub theme: Theme,
    /// Filesystem path this theme was loaded from.
    pub path: PathBuf,
    /// Last error from hot-reload attempt, if any.
    pub error: Option<String>,
}

impl ThemeStore {
    /// Create a store with a loaded theme.
    pub fn new(theme: Theme, path: PathBuf) -> Self {
        Self {
            theme,
            path,
            error: None,
        }
    }

    /// Replace the theme, clearing any error state.
    pub fn set(&mut self, theme: Theme, path: PathBuf) {
        self.theme = theme;
        self.path = path;
        self.error = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_theme() -> Theme {
        sh_core::theme::parse(
            r##"{
            "name": "Test",
            "author": "test",
            "version": 1,
            "colors": { "background": "#000", "surface": "#111", "text": "#fff", "accent": "#0f0" },
            "spacing": { "xs": 4, "sm": 8, "md": 16, "lg": 24 },
            "radii": { "sm": 2, "md": 6, "lg": 12 },
            "typography": { "family": "Inter", "sizes": { "caption": 11, "body": 14, "title": 18 } }
        }"##,
        )
        .unwrap()
    }

    #[test]
    fn new_sets_theme_and_clears_error() {
        let store = ThemeStore::new(sample_theme(), PathBuf::from("/themes/test.json"));
        assert_eq!(store.theme.name, "Test");
        assert!(store.error.is_none());
    }

    #[test]
    fn set_replaces_theme() {
        let mut store = ThemeStore::new(sample_theme(), PathBuf::from("/a.json"));
        let mut t = sample_theme();
        t.name = "Replaced".into();
        store.set(t, PathBuf::from("/b.json"));
        assert_eq!(store.theme.name, "Replaced");
        assert_eq!(store.path, PathBuf::from("/b.json"));
        assert!(store.error.is_none());
    }

    #[test]
    fn set_clears_previous_error() {
        let mut store = ThemeStore::new(sample_theme(), PathBuf::from("/a.json"));
        store.error = Some("old error".into());
        store.set(sample_theme(), PathBuf::from("/b.json"));
        assert!(store.error.is_none());
    }
}
