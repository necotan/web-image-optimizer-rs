use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use rayon::ThreadPoolBuilder;
use rayon::prelude::*;

use crate::encode::panic_message;
use crate::error::ProcessError;
use crate::pipeline::{FileReport, process_file};
use crate::preset::Settings;

#[derive(Debug)]
pub enum BatchEvent {
    Started { index: usize },
    Finished { index: usize, report: FileReport },
    Failed { index: usize, error: ProcessError },
    Cancelled,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BatchSummary {
    pub succeeded: usize,
    pub failed: usize,
    /// キャンセルで開始しなかったファイルの数
    pub skipped: usize,
    pub input_bytes: u64,
    pub output_bytes: u64,
}

/// メモリのピークを抑えるため 4 までに制限する (F-11)
pub fn default_jobs() -> usize {
    thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(4)
}

pub fn run_batch(
    files: &[PathBuf],
    out_dir: &Path,
    s: &Settings,
    jobs: usize,
    cancel: &AtomicBool,
    on_event: impl Fn(BatchEvent) + Sync,
) -> BatchSummary {
    let summary = Mutex::new(BatchSummary::default());
    let update = |apply: &dyn Fn(&mut BatchSummary)| {
        let mut guard = summary.lock().unwrap_or_else(|e| e.into_inner());
        apply(&mut guard);
    };

    let run_one = |index: usize, input: &PathBuf| {
        if cancel.load(Ordering::SeqCst) {
            update(&|sum| sum.skipped += 1);
            return;
        }
        on_event(BatchEvent::Started { index });
        match process_guarded(input, out_dir, s) {
            Ok(report) => {
                update(&|sum| {
                    sum.succeeded += 1;
                    sum.input_bytes += report.input_bytes;
                    sum.output_bytes += report.output_bytes;
                });
                on_event(BatchEvent::Finished { index, report });
            }
            Err(error) => {
                update(&|sum| sum.failed += 1);
                on_event(BatchEvent::Failed { index, error });
            }
        }
    };

    match ThreadPoolBuilder::new().num_threads(jobs.max(1)).build() {
        Ok(pool) => pool.install(|| {
            files
                .par_iter()
                .enumerate()
                .for_each(|(index, input)| run_one(index, input));
        }),
        // スレッドを作れなくても、順番に処理すれば変換はできる
        Err(_) => files
            .iter()
            .enumerate()
            .for_each(|(index, input)| run_one(index, input)),
    }

    if cancel.load(Ordering::SeqCst) {
        on_event(BatchEvent::Cancelled);
    }
    summary.into_inner().unwrap_or_else(|e| e.into_inner())
}

fn process_guarded(input: &Path, out_dir: &Path, s: &Settings) -> Result<FileReport, ProcessError> {
    let Some(name) = input.file_name() else {
        return Err(ProcessError::Read(std::io::Error::from(
            std::io::ErrorKind::InvalidInput,
        )));
    };
    let output = out_dir.join(name);
    // デコーダの panic で全体を止めない (F-10)
    panic::catch_unwind(AssertUnwindSafe(|| process_file(input, &output, s)))
        .unwrap_or_else(|payload| Err(ProcessError::Panic(panic_message(payload.as_ref()))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_jobs_is_between_1_and_4() {
        assert!((1..=4).contains(&default_jobs()));
    }
}
