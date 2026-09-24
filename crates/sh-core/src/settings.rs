//! User settings schema, defaults, and atomic persistence.

use crate::errors::{Result, ShImagesError};
use crate::i18n::Language;
use crate::keymap::{defaults as default_keymap, Keymap};
use crate::navigation::{SortBy, SortDir};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Current settings schema version written by the application.
pub const CURRENT_SETTINGS_VERSION: u32 = 9;
/// Default delay between automatic slideshow advances, in seconds.
pub const DEFAULT_SLIDESHOW_INTERVAL_SECS: u32 = 3;
/// Smallest supported slideshow interval, in seconds.
pub const SLIDESHOW_INTERVAL_MIN_SECS: u32 = 1;
/// Largest supported slideshow interval, in seconds.
pub const SLIDESHOW_INTERVAL_MAX_SECS: u32 = 60;

/// Gallery grid density preset. Serialized lowercase (`"s"` / `"m"` / `"l"`)
/// in settings.json, following the `Language` (`"en"` / `"es"`) precedent.
/// The pixel geometry for each preset lives beside the grid renderer
/// (`sh-app` `ui/grid.rs`); core owns only the persisted choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GridSize {
    /// Dense: more columns, smaller cells.
    S,
    /// Current density. The default; reproduces today's geometry exactly.
    #[default]
    M,
    /// Sparse: fewer columns, larger cells.
    L,
}

/// Default for the checkerboard visibility flag: ON (spec: the flag
/// defaults to true; v6 files migrate silently to ON).
fn default_true() -> bool {
    true
}

/// Return the safe default for settings files written before v9.
fn default_slideshow_interval_secs() -> u32 {
    DEFAULT_SLIDESHOW_INTERVAL_SECS
}

/// Versioned settings file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    /// Schema version for future migration.
    pub version: u32,
    /// Name of the active theme JSON file.
    pub theme: String,
    /// Last opened directory, if any.
    pub last_dir: Option<PathBuf>,
    /// Maximum cache size in megabytes.
    pub cache_memory_limit_mb: u32,
    /// Whether to show hidden files in the navigator.
    pub show_hidden_files: bool,
    /// Largest dimension (width or height) decoded before downscaling.
    pub max_decode_dimension: u32,
    /// Gallery sort criterion (V3). `#[serde(default)]` is REQUIRED: `load`
    /// falls back to whole-file defaults on parse failure, so a v1 file
    /// missing this key must still deserialize — otherwise the user's
    /// `last_dir` and friends are wiped on the first V3 run.
    #[serde(default)]
    pub sort_by: SortBy,
    /// Gallery sort direction (V3). Same migration contract as `sort_by`.
    #[serde(default)]
    pub sort_dir: SortDir,
    /// Recent folders, most-recent-first (V3). `#[serde(default)]` is
    /// REQUIRED for the v2 → v3 migration: `load` falls back to whole-file
    /// defaults on parse failure, so a v2 file missing this key must still
    /// deserialize — otherwise the user's `last_dir` is wiped.
    #[serde(default)]
    pub recent_dirs: Vec<PathBuf>,
    /// Rebindable shortcuts (V4). `#[serde(default = "default_keymap")]` is
    /// REQUIRED for the v3 → v4 migration: `load` falls back to whole-file
    /// defaults on parse failure, so a v3 file missing this key must still
    /// deserialize — otherwise the user's `last_dir` and friends are wiped
    /// on the first V4 run. The default fn returns `defaults()`, NOT an
    /// empty map, because every keymap-dispatched action needs a binding.
    #[serde(default = "default_keymap")]
    pub keymap: Keymap,
    /// UI language (V5). `#[serde(default)]` is REQUIRED for the v4 → v5
    /// migration: `load` falls back to whole-file defaults on parse failure,
    /// so a v4 file missing this key must still deserialize — otherwise the
    /// user's `last_dir` and friends are wiped on the first V5 run.
    /// Serialized lowercase (`"en"` / `"es"`) via `Language`'s serde rules.
    #[serde(default)]
    pub language: Language,
    /// Gallery grid density (V6). `#[serde(default)]` is REQUIRED for the
    /// v5 → v6 migration: `load` falls back to whole-file defaults on parse
    /// failure, so a v5 file missing this key must still deserialize —
    /// otherwise the user's `language`, `last_dir`, and friends are wiped
    /// on the first V6 run. Defaults to `M` (today's geometry).
    #[serde(default)]
    pub grid_size: GridSize,
    /// Transparency checkerboard visibility (V7). `#[serde(default =
    /// "default_true")]` is REQUIRED for the v6 → v7 migration: `load`
    /// falls back to whole-file defaults on parse failure, so a v6 file
    /// missing this key must still deserialize — otherwise the user's
    /// stored prefs are wiped on the first V7 run. Defaults to ON;
    /// persisted-only in this slice (no Settings UI row).
    #[serde(default = "default_true")]
    pub checkerboard: bool,
    /// Filmstrip visibility (V8). `#[serde(default = "default_true")]` is
    /// REQUIRED for the v7 → v8 migration: `load` falls back to whole-file
    /// defaults on parse failure, so a v7 file missing this key must still
    /// deserialize — otherwise the user's stored prefs are wiped on the first
    /// V8 run. Defaults to ON; persisted-only in this slice (no Settings UI row).
    #[serde(default = "default_true")]
    pub filmstrip: bool,
    /// Slideshow interval in seconds (V9). `#[serde(default)]` keeps v8 files
    /// loadable while startup upgrades their schema version separately.
    #[serde(default = "default_slideshow_interval_secs")]
    pub slideshow_interval_secs: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: CURRENT_SETTINGS_VERSION,
            theme: "noir-gallery.json".into(),
            last_dir: None,
            cache_memory_limit_mb: 128,
            show_hidden_files: false,
            max_decode_dimension: 8192,
            sort_by: SortBy::Name,
            sort_dir: SortDir::Asc,
            recent_dirs: Vec::new(),
            keymap: default_keymap(),
            language: Language::En,
            grid_size: GridSize::M,
            checkerboard: true,
            filmstrip: true,
            slideshow_interval_secs: DEFAULT_SLIDESHOW_INTERVAL_SECS,
        }
    }
}

impl Settings {
    /// Set the slideshow interval after validating the supported range.
    ///
    /// # Errors
    ///
    /// Returns [`ShImagesError::Config`] when `seconds` is outside the
    /// inclusive `1..=60` range. The existing value remains unchanged.
    pub fn set_slideshow_interval_secs(&mut self, seconds: u32) -> Result<()> {
        validate_slideshow_interval_secs(seconds)?;
        self.slideshow_interval_secs = seconds;
        Ok(())
    }

    fn validate(&self) -> Result<()> {
        validate_slideshow_interval_secs(self.slideshow_interval_secs)
    }
}

fn slideshow_interval_is_valid(seconds: u32) -> bool {
    (SLIDESHOW_INTERVAL_MIN_SECS..=SLIDESHOW_INTERVAL_MAX_SECS).contains(&seconds)
}

fn validate_slideshow_interval_secs(seconds: u32) -> Result<()> {
    if slideshow_interval_is_valid(seconds) {
        Ok(())
    } else {
        Err(ShImagesError::Config(format!(
            "slideshow interval must be between {SLIDESHOW_INTERVAL_MIN_SECS} and {SLIDESHOW_INTERVAL_MAX_SECS} seconds, got {seconds}"
        )))
    }
}

/// Load settings from a path; missing/corrupt file falls back to defaults.
/// v2 → v3 migration: a file whose `recent_dirs` is empty seeds the list
/// from `last_dir`, so a v2 user keeps their Continue target.
/// v3 → v4 migration: a file missing `keymap` deserializes via per-field
/// #[serde(default)] into defaults(); corrupt falls back to whole-file defaults.
/// v4 → v5 migration: a file missing `language` deserializes via per-field
/// #[serde(default)] into `Language::En`; corrupt falls back to whole-file
/// defaults and the file is left untouched until the next save.
/// v5 → v6 migration: a file missing `grid_size` deserializes via per-field
/// `#[serde(default)]` into `GridSize::M`; corrupt falls back to whole-file
/// defaults and the file is left untouched until the next save.
/// v6 → v7 migration: a file missing `checkerboard` deserializes via
/// per-field `#[serde(default = "default_true")]` to ON; corrupt falls
/// back to whole-file defaults and the file is left untouched until the
/// next save.
/// v7 → v8 migration: a file missing `filmstrip` deserializes via per-field
/// #[serde(default = "default_true")] to ON; corrupt falls back to whole-file
/// defaults and the file is left untouched until the next save.
/// v8 → v9 migration: a file missing `slideshow_interval_secs` deserializes
/// to the safe three-second default. Invalid persisted intervals are also
/// replaced with that default so loading never yields a zero-delay timer.
pub fn load(path: &Path) -> Settings {
    let mut s: Settings = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Settings>(&text).ok())
        .unwrap_or_default();
    if s.recent_dirs.is_empty() {
        if let Some(last) = s.last_dir.clone() {
            s.recent_dirs = vec![last];
        }
    }
    if !slideshow_interval_is_valid(s.slideshow_interval_secs) {
        s.slideshow_interval_secs = DEFAULT_SLIDESHOW_INTERVAL_SECS;
    }
    s
}

/// Atomically write settings: `.tmp` + rename.
///
/// # Errors
///
/// Returns [`ShImagesError::Config`] when the slideshow interval is outside
/// the supported `1..=60` range, or an I/O/configuration error when the file
/// cannot be written.
pub fn save(path: &Path, settings: &Settings) -> Result<()> {
    settings.validate()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let text =
        serde_json::to_string_pretty(settings).map_err(|e| ShImagesError::Config(e.to_string()))?;
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigation::{SortBy, SortDir};
    use tempfile::tempdir;

    #[test]
    fn defaults_are_sane() {
        let s = Settings::default();
        assert_eq!(s.version, CURRENT_SETTINGS_VERSION);
        assert_eq!(s.theme, "noir-gallery.json");
        assert_eq!(s.cache_memory_limit_mb, 128);
        assert!(!s.show_hidden_files);
        assert_eq!(s.max_decode_dimension, 8192);
        assert!(s.last_dir.is_none());
        assert!(s.recent_dirs.is_empty());
        assert_eq!(s.sort_by, SortBy::Name);
        assert_eq!(s.sort_dir, SortDir::Asc);
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("settings.json");
        // V3 invariant: last_dir mirrors recent_dirs[0] — constructing the
        // fixture that way makes the load-time seed a no-op, so this test
        // keeps proving the pure serde roundtrip.
        let s = Settings {
            last_dir: Some(PathBuf::from("C:\\Fotos")),
            recent_dirs: vec![PathBuf::from("C:\\Fotos")],
            ..Settings::default()
        };
        save(&p, &s).unwrap();
        assert!(!p.with_extension("json.tmp").exists());
        let loaded = load(&p);
        assert_eq!(loaded, s);
    }

    #[test]
    fn load_missing_returns_defaults() {
        let dir = tempdir().unwrap();
        let loaded = load(&dir.path().join("missing.json"));
        assert_eq!(loaded, Settings::default());
    }

    #[test]
    fn load_corrupt_returns_defaults() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("settings.json");
        std::fs::write(&p, "{ not json").unwrap();
        let loaded = load(&p);
        assert_eq!(loaded, Settings::default());
    }

    #[test]
    fn save_creates_parent_dirs() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("a/b/c/settings.json");
        save(&p, &Settings::default()).unwrap();
        assert!(p.exists());
        assert!(!p.with_extension("json.tmp").exists()); // tmp renamed away
    }

    // Proves atomic rename replaces an existing destination on Windows.
    #[test]
    fn save_overwrites_existing_file() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("settings.json");
        save(&p, &Settings::default()).unwrap();
        let changed = Settings {
            theme: "dark-clinical.json".into(),
            ..Settings::default()
        };
        save(&p, &changed).unwrap();
        assert_eq!(load(&p), changed);
    }

    /// Restores the process working directory on drop, even during a panic.
    struct CwdGuard(std::path::PathBuf);

    impl CwdGuard {
        fn enter(dir: &std::path::Path) -> Self {
            let prev = std::env::current_dir().expect("current dir readable in tests");
            std::env::set_current_dir(dir).expect("test cwd swap succeeds");
            Self(prev)
        }
    }

    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.0);
        }
    }

    #[test]
    fn save_with_bare_filename_writes_to_cwd() {
        let dir = tempdir().unwrap();
        let _guard = CwdGuard::enter(dir.path());
        save(Path::new("settings.json"), &Settings::default()).unwrap();
        let loaded = load(&dir.path().join("settings.json"));
        assert_eq!(loaded, Settings::default());
    }

    // ── V3: sort settings + v1 → v2 serde-default migration ──

    #[test]
    fn v1_file_without_sort_keys_loads_with_name_asc_and_keeps_last_dir() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("settings.json");
        // A real V1 file as shipped in V2 — missing sort_by/sort_dir entirely.
        // CRITICAL contract: `load` falls back to defaults on the WHOLE file,
        // so the new keys MUST deserialize from an old file via per-field
        // `#[serde(default)]` — otherwise the user loses `last_dir` on the
        // first V3 run.
        std::fs::write(
            &p,
            r#"{
                "version": 1,
                "theme": "light-clean.json",
                "last_dir": "C:\\Fotos",
                "cache_memory_limit_mb": 128,
                "show_hidden_files": false,
                "max_decode_dimension": 8192
            }"#,
        )
        .unwrap();
        let s = load(&p);
        assert_eq!(s.version, 1); // loaded as-is; bumped on next save
        assert_eq!(s.sort_by, SortBy::Name);
        assert_eq!(s.sort_dir, SortDir::Asc);
        assert_eq!(s.last_dir, Some(PathBuf::from("C:\\Fotos")));
    }

    #[test]
    fn sort_settings_roundtrip() {
        let s = Settings {
            sort_by: SortBy::Size,
            sort_dir: SortDir::Desc,
            ..Settings::default()
        };
        let dir = tempdir().unwrap();
        let p = dir.path().join("settings.json");
        save(&p, &s).unwrap();
        let loaded = load(&p);
        assert_eq!(loaded.sort_by, SortBy::Size);
        assert_eq!(loaded.sort_dir, SortDir::Desc);
    }

    #[test]
    fn default_settings_version_is_current_with_name_asc() {
        let s = Settings::default();
        assert_eq!(s.version, CURRENT_SETTINGS_VERSION);
        assert_eq!(s.sort_by, SortBy::Name);
        assert_eq!(s.sort_dir, SortDir::Asc);
        // Older-binary interop: last_dir still exists on the default.
        assert!(s.recent_dirs.is_empty());
        assert_eq!(s.last_dir, None);
    }

    #[test]
    fn sort_keys_serialize_as_snake_case_words() {
        // The on-disk format is pinned: human-readable words, not enum names.
        let s = Settings {
            sort_by: SortBy::Type,
            sort_dir: SortDir::Desc,
            ..Settings::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""sort_by":"type""#), "got: {json}");
        assert!(json.contains(r#""sort_dir":"desc""#), "got: {json}");
    }

    // ── V3: recent-folders settings + v2 → v3 last_dir-seed migration ──

    #[test]
    fn v2_file_with_only_last_dir_seeds_recent_dirs() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("settings.json");
        // A real V2 file as shipped by the sort engine — no recent_dirs key.
        // Contract: `#[serde(default)]` must let it deserialize, and `load`
        // seeds the recents list from last_dir so nothing is lost.
        std::fs::write(
            &p,
            r#"{
                "version": 2,
                "theme": "light-clean.json",
                "last_dir": "C:\\Fotos",
                "cache_memory_limit_mb": 128,
                "show_hidden_files": false,
                "max_decode_dimension": 8192,
                "sort_by": "name",
                "sort_dir": "asc"
            }"#,
        )
        .unwrap();
        let s = load(&p);
        assert_eq!(s.version, 2); // read as-is; bumped on next save
        assert_eq!(s.last_dir, Some(PathBuf::from("C:\\Fotos")));
        assert_eq!(s.recent_dirs, vec![PathBuf::from("C:\\Fotos")]);
    }

    #[test]
    fn v3_recent_dirs_roundtrip() {
        let s = Settings {
            recent_dirs: vec![
                PathBuf::from("C:\\b"),
                PathBuf::from("C:\\a"),
                PathBuf::from("C:\\c"),
            ],
            last_dir: Some(PathBuf::from("C:\\b")),
            ..Settings::default()
        };
        let dir = tempdir().unwrap();
        let p = dir.path().join("settings.json");
        save(&p, &s).unwrap();
        let loaded = load(&p);
        assert_eq!(loaded.recent_dirs, s.recent_dirs);
        assert_eq!(loaded.last_dir, Some(PathBuf::from("C:\\b")));
    }

    #[test]
    fn v3_file_with_empty_recents_and_no_last_dir_stays_empty() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("settings.json");
        // Hand-cleared recents must not re-seed from a last_dir that was
        // also cleared (both keys absent/defaulted).
        std::fs::write(
            &p,
            r#"{
                "version": 3,
                "theme": "light-clean.json",
                "last_dir": null,
                "recent_dirs": [],
                "cache_memory_limit_mb": 128,
                "show_hidden_files": false,
                "max_decode_dimension": 8192,
                "sort_by": "name",
                "sort_dir": "asc"
            }"#,
        )
        .unwrap();
        let s = load(&p);
        assert!(s.recent_dirs.is_empty());
        assert_eq!(s.last_dir, None);
    }

    #[test]
    fn v3_file_without_keymap_loads_with_defaults_and_keeps_last_dir() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("settings.json");
        std::fs::write(
            &p,
            r#"{
            "version": 3,
            "theme": "light-clean.json",
            "last_dir": "C:\\Fotos",
            "cache_memory_limit_mb": 128,
            "show_hidden_files": false,
            "max_decode_dimension": 8192,
            "sort_by": "name",
            "sort_dir": "asc",
            "recent_dirs": ["C:\\Fotos"]
        }"#,
        )
        .unwrap();
        let s = load(&p);
        assert_eq!(s.version, 3);
        assert_eq!(s.last_dir, Some(PathBuf::from("C:\\Fotos")));
        assert_eq!(s.keymap, crate::keymap::defaults());
    }

    #[test]
    fn default_settings_version_is_current_with_default_keymap() {
        let s = Settings::default();
        assert_eq!(s.version, CURRENT_SETTINGS_VERSION);
        assert_eq!(s.keymap, crate::keymap::defaults());
    }

    #[test]
    fn v4_keymap_roundtrip() {
        let mut s = Settings::default();
        s.keymap.insert(
            "toggle-slideshow".into(),
            crate::keymap::KeyBinding {
                ctrl: true,
                shift: false,
                alt: false,
                platform: false,
                key: "k".into(),
            },
        );
        let dir = tempdir().unwrap();
        let p = dir.path().join("settings.json");
        save(&p, &s).unwrap();
        let loaded = load(&p);
        assert_eq!(loaded.keymap.get("toggle-slideshow").unwrap().key, "k");
        assert_eq!(loaded.version, CURRENT_SETTINGS_VERSION);
    }

    // ── V5: language setting + v4 → v5 migration ──

    // ── V5: language setting + v4 → v5 migration ──

    #[test]
    fn fresh_settings_default_to_english() {
        assert_eq!(Settings::default().language, crate::i18n::Language::En);
    }

    #[test]
    fn v4_file_without_language_loads_as_english_with_prefs_intact() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("settings.json");
        // A real V4 file as shipped by the keymap engine — no language key.
        // Contract (mirrors the v3 → v4 migration): per-field
        // `#[serde(default)]` must let it deserialize, so the user's
        // `last_dir`, theme, and keymap survive with language defaulting
        // to English.
        std::fs::write(
            &p,
            r#"{
                "version": 4,
                "theme": "light-clean.json",
                "last_dir": "C:\\Fotos",
                "cache_memory_limit_mb": 128,
                "show_hidden_files": false,
                "max_decode_dimension": 8192,
                "sort_by": "name",
                "sort_dir": "asc",
                "recent_dirs": ["C:\\Fotos"],
                "keymap": {}
            }"#,
        )
        .unwrap();
        let s = load(&p);
        assert_eq!(s.version, 4); // read as-is; bumped on next save
        assert_eq!(s.language, crate::i18n::Language::En);
        assert_eq!(s.last_dir, Some(PathBuf::from("C:\\Fotos")));
        assert_eq!(s.theme, "light-clean.json");
    }

    #[test]
    fn v5_spanish_settings_roundtrip() {
        let s = Settings {
            language: crate::i18n::Language::Es,
            ..Settings::default()
        };
        let dir = tempdir().unwrap();
        let p = dir.path().join("settings.json");
        save(&p, &s).unwrap();
        let loaded = load(&p);
        assert_eq!(loaded.language, crate::i18n::Language::Es);
        assert_eq!(loaded.version, CURRENT_SETTINGS_VERSION);
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""language":"es""#), "got: {json}");
    }

    #[test]
    fn corrupt_file_returns_english_defaults_and_stays_untouched_until_save() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("settings.json");
        std::fs::write(&p, "{ not json").unwrap();
        let before = std::fs::read(&p).unwrap();
        let loaded = load(&p);
        assert_eq!(loaded, Settings::default());
        assert_eq!(loaded.language, crate::i18n::Language::En);
        assert_eq!(std::fs::read(&p).unwrap(), before);
    }

    // ── V6: grid-size setting + v5 → v6 migration ──

    #[test]
    fn grid_size_default_is_m() {
        assert_eq!(GridSize::default(), GridSize::M);
    }

    #[test]
    fn grid_size_has_exactly_three_variants() {
        // Exhaustive match with no wildcard: a fourth variant breaks
        // compilation here.
        for size in [GridSize::S, GridSize::M, GridSize::L] {
            let label = match size {
                GridSize::S => "s",
                GridSize::M => "m",
                GridSize::L => "l",
            };
            assert!(!label.is_empty());
        }
    }

    #[test]
    fn grid_size_serde_lowercase_roundtrip() {
        // The on-disk format is pinned: single lowercase letters, following
        // the `Language` (`"en"`/`"es"`) precedent.
        assert_eq!(serde_json::to_string(&GridSize::S).unwrap(), r#""s""#);
        assert_eq!(serde_json::to_string(&GridSize::M).unwrap(), r#""m""#);
        assert_eq!(serde_json::to_string(&GridSize::L).unwrap(), r#""l""#);
        assert_eq!(
            serde_json::from_str::<GridSize>(r#""s""#).unwrap(),
            GridSize::S
        );
        assert_eq!(
            serde_json::from_str::<GridSize>(r#""m""#).unwrap(),
            GridSize::M
        );
        assert_eq!(
            serde_json::from_str::<GridSize>(r#""l""#).unwrap(),
            GridSize::L
        );
    }

    #[test]
    fn default_settings_version_is_current_with_grid_size_m() {
        let s = Settings::default();
        assert_eq!(s.version, CURRENT_SETTINGS_VERSION);
        assert_eq!(s.grid_size, GridSize::M);
    }

    #[test]
    fn v5_file_without_grid_size_loads_as_m_with_prefs_intact() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("settings.json");
        // A real V5 file as shipped by the language engine — no grid_size key.
        // Contract (mirrors the v4 → v5 migration): per-field
        // `#[serde(default)]` must let it deserialize, so the user's
        // `language`, `last_dir`, theme, and keymap survive with the grid
        // defaulting to M.
        std::fs::write(
            &p,
            r#"{
                "version": 5,
                "theme": "light-clean.json",
                "last_dir": "C:\\Fotos",
                "cache_memory_limit_mb": 128,
                "show_hidden_files": false,
                "max_decode_dimension": 8192,
                "sort_by": "name",
                "sort_dir": "asc",
                "recent_dirs": ["C:\\Fotos"],
                "keymap": {},
                "language": "es"
            }"#,
        )
        .unwrap();
        let s = load(&p);
        assert_eq!(s.version, 5); // read as-is; bumped on next save
        assert_eq!(s.grid_size, GridSize::M);
        assert_eq!(s.language, crate::i18n::Language::Es);
        assert_eq!(s.last_dir, Some(PathBuf::from("C:\\Fotos")));
        assert_eq!(s.theme, "light-clean.json");
    }

    #[test]
    fn v6_grid_size_l_roundtrip() {
        let s = Settings {
            grid_size: GridSize::L,
            ..Settings::default()
        };
        let dir = tempdir().unwrap();
        let p = dir.path().join("settings.json");
        save(&p, &s).unwrap();
        let loaded = load(&p);
        assert_eq!(loaded.grid_size, GridSize::L);
        assert_eq!(loaded.version, CURRENT_SETTINGS_VERSION);
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""grid_size":"l""#), "got: {json}");
    }

    #[test]
    fn v4_file_migrates_with_grid_size_m_and_english_defaults() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("settings.json");
        // A real V4 file — no language key, no grid_size key. Both default
        // via per-field `#[serde(default)]` while stored prefs survive.
        std::fs::write(
            &p,
            r#"{
                "version": 4,
                "theme": "light-clean.json",
                "last_dir": "C:\\Fotos",
                "cache_memory_limit_mb": 128,
                "show_hidden_files": false,
                "max_decode_dimension": 8192,
                "sort_by": "name",
                "sort_dir": "asc",
                "recent_dirs": ["C:\\Fotos"],
                "keymap": {}
            }"#,
        )
        .unwrap();
        let s = load(&p);
        assert_eq!(s.version, 4); // read as-is; bumped on next save
        assert_eq!(s.grid_size, GridSize::M);
        assert_eq!(s.language, crate::i18n::Language::En);
        assert_eq!(s.last_dir, Some(PathBuf::from("C:\\Fotos")));
    }

    #[test]
    fn corrupt_file_returns_grid_size_m_and_stays_untouched_until_save() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("settings.json");
        std::fs::write(&p, "{ not json").unwrap();
        let before = std::fs::read(&p).unwrap();
        let loaded = load(&p);
        assert_eq!(loaded.grid_size, GridSize::M);
        assert_eq!(loaded, Settings::default());
        assert_eq!(std::fs::read(&p).unwrap(), before);
    }

    // ── V7: checkerboard visibility setting + v6 → v7 migration ──

    #[test]
    fn checkerboard_defaults_on() {
        let s = Settings::default();
        assert!(s.checkerboard);
        assert_eq!(s.version, CURRENT_SETTINGS_VERSION);
    }

    #[test]
    fn v6_file_without_checkerboard_loads_true_with_prefs_intact() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("settings.json");
        // A real V6 file as shipped by the grid-size engine — no
        // checkerboard key. Contract (mirrors the v5 → v6 migration):
        // per-field `#[serde(default = "default_true")]` must let it
        // deserialize, so stored prefs survive with the flag defaulting
        // ON.
        std::fs::write(
            &p,
            r#"{
                "version": 6,
                "theme": "light-clean.json",
                "last_dir": "C:\\Fotos",
                "cache_memory_limit_mb": 128,
                "show_hidden_files": false,
                "max_decode_dimension": 8192,
                "sort_by": "name",
                "sort_dir": "asc",
                "recent_dirs": ["C:\\Fotos"],
                "keymap": {},
                "language": "es",
                "grid_size": "l"
            }"#,
        )
        .unwrap();
        let s = load(&p);
        assert_eq!(s.version, 6); // read as-is; bumped on next save
        assert!(s.checkerboard);
        assert_eq!(s.grid_size, GridSize::L);
        assert_eq!(s.language, crate::i18n::Language::Es);
        assert_eq!(s.last_dir, Some(PathBuf::from("C:\\Fotos")));
    }

    #[test]
    fn checkerboard_explicit_off_roundtrips() {
        let s = Settings {
            checkerboard: false,
            ..Settings::default()
        };
        let dir = tempdir().unwrap();
        let p = dir.path().join("settings.json");
        save(&p, &s).unwrap();
        let loaded = load(&p);
        assert!(!loaded.checkerboard);
        assert_eq!(loaded.version, CURRENT_SETTINGS_VERSION);
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""checkerboard":false"#), "got: {json}");
    }

    #[test]
    fn corrupt_file_returns_checkerboard_on() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("settings.json");
        std::fs::write(&p, "{ not json").unwrap();
        let loaded = load(&p);
        assert!(loaded.checkerboard);
        assert_eq!(loaded, Settings::default());
    }

    // ── V8: filmstrip visibility setting + v7 → v8 migration ──

    #[test]
    fn filmstrip_defaults_on_with_current_version() {
        let s = Settings::default();
        assert!(s.filmstrip);
        assert_eq!(s.version, CURRENT_SETTINGS_VERSION);
    }

    #[test]
    fn v7_file_without_filmstrip_loads_true_with_prefs_intact() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("settings.json");
        // A real V7 file as shipped by the checkerboard engine — no
        // filmstrip key. Contract (mirrors the v6 → v7 migration):
        // per-field `#[serde(default = "default_true")]` must let it
        // deserialize, so stored prefs survive with the flag defaulting
        // ON.
        std::fs::write(
            &p,
            r#"{
                "version": 7,
                "theme": "light-clean.json",
                "last_dir": "C:\\Fotos",
                "cache_memory_limit_mb": 128,
                "show_hidden_files": false,
                "max_decode_dimension": 8192,
                "sort_by": "name",
                "sort_dir": "asc",
                "recent_dirs": ["C:\\Fotos"],
                "keymap": {},
                "language": "es",
                "grid_size": "l",
                "checkerboard": false
            }"#,
        )
        .unwrap();
        let s = load(&p);
        assert_eq!(s.version, 7); // read as-is; bumped on next save
        assert!(s.filmstrip);
        assert!(!s.checkerboard);
        assert_eq!(s.grid_size, GridSize::L);
        assert_eq!(s.language, crate::i18n::Language::Es);
        assert_eq!(s.last_dir, Some(PathBuf::from("C:\\Fotos")));
    }

    #[test]
    fn filmstrip_explicit_off_roundtrips() {
        let s = Settings {
            filmstrip: false,
            ..Settings::default()
        };
        let dir = tempdir().unwrap();
        let p = dir.path().join("settings.json");
        save(&p, &s).unwrap();
        let loaded = load(&p);
        assert!(!loaded.filmstrip);
        assert_eq!(loaded.version, CURRENT_SETTINGS_VERSION);
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""filmstrip":false"#), "got: {json}");
    }

    #[test]
    fn corrupt_file_returns_filmstrip_on() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("settings.json");
        std::fs::write(&p, "{ not json").unwrap();
        let loaded = load(&p);
        assert!(loaded.filmstrip);
        assert_eq!(loaded, Settings::default());
    }

    // ── V9: slideshow interval setting + v8 → v9 migration ──

    #[test]
    fn slideshow_interval_defaults_to_three_with_current_version() {
        let settings = Settings::default();

        assert_eq!(settings.version, CURRENT_SETTINGS_VERSION);
        assert_eq!(
            settings.slideshow_interval_secs,
            DEFAULT_SLIDESHOW_INTERVAL_SECS
        );
    }

    #[test]
    fn v8_file_without_slideshow_interval_loads_three_with_prefs_intact() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{
                "version": 8,
                "theme": "light-clean.json",
                "last_dir": "C:\\Fotos",
                "cache_memory_limit_mb": 128,
                "show_hidden_files": false,
                "max_decode_dimension": 8192,
                "sort_by": "name",
                "sort_dir": "asc",
                "recent_dirs": ["C:\\Fotos"],
                "keymap": {},
                "language": "es",
                "grid_size": "l",
                "checkerboard": false,
                "filmstrip": false
            }"#,
        )
        .unwrap();

        let settings = load(&path);

        assert_eq!(settings.version, 8);
        assert_eq!(
            settings.slideshow_interval_secs,
            DEFAULT_SLIDESHOW_INTERVAL_SECS
        );
        assert_eq!(settings.theme, "light-clean.json");
        assert_eq!(settings.last_dir, Some(PathBuf::from("C:\\Fotos")));
        assert_eq!(settings.language, crate::i18n::Language::Es);
        assert_eq!(settings.grid_size, GridSize::L);
        assert!(!settings.checkerboard);
        assert!(!settings.filmstrip);
    }

    #[test]
    fn slideshow_interval_boundaries_roundtrip() {
        let dir = tempdir().unwrap();

        for seconds in [SLIDESHOW_INTERVAL_MIN_SECS, SLIDESHOW_INTERVAL_MAX_SECS] {
            let mut settings = Settings::default();
            settings
                .set_slideshow_interval_secs(seconds)
                .expect("boundary interval must be valid");
            let path = dir.path().join(format!("settings-{seconds}.json"));

            save(&path, &settings).expect("valid settings must save");

            assert_eq!(load(&path).slideshow_interval_secs, seconds);
        }
    }

    #[test]
    fn slideshow_interval_rejects_out_of_range_values() {
        let dir = tempdir().unwrap();
        let mut settings = Settings::default();

        for invalid in [
            SLIDESHOW_INTERVAL_MIN_SECS - 1,
            SLIDESHOW_INTERVAL_MAX_SECS + 1,
        ] {
            assert!(matches!(
                settings.set_slideshow_interval_secs(invalid),
                Err(ShImagesError::Config(_))
            ));
            assert_eq!(
                settings.slideshow_interval_secs,
                DEFAULT_SLIDESHOW_INTERVAL_SECS
            );

            let invalid_settings = Settings {
                slideshow_interval_secs: invalid,
                ..Settings::default()
            };
            assert!(matches!(
                save(
                    &dir.path().join(format!("invalid-{invalid}.json")),
                    &invalid_settings
                ),
                Err(ShImagesError::Config(_))
            ));
        }
    }

    #[test]
    fn invalid_persisted_slideshow_interval_falls_back_to_default() {
        let dir = tempdir().unwrap();

        for invalid in [
            SLIDESHOW_INTERVAL_MIN_SECS - 1,
            SLIDESHOW_INTERVAL_MAX_SECS + 1,
        ] {
            let stored = Settings {
                slideshow_interval_secs: invalid,
                ..Settings::default()
            };
            let path = dir.path().join(format!("invalid-{invalid}.json"));
            std::fs::write(
                &path,
                serde_json::to_string(&stored).expect("settings fixture must serialize"),
            )
            .unwrap();

            assert_eq!(
                load(&path).slideshow_interval_secs,
                DEFAULT_SLIDESHOW_INTERVAL_SECS
            );
        }
    }
}
