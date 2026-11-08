#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod batch;
mod encode;
mod error;
mod metadata;
mod orient;
mod pipeline;
mod preset;
mod scan;

pub use batch::{BatchEvent, BatchSummary, default_jobs, run_batch};
pub use error::{ProcessError, ScanError, SettingsError};
pub use pipeline::{FileReport, Warning, process_file, process_to_memory};
pub use preset::{Preset, Settings};
pub use scan::scan_dir;
