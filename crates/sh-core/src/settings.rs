//! User settings schema, defaults, and atomic persistence.

use crate::errors::{Result, ShImagesError};
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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            theme: "deep-neutral.json".into(),
            last_dir: None,
            cache_memory_limit_mb: 128,
            show_hidden_files: false,
            max_decode_dimension: 8192,
        }
    }
}

/// Load settings from a path; missing/corrupt file falls back to defaults.
pub fn load(path: &Path) -> Settings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
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
    use tempfile::tempdir;

    #[test]
    fn defaults_are_sane() {
        let s = Settings::default();
        assert_eq!(s.version, 1);
        assert_eq!(s.theme, "deep-neutral.json");
        assert_eq!(s.cache_memory_limit_mb, 128);
        assert!(!s.show_hidden_files);
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("settings.json");
        let s = Settings {
            last_dir: Some(PathBuf::from("C:\\Fotos")),
            ..Settings::default()
        };
        save(&p, &s).unwrap();
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

    // ADDITIONAL TEST REQUIRED (bounded adaptation, see note 2):
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
}
