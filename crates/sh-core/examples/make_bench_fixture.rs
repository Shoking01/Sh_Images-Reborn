//! Regenerates the benchmark fixture `tests/fixtures/bench_1080p.png`.
//!
//! Run once after cloning (or when the fixture must change):
//!
//! ```sh
//! cargo run -p sh-core --example make_bench_fixture
//! ```
//!
//! The fixture is a 1920×1080 horizontal gradient: every row is identical, so
//! PNG's "up" row filter zeroes the deltas and deflate keeps the file tiny
//! (a few KB — within the AGENTS.md §8.2 fixture size budget). Decode still
//! exercises the full pipeline: inflate + row unfiltering of all 1080 rows +
//! RGBA conversion of 8.3MB of output.

use image::{Rgb, RgbImage};
use std::path::PathBuf;

fn main() {
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join("bench_1080p.png");
    std::fs::create_dir_all(out.parent().expect("fixture path has a parent"))
        .expect("create fixtures dir");
    let img: RgbImage = RgbImage::from_fn(1920, 1080, |x, _y| {
        let r = (x * 255 / 1919) as u8;
        let g = (x % 256) as u8;
        let b = (255 - x % 256) as u8;
        Rgb([r, g, b])
    });
    img.save(&out).expect("write bench fixture");
    println!("wrote {}", out.display());
}
