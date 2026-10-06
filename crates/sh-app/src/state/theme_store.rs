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

/// The theme file the Theme Editor is authoring, once one has been opened.
///
/// **Why this is a target and not a mirror of [`ThemeStore`].** The draft
/// belongs to a theme the user pointed at, not to whatever theme happens to
/// be applied. Those are two different questions — "what is showing" and "what
/// am I editing" — and keying the draft on the applied theme is what made
/// picking a different theme silently discard unsaved edits. Holding an
/// explicit target makes the re-seed condition "the target changed", which
/// only happens on a deliberate Edit action.
///
/// # Why a PATH and not the theme name
///
/// `merge_theme_entries` dedupes rows by theme NAME and documents that two
/// files may declare the same name. A name therefore does not identify a file,
/// and a save path derived from one would overwrite whichever file the row
/// happened to be. The path is the only identity a theme file has here.
///
/// # Copy-on-write
///
/// A built-in theme has no file at all: it is compiled into the binary. Its
/// [`path`](Self::path) is the *user copy's* destination under the config
/// themes directory — the same file [`theme_startup`] and
/// `App::apply_theme_entry` create so the theme stays editable and
/// hot-reloadable. Editing a built-in therefore writes a new file there and
/// leaves the compiled-in JSON untouched; [`from_builtin`](Self::from_builtin)
/// records that this target is such a destination rather than a file that
/// already exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeEditTarget {
    /// Where a save writes. The one identity a theme file has.
    pub path: PathBuf,
    /// The filename `Settings::theme` is persisted under after a successful
    /// save — the identity [`ThemeStore::new`] adopts.
    pub file_name: String,
    /// The target is a copy-on-write destination for a built-in, not a file
    /// that already existed when the editor was opened.
    pub from_builtin: bool,
}

/// What the last save attempt reported back to the Theme Editor.
///
/// A separate type per outcome rather than one message string, so the editor
/// can react differently: [`NameTaken`](Self::NameTaken) and
/// [`Missing`](Self::Missing) are each a fact about the FILE,
/// [`Invalid`](Self::Invalid) names a row to mark, and
/// [`SaveFailed`](Self::SaveFailed) is the only one worth a log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeEditorMessage {
    /// A built-in's copy destination is already occupied. Nothing was written.
    ///
    /// Reachable only for the copy-on-write case, and that narrowing is the
    /// point. Once a user-owned target replaced its file in place, "editing a
    /// theme that already exists" stopped being this outcome and became a
    /// successful save — so this now means one specific thing: the destination
    /// for a built-in's copy is unexpectedly taken, and the user's intent was
    /// otherwise valid.
    NameTaken(PathBuf),
    /// The file being edited is gone. Nothing was written.
    ///
    /// Its own arm rather than folded into [`NameTaken`](Self::NameTaken)
    /// because the two are opposites. Before the replace path landed, a missing
    /// target surfaced as `NameTaken` — which told a user whose theme had been
    /// deleted that a name was taken, sending them to look for a collision that
    /// did not exist.
    ///
    /// Carries a path rather than a `field`, because the fault is the FILE: no
    /// row of the draft is at fault, so
    /// [`field_label`](Self::field_label) deliberately marks nothing.
    Missing(PathBuf),
    /// The draft does not resolve. `field` is the row label the error names,
    /// when it names one — the editor marks that row rather than reporting a
    /// failure the user cannot locate.
    Invalid {
        /// The [`crate::ui::settings_panel::sections::theme_editor::Field`]
        /// label this error names, or `None` for a message that names none.
        field: Option<String>,
        /// The message verbatim, as the draft produced it.
        text: String,
    },
    /// A typed I/O failure; nothing was written.
    SaveFailed(String),
}

impl ThemeEditorMessage {
    /// The draft's own validation failure, marked on the row it names.
    ///
    /// The single place an `Invalid` is built, because BOTH write paths produce
    /// one and the mapping from message to row was duplicated per arm. A second
    /// copy would be free to drift and mark a different row than the first.
    pub fn invalid(text: String) -> Self {
        use crate::ui::settings_panel::sections::theme_editor as te;
        let field = te::field_named_by(&text).map(|field| te::label(field).to_string());
        Self::Invalid { field, text }
    }

    /// The line the editor header shows for this outcome.
    ///
    /// The `Invalid` and `SaveFailed` arms pass core's own text through
    /// untouched, which is how every other error in this app reaches the user
    /// (see `session.error`). Only the two file-level arms compose a message
    /// here, and both interpolate the file name through [`sh_core::i18n`] so
    /// they are translated like every other string in the app.
    pub fn text(&self, lang: sh_core::i18n::Language) -> String {
        use sh_core::i18n::{theme_copy_taken, theme_target_missing};
        match self {
            Self::NameTaken(path) => theme_copy_taken(lang, &file_name_of(path)),
            Self::Missing(path) => theme_target_missing(lang, &file_name_of(path)),
            Self::Invalid { text, .. } | Self::SaveFailed(text) => text.clone(),
        }
    }

    /// The row label this message is about, or `None` when it names none.
    ///
    /// Only an [`Invalid`](Self::Invalid) can: it is the one outcome produced
    /// by the draft's own validation, which reports per slot. The two
    /// file-level refusals are about the file, not about a field, so they mark
    /// no row rather than marking a plausible-looking one.
    pub fn field_label(&self) -> Option<&str> {
        match self {
            Self::Invalid { field, .. } => field.as_deref(),
            _ => None,
        }
    }
}

/// The file name a path contributes to a user-facing message, falling back to
/// the whole path when it has none.
fn file_name_of(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
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
