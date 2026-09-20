//! Integration tests: transparency verdicts against static fixture files.
//!
//! The fixtures live in `tests/fixtures/` (AGENTS.md §8.2): a transparent
//! 1x1 PNG, an opaque 1x1 PNG, and a tiny JPEG. They pin the on-disk byte
//! shapes the unit tests generate at runtime, so a decoder regression on
//! real files is caught here.

use sh_core::decode::{has_alpha_rgba, load, probe_has_alpha};
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn transparent_1x1_fixture_probes_true_and_scans_true() {
    let p = fixture("transparent_1x1.png");
    assert_eq!(probe_has_alpha(&p).unwrap(), true);
    let decoded = load(&p).unwrap();
    assert!(has_alpha_rgba(&decoded));
}

#[test]
fn opaque_1x1_fixture_probes_false() {
    let p = fixture("opaque_1x1.png");
    assert_eq!(probe_has_alpha(&p).unwrap(), false);
}

#[test]
fn tiny_jpeg_fixture_probes_false() {
    let p = fixture("tiny.jpg");
    assert_eq!(probe_has_alpha(&p).unwrap(), false);
}
