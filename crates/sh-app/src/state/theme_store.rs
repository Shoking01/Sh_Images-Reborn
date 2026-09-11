//! Active theme state; UI maps theme tokens to colors.

use sh_core::theme::Theme;
use std::path::PathBuf;

/// The active theme plus its source path (for hot reload identity).
#[derive(Debug, Clone)]
pub struct ThemeStore {
    /// The currently active validated theme.
    pub theme: Theme,
    /// Settings-file name of the active theme (e.g. `deep-neutral.json`).
    ///
    /// Distinct from [`Theme::name`]: this is the on-disk identity used for
    /// persistence (`settings.json`) and theme-file resolution, not the
    /// display name declared inside the theme JSON.
    pub name: String,
    /// Filesystem path this theme was loaded from.
    pub path: PathBuf,
    /// Last error from hot-reload attempt, if any.
    pub error: Option<String>,
}

impl ThemeStore {
    /// Create a store with a loaded theme and its settings-file name.
    pub fn new(theme: Theme, name: String, path: PathBuf) -> Self {
        Self {
            theme,
            name,
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

/// Startup decision for resolving the active theme file: either use the
/// user's file as-is, or bootstrap it from a built-in theme.
///
/// Pure so the fresh-install bootstrap path is unit-testable without disk
/// I/O (the caller performs the actual read/write).
#[derive(Debug, PartialEq)]
pub enum ThemeStartup {
    /// The theme file exists on disk; load and parse it.
    UseExisting,
    /// The file is missing; write the built-in theme JSON there so users
    /// (and the hot-reload acceptance flow) have an editable file from the
    /// very first run.
    Bootstrap { builtin_json: &'static str },
}

/// Resolve how to obtain the active theme at startup, given whether the
/// settings-named theme file already exists.
///
/// Missing file → [`ThemeStartup::Bootstrap`] with the built-in JSON the
/// caller should write. Existing file (whatever it contains — validity is
/// handled by parse + fallback at the call site) → [`ThemeStartup::UseExisting`].
pub fn theme_startup(name: &str, file_exists: bool) -> ThemeStartup {
    if file_exists {
        ThemeStartup::UseExisting
    } else {
        let builtin_json = crate::theme_builtins::builtin_theme_json(name);
        ThemeStartup::Bootstrap { builtin_json }
    }
}

/// Hot-reload decision from (last applied text, newly read text), before
/// any parsing: skip unchanged files, otherwise parse.
///
/// Pure: the dedupe-by-text rule (same bytes → no work) is exactly what
/// keeps a 1s poll from re-parsing — and re-notifying — on every tick.
#[derive(Debug, PartialEq)]
pub enum HotReloadDecision {
    /// File text identical to the last applied theme: nothing to do.
    Unchanged,
    /// Text differs: parse it and apply on success.
    Apply,
}

/// Decide whether newly read theme text warrants a parse+apply attempt.
pub fn hot_reload_decision(last_applied: &str, new_text: &str) -> HotReloadDecision {
    if last_applied == new_text {
        HotReloadDecision::Unchanged
    } else {
        HotReloadDecision::Apply
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
        let store = ThemeStore::new(
            sample_theme(),
            "test.json".into(),
            PathBuf::from("/themes/test.json"),
        );
        assert_eq!(store.theme.name, "Test");
        assert!(store.error.is_none());
    }

    #[test]
    fn set_replaces_theme() {
        let mut store = ThemeStore::new(sample_theme(), "a.json".into(), PathBuf::from("/a.json"));
        let mut t = sample_theme();
        t.name = "Replaced".into();
        store.set(t, PathBuf::from("/b.json"));
        assert_eq!(store.theme.name, "Replaced");
        assert_eq!(store.path, PathBuf::from("/b.json"));
        assert!(store.error.is_none());
    }

    #[test]
    fn set_clears_previous_error() {
        let mut store = ThemeStore::new(sample_theme(), "a.json".into(), PathBuf::from("/a.json"));
        store.error = Some("old error".into());
        store.set(sample_theme(), PathBuf::from("/b.json"));
        assert!(store.error.is_none());
    }

    #[test]
    fn new_stores_theme_name() {
        let store = ThemeStore::new(
            sample_theme(),
            "deep-neutral.json".into(),
            PathBuf::from("/a.json"),
        );
        assert_eq!(store.name, "deep-neutral.json");
    }

    #[test]
    fn set_keeps_name_updates_path() {
        let mut store = ThemeStore::new(sample_theme(), "a.json".into(), PathBuf::from("/a.json"));
        store.set(sample_theme(), PathBuf::from("/b.json"));
        assert_eq!(store.name, "a.json");
        assert_eq!(store.path, PathBuf::from("/b.json"));
    }

    // ── theme_startup ──

    #[test]
    fn startup_existing_file_uses_it() {
        assert_eq!(
            theme_startup("custom.json", true),
            ThemeStartup::UseExisting
        );
    }

    #[test]
    fn startup_missing_file_bootstraps_builtin() {
        match theme_startup("deep-neutral.json", false) {
            ThemeStartup::Bootstrap { builtin_json } => {
                assert!(!builtin_json.is_empty());
                // Bootstrapped JSON must itself be a valid theme.
                assert!(sh_core::theme::parse(builtin_json).is_ok());
            }
            other => panic!("expected Bootstrap, got {other:?}"),
        }
    }

    #[test]
    fn startup_missing_unknown_name_still_bootstraps() {
        // Any unknown name falls back to deep-neutral for the bootstrap.
        assert!(matches!(
            theme_startup("no-such-theme.json", false),
            ThemeStartup::Bootstrap { .. }
        ));
    }

    // ── hot_reload_decision ──

    #[test]
    fn hot_reload_unchanged_text_skips() {
        assert_eq!(
            hot_reload_decision("{...}", "{...}"),
            HotReloadDecision::Unchanged
        );
    }

    #[test]
    fn hot_reload_changed_text_applies() {
        assert_eq!(
            hot_reload_decision("{old}", "{new}"),
            HotReloadDecision::Apply
        );
    }

    #[test]
    fn hot_reload_first_read_applies() {
        // Empty last-applied vs real content: apply.
        assert_eq!(hot_reload_decision("", "{}"), HotReloadDecision::Apply);
    }

    #[test]
    fn hot_reload_empty_to_empty_skips() {
        assert_eq!(hot_reload_decision("", ""), HotReloadDecision::Unchanged);
    }
}
