#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Sh_Images core library.
//!
//! Pure business logic with NO GPUI dependency. The compiler enforces this:
//! `sh-core`'s Cargo.toml does not declare `gpui`, so this crate can never
//! import UI/platform code. Everything here is headless-testable.

pub mod batch;
pub mod cache;
pub mod crop;
pub mod decode;
pub mod errors;
pub mod navigation;
pub mod recent;
pub mod settings;
pub mod theme;
pub mod transform;
