//! Unified error type for Sh_Images.

use thiserror::Error;

/// Top-level error for every fallible operation in Sh_Images.
#[derive(Error, Debug)]
pub enum ShImagesError {
    /// An image could not be decoded.
    #[error("decode error: {0}")]
    Decode(#[from] image::ImageError),

    /// An I/O operation failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The file format is not supported.
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),

    /// Configuration is invalid or unreadable.
    #[error("config error: {0}")]
    Config(String),

    /// A theme is invalid.
    #[error("theme error: {0}")]
    Theme(String),

    /// The path does not point to a file.
    #[error("not a file: {0}")]
    NotAFile(String),

    /// Any other failure.
    #[error("unknown error: {0}")]
    Unknown(String),
}

/// Convenience alias for `Result<T, ShImagesError>`.
pub type Result<T> = std::result::Result<T, ShImagesError>;
