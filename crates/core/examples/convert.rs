//! 開発用の実行例 (比較と exiftool での確認に使う)

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use wio_core::{BatchEvent, Preset, default_jobs, run_batch, scan_dir};

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 * 1024 {
        format!("{:.0}KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(folder) = args.first().map(PathBuf::from) else {
        eprintln!("usage: convert <folder> [low|default|high] [jobs]");
        return ExitCode::FAILURE;
    };
    let preset = args
        .get(1)
        .map_or(Ok(Preset::Default), |name| name.parse::<Preset>());
    let Ok(preset) = preset else {
        eprintln!("unknown preset");
        return ExitCode::FAILURE;
    };
    let jobs = args
        .get(2)
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(default_jobs);

    let files = match scan_dir(&folder) {
        Ok(files) => files,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    let settings = preset.settings();
    let out_dir = folder.join("export");
    println!(
        "{} files ({}: long edge {}px / quality {}, jobs {jobs})",
        files.len(),
        preset.name(),
        settings.long_edge,
        settings.quality
    );

    let failed = Mutex::new(Vec::new());
    let cancel = AtomicBool::new(false);
    let started = Instant::now();
    let summary = run_batch(
        &files,
        &out_dir,
        &settings,
        jobs,
        &cancel,
        |event| match event {
            BatchEvent::Finished { index, report } => {
                let warnings = if report.warnings.is_empty() {
                    String::new()
                } else {
                    format!(" {:?}", report.warnings)
                };
                println!(
                    "  {}: {} -> {}{warnings}",
                    file_name(&files[index]),
                    format_bytes(report.input_bytes),
                    format_bytes(report.output_bytes)
                );
            }
            BatchEvent::Failed { index, error } => {
                if let Ok(mut list) = failed.lock() {
                    list.push(format!("{}: {error}", file_name(&files[index])));
                }
            }
            BatchEvent::Started { .. } | BatchEvent::Cancelled => {}
        },
    );

    println!();
    println!(
        "done: {} files / {} -> {} / {:.1}s / {} failed",
        summary.succeeded,
        format_bytes(summary.input_bytes),
        format_bytes(summary.output_bytes),
        started.elapsed().as_secs_f64(),
        summary.failed
    );
    println!("output: {}", out_dir.display());

    let list = failed.into_inner().unwrap_or_default();
    for line in &list {
        eprintln!("failed: {line}");
    }
    if list.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
