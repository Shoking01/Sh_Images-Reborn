//! Folder scanning, natural sorting, and circular navigation.

use crate::errors::{Result, ShImagesError};
use std::path::{Path, PathBuf};

/// A list of image paths with a current position.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageList {
    /// All image paths in the scanned directory, naturally sorted.
    pub paths: Vec<PathBuf>,
    /// Index of the current image in `paths`.
    pub current: usize,
}

/// Resolve a path into an ordered image list, placing `path` at `current`.
///
/// Scans the parent directory, filters to supported image extensions,
/// natural-sorts the result, and locates `path` inside it.
pub fn resolve(path: &Path) -> Result<ImageList> {
    let parent = path
        .parent()
        .ok_or_else(|| ShImagesError::NotAFile(path.display().to_string()))?;
    let mut paths: Vec<PathBuf> = std::fs::read_dir(parent)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|p| p.is_file() && is_supported(p))
        .collect();
    paths.sort_by(|a, b| natord::compare(&path_key(a), &path_key(b)));
    let current = paths
        .iter()
        .position(|p| p == path)
        .ok_or_else(|| ShImagesError::NotAFile(path.display().to_string()))?;
    Ok(ImageList { paths, current })
}

/// Returns `next` index with circular wrap-around.
pub fn next_index(list: &ImageList) -> usize {
    if list.paths.is_empty() {
        0
    } else {
        (list.current + 1) % list.paths.len()
    }
}

/// Returns `prev` index with circular wrap-around.
pub fn prev_index(list: &ImageList) -> usize {
    if list.paths.is_empty() {
        0
    } else {
        (list.current + list.paths.len() - 1) % list.paths.len()
    }
}

/// Returns true if `path` has a supported image extension.
pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tif" | "tiff"
            )
        })
        .unwrap_or(false)
}

/// Key used for natural sort (file name lowercase, or full path fallback).
fn path_key(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_lowercase())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, b"x").unwrap();
        p
    }

    #[test]
    fn resolve_scans_parent_and_orders_naturally() {
        let dir = tempdir().unwrap();
        touch(dir.path(), "img10.png");
        touch(dir.path(), "img2.png");
        touch(dir.path(), "img1.png");
        touch(dir.path(), "notes.txt");

        let target = dir.path().join("img2.png");
        let list = resolve(&target).unwrap();

        assert_eq!(list.paths.len(), 3);
        assert_eq!(list.paths[0].file_name().unwrap(), "img1.png");
        assert_eq!(list.paths[1].file_name().unwrap(), "img2.png");
        assert_eq!(list.paths[2].file_name().unwrap(), "img10.png");
        assert_eq!(list.current, 1);
    }

    #[test]
    fn filters_non_image_files() {
        let dir = tempdir().unwrap();
        touch(dir.path(), "a.png");
        touch(dir.path(), "b.txt");
        touch(dir.path(), "c.jpg");
        let list = resolve(&dir.path().join("a.png")).unwrap();
        assert_eq!(list.paths.len(), 2);
    }

    #[test]
    fn handles_unicode_filenames() {
        let dir = tempdir().unwrap();
        touch(dir.path(), "foto áéí.png");
        touch(dir.path(), "foto ü.png");
        let list = resolve(&dir.path().join("foto áéí.png")).unwrap();
        assert_eq!(list.paths.len(), 2);
        assert_eq!(list.current, 0);
    }

    #[test]
    fn next_and_prev_wrap_circularly() {
        let dir = tempdir().unwrap();
        touch(dir.path(), "a.png");
        touch(dir.path(), "b.png");
        touch(dir.path(), "c.png");

        let mut list = resolve(&dir.path().join("a.png")).unwrap();
        assert_eq!(next_index(&list), 1);
        list.current = 1;
        assert_eq!(next_index(&list), 2);
        list.current = 2;
        assert_eq!(next_index(&list), 0); // wraps

        list.current = 0;
        assert_eq!(prev_index(&list), 2); // wraps backward
        list.current = 2;
        assert_eq!(prev_index(&list), 1);
    }

    #[test]
    fn missing_path_errors() {
        let dir = tempdir().unwrap();
        let err = resolve(&dir.path().join("nope.png")).unwrap_err();
        assert!(matches!(err, ShImagesError::NotAFile(_)));
    }

    #[test]
    fn is_supported_case_insensitive() {
        assert!(is_supported(Path::new("x.PNG")));
        assert!(is_supported(Path::new("x.JpEg")));
        assert!(is_supported(Path::new("x.tiff")));
        assert!(!is_supported(Path::new("x.txt")));
        assert!(!is_supported(Path::new("x")));
    }
}
