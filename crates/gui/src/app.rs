use std::path::{Path, PathBuf};

use eframe::egui;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use egui_extras::{Column, TableBuilder};
use serde::{Deserialize, Serialize};
use wio_core::{
    BatchEvent, BatchSummary, Preset, ProcessError, ScanError, Settings, Warning, default_jobs,
    run_batch, scan_dir,
};

use crate::fonts;
use crate::i18n::{self, Lang, Strings};

/// 同時処理数の上限。これ以上はメモリが増えるだけで速くならない
const MAX_JOBS: usize = 16;

#[derive(Serialize, Deserialize)]
struct Saved {
    settings_long_edge: u32,
    settings_quality: u8,
    jobs: usize,
    lang: Lang,
}

/// 言語を切り替えたら表示済みの行も訳し直せるよう、表示文字列ではなく状態として持つ
enum RowStatus {
    Waiting,
    Processing,
    Done(Vec<Warning>),
    Failed(ProcessError),
    Skipped,
}

struct FileRow {
    path: PathBuf,
    name: String,
    input_bytes: u64,
    output_bytes: Option<u64>,
    status: RowStatus,
}

enum WorkerMessage {
    Event(BatchEvent),
    Finished(BatchSummary),
}

struct Running {
    rx: Receiver<WorkerMessage>,
    cancel: Arc<AtomicBool>,
    started: Instant,
}

struct Finished {
    summary: BatchSummary,
    elapsed: Duration,
    cancelled: bool,
}

pub struct App {
    lang: Lang,
    has_japanese_font: bool,
    input_dir: Option<PathBuf>,
    scan_error: Option<ScanError>,
    rows: Vec<FileRow>,
    settings: Settings,
    jobs: usize,
    running: Option<Running>,
    finished: Option<Finished>,
    start_failed: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> App {
        let has_japanese_font = fonts::install_japanese_font(&cc.egui_ctx);
        let saved: Option<Saved> = cc
            .storage
            .and_then(|storage| eframe::get_value(storage, eframe::APP_KEY));

        let mut app = App {
            lang: Lang::detect(),
            has_japanese_font,
            input_dir: None,
            scan_error: None,
            rows: Vec::new(),
            settings: Preset::Default.settings(),
            jobs: default_jobs(),
            running: None,
            finished: None,
            start_failed: false,
        };
        if let Some(saved) = saved {
            let settings = Settings {
                long_edge: saved.settings_long_edge,
                quality: saved.settings_quality,
            };
            if settings.validate().is_ok() {
                app.settings = settings;
            }
            app.jobs = saved.jobs.clamp(1, MAX_JOBS);
            app.lang = saved.lang;
        }
        app
    }

    /// 日本語フォントがなければ日本語のラベルが読めないので英語にする (design §7.3)
    fn display_lang(&self) -> Lang {
        if self.has_japanese_font {
            self.lang
        } else {
            Lang::En
        }
    }

    fn is_running(&self) -> bool {
        self.running.is_some()
    }

    fn current_preset(&self) -> Option<Preset> {
        Preset::ALL
            .into_iter()
            .find(|p| p.settings() == self.settings)
    }

    fn set_folder(&mut self, dir: PathBuf) {
        self.finished = None;
        self.start_failed = false;
        match scan_dir(&dir) {
            Ok(files) => {
                self.rows = files
                    .into_iter()
                    .map(|path| FileRow {
                        name: path
                            .file_name()
                            .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                        input_bytes: std::fs::metadata(&path).map_or(0, |m| m.len()),
                        output_bytes: None,
                        status: RowStatus::Waiting,
                        path,
                    })
                    .collect();
                self.scan_error = None;
            }
            Err(error) => {
                self.rows.clear();
                self.scan_error = Some(error);
            }
        }
        self.input_dir = Some(dir);
    }

    fn start(&mut self, ctx: &egui::Context) {
        let Some(dir) = self.input_dir.clone() else {
            return;
        };
        for row in &mut self.rows {
            row.status = RowStatus::Waiting;
            row.output_bytes = None;
        }
        self.finished = None;
        self.start_failed = false;

        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let files: Vec<PathBuf> = self.rows.iter().map(|r| r.path.clone()).collect();
        let settings = self.settings;
        let jobs = self.jobs;
        let worker_cancel = Arc::clone(&cancel);
        let ctx = ctx.clone();

        let spawned = thread::Builder::new().spawn(move || {
            let out_dir = dir.join("export");
            let summary = run_batch(&files, &out_dir, &settings, jobs, &worker_cancel, |event| {
                let _ = tx.send(WorkerMessage::Event(event));
                ctx.request_repaint();
            });
            let _ = tx.send(WorkerMessage::Finished(summary));
            ctx.request_repaint();
        });

        match spawned {
            Ok(_) => {
                self.running = Some(Running {
                    rx,
                    cancel,
                    started: Instant::now(),
                });
            }
            Err(_) => self.start_failed = true,
        }
    }

    fn poll_worker(&mut self) {
        let Some(running) = &self.running else {
            return;
        };
        let mut finished = None;
        while let Ok(message) = running.rx.try_recv() {
            match message {
                WorkerMessage::Event(event) => apply_event(&mut self.rows, event),
                WorkerMessage::Finished(summary) => finished = Some(summary),
            }
        }
        if let Some(summary) = finished {
            let cancelled = running.cancel.load(Ordering::SeqCst);
            let elapsed = running.started.elapsed();
            if cancelled {
                for row in &mut self.rows {
                    if matches!(row.status, RowStatus::Waiting) {
                        row.status = RowStatus::Skipped;
                    }
                }
            }
            self.finished = Some(Finished {
                summary,
                elapsed,
                cancelled,
            });
            self.running = None;
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        if self.is_running() {
            return;
        }
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        let Some(path) = dropped.first().map(|f| f.path().to_path_buf()) else {
            return;
        };
        // フォルダ以外がドロップされたら、そのファイルのあるフォルダを対象にする
        let dir = if path.is_dir() {
            Some(path)
        } else {
            path.parent().map(Path::to_path_buf)
        };
        if let Some(dir) = dir {
            self.set_folder(dir);
        }
    }

    fn language_ui(&mut self, ui: &mut egui::Ui, s: &Strings) {
        // horizontal で囲まないと、右寄せのレイアウトがパネルの高さをすべて使ってしまう
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for lang in Lang::ALL.into_iter().rev() {
                    ui.selectable_value(&mut self.lang, lang, lang.native_name());
                }
                ui.label(s.language);
            });
        });
    }

    fn input_ui(&mut self, ui: &mut egui::Ui, s: &Strings, lang: Lang) {
        let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        ui.add_enabled_ui(!self.is_running(), |ui| {
            ui.horizontal(|ui| {
                let stroke = if hovering {
                    ui.visuals().selection.stroke
                } else {
                    ui.visuals().widgets.noninteractive.bg_stroke
                };
                egui::Frame::group(ui.style())
                    .stroke(stroke)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width() - 170.0);
                        ui.add_space(8.0);
                        ui.label(s.drop_here);
                        ui.add_space(8.0);
                    });
                if ui.button(s.select_folder).clicked()
                    && let Some(dir) = rfd::FileDialog::new().pick_folder()
                {
                    self.set_folder(dir);
                }
            });
        });

        match (&self.input_dir, &self.scan_error) {
            (_, Some(error)) => {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    i18n::scan_error_text(lang, error),
                );
            }
            (Some(dir), None) if self.rows.is_empty() => {
                ui.label(format!("{}  {}", dir.display(), s.no_jpegs));
            }
            (Some(dir), None) => {
                ui.label(format!(
                    "{}  {}",
                    dir.display(),
                    (s.jpeg_count)(self.rows.len())
                ));
            }
            (None, None) => {
                ui.weak(s.no_folder);
            }
        }
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui, s: &Strings) {
        ui.add_enabled_ui(!self.is_running(), |ui| {
            egui::Grid::new("settings")
                .num_columns(2)
                .spacing([16.0, 8.0])
                .show(ui, |ui| {
                    ui.label(s.preset);
                    ui.horizontal(|ui| {
                        let current = self.current_preset();
                        for preset in Preset::ALL {
                            if ui.radio(current == Some(preset), preset.name()).clicked() {
                                self.settings = preset.settings();
                            }
                        }
                        if current.is_none() {
                            ui.strong(s.custom);
                        }
                    });
                    ui.end_row();

                    ui.label(s.quality);
                    ui.add(egui::Slider::new(&mut self.settings.quality, 1..=100));
                    ui.end_row();

                    ui.label(s.long_edge);
                    ui.add(
                        egui::DragValue::new(&mut self.settings.long_edge)
                            .range(1..=65535)
                            .suffix(" px"),
                    );
                    ui.end_row();

                    ui.label(s.jobs);
                    ui.add(egui::DragValue::new(&mut self.jobs).range(1..=MAX_JOBS));
                    ui.end_row();
                });
        });
    }

    fn table_ui(&self, ui: &mut egui::Ui, s: &Strings, lang: Lang) {
        let row_height = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
        TableBuilder::new(ui)
            .striped(true)
            .column(Column::remainder().at_least(160.0).clip(true))
            .column(Column::exact(96.0))
            .column(Column::exact(96.0))
            .column(Column::initial(170.0).at_least(120.0).clip(true))
            .header(row_height, |mut header| {
                for title in [
                    s.column_name,
                    s.column_original,
                    s.column_output,
                    s.column_status,
                ] {
                    header.col(|ui| {
                        ui.strong(title);
                    });
                }
            })
            .body(|body| {
                body.rows(row_height, self.rows.len(), |mut row| {
                    let file = &self.rows[row.index()];
                    row.col(|ui| {
                        ui.label(&file.name);
                    });
                    row.col(|ui| {
                        ui.label(format_bytes(file.input_bytes));
                    });
                    row.col(|ui| {
                        ui.label(
                            file.output_bytes
                                .map_or_else(|| "-".to_string(), format_bytes),
                        );
                    });
                    row.col(|ui| {
                        status_ui(ui, s, lang, &file.status);
                    });
                });
            });
    }

    fn progress_ui(&mut self, ui: &mut egui::Ui, s: &Strings, lang: Lang) {
        let total = self.rows.len();
        let done = self
            .rows
            .iter()
            .filter(|r| !matches!(r.status, RowStatus::Waiting | RowStatus::Processing))
            .count();
        let fraction = if total == 0 {
            0.0
        } else {
            done as f32 / total as f32
        };

        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                match &self.running {
                    Some(running) => {
                        let cancelling = running.cancel.load(Ordering::SeqCst);
                        let label = if cancelling { s.cancelling } else { s.cancel };
                        if ui
                            .add_enabled(!cancelling, egui::Button::new(label))
                            .clicked()
                        {
                            running.cancel.store(true, Ordering::SeqCst);
                        }
                    }
                    None => {
                        let can_start = !self.rows.is_empty();
                        if ui
                            .add_enabled(can_start, egui::Button::new(s.start))
                            .clicked()
                        {
                            self.start(ui.ctx());
                        }
                    }
                }
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .text(format!("{done} / {total}"))
                        .desired_width(ui.available_width()),
                );
            });
        });

        if self.start_failed {
            ui.colored_label(ui.visuals().error_fg_color, s.start_failed);
        }

        if let Some(finished) = &self.finished {
            ui.horizontal(|ui| {
                let summary = &finished.summary;
                let mut text = (s.summary)(
                    summary.succeeded,
                    &format_bytes(summary.input_bytes),
                    &format_bytes(summary.output_bytes),
                    finished.elapsed.as_secs_f64(),
                    summary.failed,
                );
                if finished.cancelled {
                    text.push(' ');
                    text.push_str(s.cancelled);
                }
                ui.label(text);
                if let Some(dir) = &self.input_dir
                    && ui.button(s.open_output).clicked()
                {
                    open_folder(&dir.join("export"));
                }
            });
            self.issues_ui(ui, s, lang);
        }
    }

    fn issues_ui(&self, ui: &mut egui::Ui, s: &Strings, lang: Lang) {
        let failed: Vec<String> = self
            .rows
            .iter()
            .filter_map(|r| match &r.status {
                RowStatus::Failed(e) => {
                    Some(format!("{}: {}", r.name, i18n::process_error_text(lang, e)))
                }
                _ => None,
            })
            .collect();
        let warned: Vec<String> = self
            .rows
            .iter()
            .filter_map(|r| match &r.status {
                RowStatus::Done(warnings) if !warnings.is_empty() => {
                    let texts: Vec<&str> = warnings
                        .iter()
                        .map(|w| i18n::warning_text(lang, *w))
                        .collect();
                    Some(format!("{}: {}", r.name, texts.join(" / ")))
                }
                _ => None,
            })
            .collect();
        if failed.is_empty() && warned.is_empty() {
            return;
        }
        egui::ScrollArea::vertical()
            .max_height(120.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (title, lines, color) in [
                    (s.failed_files, &failed, ui.visuals().error_fg_color),
                    (s.warned_files, &warned, ui.visuals().warn_fg_color),
                ] {
                    if lines.is_empty() {
                        continue;
                    }
                    ui.strong(title);
                    for line in lines {
                        ui.colored_label(color, line);
                    }
                }
            });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_worker();
        self.handle_dropped_files(ui.ctx());

        let lang = self.display_lang();
        let s = lang.strings();

        egui::Panel::top("input").show(ui, |ui| {
            ui.add_space(6.0);
            self.language_ui(ui, s);
            self.input_ui(ui, s, lang);
            ui.separator();
            self.settings_ui(ui, s);
            ui.add_space(6.0);
        });
        egui::Panel::bottom("progress").show(ui, |ui| {
            ui.add_space(6.0);
            self.progress_ui(ui, s, lang);
            ui.add_space(6.0);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            self.table_ui(ui, s, lang);
        });
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let saved = Saved {
            settings_long_edge: self.settings.long_edge,
            settings_quality: self.settings.quality,
            jobs: self.jobs,
            lang: self.lang,
        };
        eframe::set_value(storage, eframe::APP_KEY, &saved);
    }
}

fn apply_event(rows: &mut [FileRow], event: BatchEvent) {
    match event {
        BatchEvent::Started { index } => {
            if let Some(row) = rows.get_mut(index) {
                row.status = RowStatus::Processing;
            }
        }
        BatchEvent::Finished { index, report } => {
            if let Some(row) = rows.get_mut(index) {
                row.output_bytes = Some(report.output_bytes);
                row.status = RowStatus::Done(report.warnings);
            }
        }
        BatchEvent::Failed { index, error } => {
            if let Some(row) = rows.get_mut(index) {
                row.status = RowStatus::Failed(error);
            }
        }
        BatchEvent::Cancelled => {}
    }
}

fn status_ui(ui: &mut egui::Ui, s: &Strings, lang: Lang, status: &RowStatus) {
    match status {
        RowStatus::Waiting => {
            ui.weak(s.status_waiting);
        }
        RowStatus::Processing => {
            ui.label(s.status_processing);
        }
        RowStatus::Done(warnings) if warnings.is_empty() => {
            ui.label(s.status_done);
        }
        RowStatus::Done(warnings) => {
            let texts: Vec<&str> = warnings
                .iter()
                .map(|w| i18n::warning_text(lang, *w))
                .collect();
            ui.colored_label(ui.visuals().warn_fg_color, s.status_done_with_warning)
                .on_hover_text(texts.join("\n"));
        }
        RowStatus::Failed(error) => {
            ui.colored_label(ui.visuals().error_fg_color, s.status_failed)
                .on_hover_text(i18n::process_error_text(lang, error));
        }
        RowStatus::Skipped => {
            ui.weak(s.status_skipped);
        }
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 * 1024 {
        format!("{:.0}KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn open_folder(path: &Path) {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    // explorer は成功しても終了コード 1 を返すので結果は見ない
    let _ = std::process::Command::new(program).arg(path).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bytes_like_the_old_version() {
        assert_eq!(format_bytes(500 * 1024), "500KB");
        assert_eq!(format_bytes(25_000_000), "23.8MB");
    }

    #[test]
    fn events_update_rows() {
        let mut rows: Vec<FileRow> = (0..2)
            .map(|i| FileRow {
                path: PathBuf::from(format!("{i}.jpg")),
                name: format!("{i}.jpg"),
                input_bytes: 10,
                output_bytes: None,
                status: RowStatus::Waiting,
            })
            .collect();
        apply_event(&mut rows, BatchEvent::Started { index: 0 });
        assert!(matches!(rows[0].status, RowStatus::Processing));
        apply_event(
            &mut rows,
            BatchEvent::Failed {
                index: 1,
                error: ProcessError::Decode("x".to_string()),
            },
        );
        assert!(matches!(rows[1].status, RowStatus::Failed(_)));
        apply_event(&mut rows, BatchEvent::Started { index: 9 });
    }
}
