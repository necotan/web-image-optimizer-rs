use std::io;
use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ScanError {
    #[error("folder not found: {0}")]
    NotFound(PathBuf),
    #[error("cannot read folder {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SettingsError {
    #[error("long edge must be 1 or more: {0}")]
    LongEdge(u32),
    #[error("quality must be between 1 and 100: {0}")]
    Quality(u8),
}

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error("cannot read file: {0}")]
    Read(#[source] io::Error),
    #[error("cannot decode JPEG: {0}")]
    Decode(String),
    #[error("unsupported color space: {0}")]
    UnsupportedColorSpace(String),
    #[error("image is too large: {width}x{height}")]
    ImageTooLarge { width: u32, height: u32 },
    #[error("cannot resize image: {0}")]
    Resize(String),
    #[error("cannot encode JPEG: {0}")]
    Encode(String),
    #[error("cannot write file: {0}")]
    Write(#[source] io::Error),
    #[error("internal error: {0}")]
    Panic(String),
}
