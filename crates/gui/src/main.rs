#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;

mod app;
mod fonts;
mod i18n;

/// 設定の保存先フォルダ名にもなるので変えない
const APP_ID: &str = "web-image-optimizer-rs";

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Web Image Optimizer")
            .with_inner_size([860.0, 640.0])
            .with_min_inner_size([600.0, 420.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        APP_ID,
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
