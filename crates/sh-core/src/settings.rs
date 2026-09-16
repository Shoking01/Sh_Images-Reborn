//! User settings schema, defaults, and atomic persistence.

use crate::errors::{Result, ShImagesError};
use crate::keymap::{defaults as default_keymap, Keymap};
use crate::navigation::{SortBy, SortDir};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 4,
            theme: "noir-gallery.json".into(),
            last_dir: None,
            cache_memory_limit_mb: 128,
            show_hidden_files: false,
            max_decode_dimension: 8192,
            sort_by: SortBy::Name,
            sort_dir: SortDir::Asc,
            recent_dirs: Vec::new(),
            keymap: default_keymap(),
        }
    }
}

/// Load settings from a path; missing/corrupt file falls back to defaults.
/// v2 → v3 migration: a file whose `recent_dirs` is empty seeds the list
/// from `last_dir`, so a v2 user keeps their Continue target.
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
    s
}

/// Atomically write settings: `.tmp` + rename.
pub fn save(path: &Path, settings: &Settings) -> Result<()> {
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
        assert_eq!(s.version, 4);
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
    fn default_settings_version_is_4_with_name_asc() {
        let s = Settings::default();
        assert_eq!(s.version, 4);
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
    fn default_settings_version_is_4_with_default_keymap() {
        let s = Settings::default();
        assert_eq!(s.version, 4);
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
        assert_eq!(loaded.version, 4);
    }
}
