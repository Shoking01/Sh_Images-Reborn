//! Decode benchmarks (cargo bench -p sh-core).
//!
//! Measures `decode::load` end-to-end (open + header sniff + full pixel
//! decode + RGBA conversion) against the committed 1920×1080 PNG fixture.
//! `decode::load` caps the longest side at 8192px, so a 1080p fixture
//! exercises the pure decode path with no resize.

use criterion::{criterion_group, criterion_main, Criterion};
use sh_core::decode;
use std::path::PathBuf;

fn bench_decode_1080p(c: &mut Criterion) {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bench_1080p.png");
    c.bench_function("decode_1080p", |b| {
        b.iter(|| decode::load(&fixture).expect("bench fixture must decode"))
    });
}

criterion_group!(benches, bench_decode_1080p);
criterion_main!(benches);
