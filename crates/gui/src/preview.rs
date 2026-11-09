use std::any::Any;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{
    Color32, ColorImage, CursorIcon, Pos2, Rect, Sense, TextureFilter, TextureHandle,
    TextureOptions, Vec2, ViewportBuilder, ViewportClass, ViewportId,
};
use wio_core::{Preview, ProcessError, RgbImage, Settings, Warning};

use crate::i18n::{self, Lang, Strings};

/// スライダーを動かしている間に作り直し続けないよう、最後の変更からこれだけ待つ
const SETTLE: Duration = Duration::from_millis(300);

/// 100% 表示では 1 画素ずつ見えるよう拡大は最近傍にする
/// ミップマップは全体表示を滑らかにするが、ドライバが手放さないメモリが約 30MB 増えるので使わない (design §11)
const TEXTURE_OPTIONS: TextureOptions = TextureOptions {
    magnification: TextureFilter::Nearest,
    minification: TextureFilter::Linear,
    wrap_mode: egui::TextureWrapMode::ClampToEdge,
    mipmap_mode: None,
};

/// 画像はこの大きさのタイルに分けて GPU に渡す
/// ドライバは解放したテクスチャのメモリを寸法ごとに取っておくので、写真ごとに寸法の違うテクスチャを作ると切り替えるたびにメモリが増える
//  寸法をそろえると使い回される (design §11)
const TILE: usize = 512;

/// 別のファイルに切り替えて寸法がまだわからない間のスケルトンの縦横比
const DEFAULT_IMAGE_SIZE: Vec2 = Vec2::new(3.0, 2.0);

#[derive(Debug, Clone, PartialEq)]
pub struct PreviewKey {
    pub path: PathBuf,
    pub settings: Settings,
}

struct Request {
    id: u64,
    key: PreviewKey,
}

/// 画像の一部
// 端のタイルは TILE に満たない分を余白で埋め、size に画像の部分の大きさを持つ
#[derive(Clone)]
struct Tile<T> {
    content: T,
    pos: Vec2,
    size: Vec2,
}

/// 表示用のタイルまで作業スレッドで作り、画面のスレッドではテクスチャに渡すだけにする
struct Ready {
    /// 大きすぎて表示しない画像は None
    images: Option<[Vec<Tile<ColorImage>>; 2]>,
    width: u32,
    height: u32,
    input_bytes: u64,
    output_bytes: u64,
    warnings: Vec<Warning>,
}

struct Reply {
    id: u64,
    result: Result<Ready, ProcessError>,
}

struct Worker {
    tx: Sender<Request>,
    rx: Receiver<Reply>,
}

impl Worker {
    fn spawn(ctx: egui::Context) -> std::io::Result<Worker> {
        let (tx, requests) = mpsc::channel::<Request>();
        let (replies, rx) = mpsc::channel();
        thread::Builder::new()
            .name("preview".to_string())
            .spawn(move || {
                while let Ok(mut request) = requests.recv() {
                    // 作っている間に来た依頼は最新の 1 件だけ作る
                    // メモリを 1 枚分に抑える (N-04)
                    while let Ok(newer) = requests.try_recv() {
                        request = newer;
                    }
                    let max_side = ctx.input(|i| i.max_texture_side);
                    let key = &request.key;
                    let result = panic::catch_unwind(AssertUnwindSafe(|| {
                        wio_core::preview(&key.path, &key.settings)
                            .map(|preview| prepare(preview, max_side))
                    }))
                    .unwrap_or_else(|payload| Err(ProcessError::Panic(panic_text(&*payload))));
                    let reply = Reply {
                        id: request.id,
                        result,
                    };
                    if replies.send(reply).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })?;
        Ok(Worker { tx, rx })
    }
}

fn prepare(preview: Preview, max_side: usize) -> Ready {
    let Preview {
        reference,
        output,
        input_bytes,
        output_bytes,
        warnings,
    } = preview;
    let (width, height) = (output.width, output.height);
    // タイルなら GPU の上限には当たらないが、上限を超える大きさはメモリを使いすぎるので表示しない
    let images = if width as usize <= max_side && height as usize <= max_side {
        // 1 枚ずつ分けて元を捨て、RGB と RGBA を同時に 2 枚分持たない
        let before = split(reference);
        let after = split(output);
        Some([before, after])
    } else {
        None
    };
    Ready {
        images,
        width,
        height,
        input_bytes,
        output_bytes,
        warnings,
    }
}

fn split(image: RgbImage) -> Vec<Tile<ColorImage>> {
    let (width, height) = (image.width as usize, image.height as usize);
    let mut tiles = Vec::new();
    for top in (0..height).step_by(TILE) {
        for left in (0..width).step_by(TILE) {
            let w = TILE.min(width - left);
            let h = TILE.min(height - top);
            let mut content = ColorImage::filled([TILE, TILE], Color32::BLACK);
            for y in 0..h {
                let start = ((top + y) * width + left) * 3;
                let row = &image.pixels[start..start + w * 3];
                for (x, rgb) in row.chunks_exact(3).enumerate() {
                    content.pixels[y * TILE + x] = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                }
            }
            tiles.push(Tile {
                content,
                pos: Vec2::new(left as f32, top as f32),
                size: Vec2::new(w as f32, h as f32),
            });
        }
    }
    tiles
}

fn panic_text(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Zoom {
    #[default]
    Fit,
    Actual,
}

enum Textures {
    Loaded([Vec<Tile<TextureHandle>>; 2]),
    /// 大きすぎるので作っていない
    TooLarge,
    /// 作り直しの間は解放している
    Released,
}

struct Shown {
    key: PreviewKey,
    textures: Textures,
    width: u32,
    height: u32,
    input_bytes: u64,
    output_bytes: u64,
    warnings: Vec<Warning>,
}

#[derive(Default)]
pub struct PreviewWindow {
    open: bool,
    worker: Option<Worker>,
    worker_failed: bool,
    wanted: Option<PreviewKey>,
    send_after: Option<Instant>,
    sent: Option<(u64, PreviewKey)>,
    received: Option<u64>,
    next_id: u64,
    /// 届いた結果
    /// 前のテクスチャを解放したフレームの次に読み込む
    arrived: Option<(PreviewKey, Ready)>,
    shown: Option<Shown>,
    error: Option<(PreviewKey, ProcessError)>,
    zoom: Zoom,
    /// 100% 表示で画面の中央に来る画像上の位置（画素）
    /// None なら画像の中央
    center: Option<Vec2>,
    /// 別のファイルに切り替えたとき、前の画像と同じ形のスケルトンを出すために覚えておく
    last_size: Option<Vec2>,
}

impl PreviewWindow {
    pub fn open(&mut self) {
        self.open = true;
    }

    /// 閉じたら表示用の画像をすべて捨て、待機時のメモリに戻す
    fn close(&mut self) {
        self.open = false;
        self.wanted = None;
        self.sent = None;
        self.received = None;
        self.arrived = None;
        self.shown = None;
        self.error = None;
        self.center = None;
    }

    /// 毎フレーム呼ぶ
    /// 結果を受け取り、待ち時間が過ぎていれば次の依頼を送る
    pub fn update(&mut self, ctx: &egui::Context, selected: Option<PreviewKey>, paused: bool) {
        if !self.open {
            return;
        }
        if let Some((key, ready)) = self.arrived.take() {
            self.shown = Some(load(ctx, key, ready));
        }
        self.poll(ctx);

        if selected != self.wanted {
            self.send_after = Some(send_after(
                self.wanted.as_ref(),
                selected.as_ref(),
                Instant::now(),
            ));
            if selected.as_ref().map(|k| &k.path) != self.wanted.as_ref().map(|k| &k.path) {
                self.center = None;
            }
            self.wanted = selected;
        }

        let Some(wanted) = &self.wanted else {
            return;
        };
        if paused || self.worker_failed || self.sent.as_ref().map(|(_, k)| k) == Some(wanted) {
            return;
        }
        if let Some(at) = self.send_after {
            let now = Instant::now();
            if now < at {
                ctx.request_repaint_after(at - now);
                return;
            }
        }

        if self.worker.is_none() {
            match Worker::spawn(ctx.clone()) {
                Ok(worker) => self.worker = Some(worker),
                Err(_) => {
                    self.worker_failed = true;
                    return;
                }
            }
        }
        let Some(worker) = &self.worker else {
            return;
        };
        self.next_id += 1;
        let request = Request {
            id: self.next_id,
            key: wanted.clone(),
        };
        if worker.tx.send(request).is_err() {
            self.worker_failed = true;
            return;
        }
        // 作っている間も前の画像を持つと、作る分と合わせて N-04 の上限を超える
        if let Some(shown) = &mut self.shown
            && matches!(shown.textures, Textures::Loaded(_))
        {
            shown.textures = Textures::Released;
        }
        self.sent = Some((self.next_id, wanted.clone()));
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(worker) = &self.worker else {
            return;
        };
        let mut latest = None;
        loop {
            match worker.rx.try_recv() {
                Ok(reply) => latest = Some(reply),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.worker_failed = true;
                    break;
                }
            }
        }
        let Some(reply) = latest else {
            return;
        };
        let Some((sent_id, key)) = &self.sent else {
            return;
        };
        // 古い依頼の結果は、新しい依頼の結果が来るまでのつなぎにもしない
        if reply.id != *sent_id {
            return;
        }
        self.received = Some(reply.id);
        let key = key.clone();
        match reply.result {
            Ok(ready) => {
                self.error = None;
                // 新旧のテクスチャを同時に持たないよう、先に古いほうを解放して次のフレームで読み込む
                self.shown = None;
                self.arrived = Some((key, ready));
                ctx.request_repaint();
            }
            Err(error) => {
                if self.shown.as_ref().is_some_and(|s| s.key.path != key.path) {
                    self.shown = None;
                }
                self.error = Some((key, error));
            }
        }
    }

    fn is_loading(&self) -> bool {
        match &self.sent {
            Some((id, _)) => self.received != Some(*id) || self.arrived.is_some(),
            None => false,
        }
    }

    /// 開いていれば別ウィンドウを表示する
    /// 閉じるボタンか Esc で閉じる
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        s: &Strings,
        lang: Lang,
        paused: bool,
        file_name: Option<&str>,
    ) {
        if !self.open {
            return;
        }
        let title = match file_name {
            Some(name) => format!("{name} - {}", s.preview_title),
            None => s.preview_title.to_string(),
        };
        let builder = ViewportBuilder::default()
            .with_title(title)
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([480.0, 360.0]);
        let close = ctx.show_viewport_immediate(
            ViewportId::from_hash_of("preview"),
            builder,
            |ui, class| {
                if class == ViewportClass::EmbeddedWindow {
                    // 複数ウィンドウに対応しない環境では、メインウィンドウの中に出る
                    self.ui(ui, s, lang, paused);
                } else {
                    egui::CentralPanel::default().show(ui, |ui| {
                        self.ui(ui, s, lang, paused);
                    });
                }
                ui.input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape))
            },
        );
        if close {
            self.close();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, s: &Strings, lang: Lang, paused: bool) {
        let Some(wanted) = self.wanted.clone() else {
            ui.centered_and_justified(|ui| {
                ui.weak(s.preview_hint);
            });
            return;
        };
        if self.worker_failed {
            ui.colored_label(ui.visuals().error_fg_color, s.preview_unavailable);
            return;
        }

        let up_to_date = self.sent.as_ref().map(|(_, k)| k) == Some(&wanted);
        let loading = self.is_loading();
        let zoom = &mut self.zoom;
        let shown = self
            .shown
            .as_ref()
            .filter(|shown| shown.key.path == wanted.path);

        ui.horizontal(|ui| {
            if let Some(shown) = shown {
                ui.label(size_text(s, shown));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.selectable_value(zoom, Zoom::Actual, s.zoom_actual);
                ui.selectable_value(zoom, Zoom::Fit, s.zoom_fit);
                if paused && !up_to_date {
                    ui.weak(s.preview_paused);
                } else if loading || !up_to_date {
                    ui.spinner();
                }
            });
        });

        if let Some((key, error)) = &self.error
            && key.path == wanted.path
        {
            ui.colored_label(
                ui.visuals().error_fg_color,
                i18n::process_error_text(lang, error),
            );
            return;
        }
        let Some(shown) = shown else {
            let size = self.last_size.unwrap_or(DEFAULT_IMAGE_SIZE);
            self.images_ui(ui, s, None, size);
            return;
        };
        if !shown.warnings.is_empty() {
            let texts: Vec<&str> = shown
                .warnings
                .iter()
                .map(|w| i18n::warning_text(lang, *w))
                .collect();
            ui.colored_label(ui.visuals().warn_fg_color, texts.join(" / "));
        }
        let textures = match &shown.textures {
            Textures::Loaded(textures) => textures.clone(),
            Textures::TooLarge => {
                let max = ui.ctx().input(|i| i.max_texture_side);
                ui.colored_label(ui.visuals().warn_fg_color, (s.preview_too_large)(max));
                return;
            }
            Textures::Released => {
                let size = Vec2::new(shown.width as f32, shown.height as f32);
                self.images_ui(ui, s, None, size);
                return;
            }
        };
        let image_size = Vec2::new(shown.width as f32, shown.height as f32);
        self.last_size = Some(image_size);
        self.images_ui(ui, s, Some(&textures), image_size);
    }

    fn images_ui(
        &mut self,
        ui: &mut egui::Ui,
        s: &Strings,
        textures: Option<&[Vec<Tile<TextureHandle>>; 2]>,
        image_size: Vec2,
    ) {
        let gap = ui.spacing().item_spacing.x * 2.0;
        let label = ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y;
        let area = ui.available_rect_before_wrap();
        let panes = arrange(area, image_size, gap, label);
        ui.allocate_rect(area, Sense::hover());

        let painter = ui.painter();
        for (pane, title) in panes.iter().zip([s.preview_before, s.preview_after]) {
            painter.text(
                Pos2::new(pane.left(), pane.top() - label),
                egui::Align2::LEFT_TOP,
                title,
                egui::TextStyle::Body.resolve(ui.style()),
                ui.visuals().text_color(),
            );
        }

        let ppp = ui.ctx().pixels_per_point();
        let Some(textures) = textures else {
            skeleton_ui(ui, &panes, image_size, self.zoom, ppp);
            return;
        };

        // 2 枚が同じフレームで同じ位置を描くよう、先に両方の操作を反映してから描く
        for (i, pane) in panes.iter().enumerate() {
            let response = ui.interact(
                *pane,
                ui.make_persistent_id(("preview_pane", i)),
                Sense::click_and_drag(),
            );
            match self.zoom {
                Zoom::Fit => {
                    let fit = view(*pane, image_size, Zoom::Fit, None, ppp);
                    if response.hovered() {
                        ui.ctx().set_cursor_icon(CursorIcon::ZoomIn);
                    }
                    if response.clicked()
                        && let Some(pos) = response.interact_pointer_pos()
                        && fit.rect.contains(pos)
                    {
                        self.center = Some((pos - fit.origin) / fit.scale);
                        self.zoom = Zoom::Actual;
                    }
                }
                Zoom::Actual => {
                    if response.dragged() {
                        let center = self.center.unwrap_or(image_size / 2.0);
                        self.center = Some(center - response.drag_delta() * ppp);
                        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                    } else if response.hovered() {
                        ui.ctx().set_cursor_icon(CursorIcon::Grab);
                    }
                }
            }
        }

        for (pane, tiles) in panes.iter().zip(textures) {
            ui.painter()
                .rect_filled(*pane, 0.0, ui.visuals().extreme_bg_color);
            let v = view(*pane, image_size, self.zoom, self.center, ppp);
            if self.zoom == Zoom::Actual {
                // 端より外へドラッグした分をためないよう、収まる位置に戻して覚える
                self.center = Some(v.center);
            }
            let painter = ui.painter().with_clip_rect(v.rect.intersect(*pane));
            for tile in tiles {
                let rect = Rect::from_min_size(v.origin + tile.pos * v.scale, tile.size * v.scale);
                if !rect.intersects(v.rect) {
                    continue;
                }
                let uv = Rect::from_min_size(Pos2::ZERO, tile.size / TILE as f32);
                painter.image(tile.content.id(), rect, uv, Color32::WHITE);
            }
        }
    }
}

/// 作り直しの間、画像と同じ位置と大きさで明滅する面を描く
fn skeleton_ui(ui: &egui::Ui, panes: &[Rect; 2], image_size: Vec2, zoom: Zoom, ppp: f32) {
    let time = ui.input(|i| i.time);
    let pulse = ((time * std::f64::consts::TAU / 1.4).sin() * 0.5 + 0.5) as f32;
    let visuals = ui.visuals();
    let fill = visuals
        .widgets
        .noninteractive
        .bg_fill
        .lerp_to_gamma(visuals.widgets.inactive.bg_fill, pulse);
    for pane in panes {
        let painter = ui.painter().with_clip_rect(*pane);
        painter.rect_filled(*pane, 0.0, visuals.extreme_bg_color);
        let target = view(*pane, image_size, zoom, None, ppp).rect;
        painter.rect_filled(target, 4.0, fill);
    }
    ui.ctx().request_repaint();
}

fn load(ctx: &egui::Context, key: PreviewKey, ready: Ready) -> Shown {
    let upload = |name: &str, tiles: Vec<Tile<ColorImage>>| -> Vec<Tile<TextureHandle>> {
        tiles
            .into_iter()
            .enumerate()
            .map(|(i, tile)| Tile {
                content: ctx.load_texture(format!("{name}-{i}"), tile.content, TEXTURE_OPTIONS),
                pos: tile.pos,
                size: tile.size,
            })
            .collect()
    };
    let textures = match ready.images {
        Some([before, after]) => Textures::Loaded([
            upload("preview-before", before),
            upload("preview-after", after),
        ]),
        None => Textures::TooLarge,
    };
    Shown {
        key,
        textures,
        width: ready.width,
        height: ready.height,
        input_bytes: ready.input_bytes,
        output_bytes: ready.output_bytes,
        warnings: ready.warnings,
    }
}

/// 2 枚を左右に並べるか上下に重ねるか、画像が大きく見えるほうを選ぶ
/// 返す枠は見出しの下の部分
fn arrange(area: Rect, image_size: Vec2, gap: f32, label: f32) -> [Rect; 2] {
    let side_by_side = Vec2::new((area.width() - gap) / 2.0, area.height() - label);
    let stacked = Vec2::new(area.width(), (area.height() - gap) / 2.0 - label);
    let scale = |pane: Vec2| (pane.x / image_size.x).min(pane.y / image_size.y);
    if scale(side_by_side) >= scale(stacked) {
        let pane = side_by_side.max(Vec2::splat(1.0));
        let top = area.top() + label;
        [
            Rect::from_min_size(Pos2::new(area.left(), top), pane),
            Rect::from_min_size(Pos2::new(area.left() + pane.x + gap, top), pane),
        ]
    } else {
        let pane = stacked.max(Vec2::splat(1.0));
        let top = area.top() + label;
        [
            Rect::from_min_size(Pos2::new(area.left(), top), pane),
            Rect::from_min_size(Pos2::new(area.left(), top + pane.y + gap + label), pane),
        ]
    }
}

/// 別のファイルを選んだらすぐ、設定を変えただけなら少し待ってから作る
fn send_after(previous: Option<&PreviewKey>, next: Option<&PreviewKey>, now: Instant) -> Instant {
    match (previous, next) {
        (Some(previous), Some(next)) if previous.path == next.path => now + SETTLE,
        _ => now,
    }
}

fn size_text(s: &Strings, shown: &Shown) -> String {
    (s.preview_size)(
        &crate::app::format_bytes(shown.input_bytes),
        &crate::app::format_bytes(shown.output_bytes),
    )
}

/// 画像の画素から画面の位置への対応
struct View {
    /// 画像の左上が来る画面上の位置
    /// 100% 表示では枠の外になる
    origin: Pos2,
    /// 画像 1 画素あたりの画面上の大きさ（ポイント）
    scale: f32,
    /// 画像が見えている範囲
    rect: Rect,
    /// 100% 表示で枠の中央に来る画像上の位置（画素）
    /// 端を越えないよう収めた値
    center: Vec2,
}

fn view(pane: Rect, image_size: Vec2, zoom: Zoom, center: Option<Vec2>, ppp: f32) -> View {
    match zoom {
        // 枠に収まる大きさで中央に置く
        // 100% より大きくはしない
        Zoom::Fit => {
            let scale = (pane.width() / image_size.x)
                .min(pane.height() / image_size.y)
                .min(1.0 / ppp);
            let rect = Rect::from_center_size(pane.center(), image_size * scale);
            View {
                origin: rect.min,
                scale,
                rect,
                center: center.unwrap_or(image_size / 2.0),
            }
        }
        // 画像 1 画素を画面 1 画素に対応させる
        // ずれるとぼやけるので、位置は画面の画素に揃える
        Zoom::Actual => {
            let visible = Vec2::new(
                (pane.width() * ppp).floor().min(image_size.x),
                (pane.height() * ppp).floor().min(image_size.y),
            );
            let half = visible / 2.0;
            let center = center.unwrap_or(image_size / 2.0);
            let center = Vec2::new(
                center.x.clamp(half.x, image_size.x - half.x),
                center.y.clamp(half.y, image_size.y - half.y),
            );
            let start = (center - half).round();
            let size = visible / ppp;
            let min = pane.center() - size / 2.0;
            let min = Pos2::new((min.x * ppp).round() / ppp, (min.y * ppp).round() / ppp);
            View {
                origin: min - start / ppp,
                scale: 1.0 / ppp,
                rect: Rect::from_min_size(min, size),
                center,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(path: &str, quality: u8) -> PreviewKey {
        PreviewKey {
            path: PathBuf::from(path),
            settings: Settings {
                long_edge: 2400,
                quality,
            },
        }
    }

    #[test]
    fn waits_only_for_setting_changes() {
        let now = Instant::now();
        assert_eq!(send_after(None, Some(&key("a.jpg", 82)), now), now);
        assert_eq!(
            send_after(Some(&key("a.jpg", 82)), Some(&key("b.jpg", 82)), now),
            now
        );
        assert_eq!(
            send_after(Some(&key("a.jpg", 82)), Some(&key("a.jpg", 70)), now),
            now + SETTLE
        );
    }

    #[test]
    fn arranges_to_show_the_image_larger() {
        let wide_area = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 400.0));
        let tall_area = Rect::from_min_size(Pos2::ZERO, Vec2::new(500.0, 900.0));
        let landscape = Vec2::new(3200.0, 2133.0);
        let [a, b] = arrange(wide_area, landscape, 10.0, 20.0);
        assert_eq!(a.top(), b.top());
        let [a, b] = arrange(tall_area, landscape, 10.0, 20.0);
        assert_eq!(a.left(), b.left());
        assert!(b.top() >= a.bottom() + 10.0 + 20.0);
    }

    #[test]
    fn fit_never_enlarges_past_100_percent() {
        let pane = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let small = view(pane, Vec2::new(100.0, 50.0), Zoom::Fit, None, 1.25);
        assert_eq!(small.rect.size(), Vec2::new(80.0, 40.0));
        let large = view(pane, Vec2::new(4000.0, 1000.0), Zoom::Fit, None, 1.0);
        assert_eq!(large.rect.size(), Vec2::new(800.0, 200.0));
        assert_eq!(large.origin, Pos2::new(0.0, 200.0));
        assert_eq!(large.scale, 0.2);
    }

    #[test]
    fn actual_view_maps_one_pixel_to_one_pixel_and_stays_inside() {
        let pane = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(200.0, 100.0));
        let image = Vec2::new(1000.0, 800.0);
        let v = view(
            pane,
            image,
            Zoom::Actual,
            Some(Vec2::new(-50.0, 9999.0)),
            1.5,
        );
        assert_eq!(v.rect.size() * 1.5, Vec2::new(300.0, 150.0));
        assert_eq!(v.center, Vec2::new(150.0, 725.0));
        // 見えている左上は画像の (0, 650)
        let top_left = (v.rect.min - v.origin) / v.scale;
        assert!(
            (top_left - Vec2::new(0.0, 650.0)).length() < 1e-3,
            "{top_left:?}"
        );
        let snapped = v.rect.min * 1.5;
        assert_eq!(snapped, snapped.round());
    }

    #[test]
    fn splits_into_same_size_tiles_without_losing_pixels() {
        let (width, height) = (TILE as u32 + 3, 2);
        let pixels: Vec<u8> = (0..width * height * 3).map(|i| (i % 251) as u8).collect();
        let tiles = split(RgbImage {
            width,
            height,
            pixels: pixels.clone(),
        });
        assert_eq!(tiles.len(), 2);
        assert!(tiles.iter().all(|t| t.content.size == [TILE, TILE]));
        assert_eq!(tiles[1].pos, Vec2::new(TILE as f32, 0.0));
        assert_eq!(tiles[1].size, Vec2::new(3.0, 2.0));
        // 2 行目の最後の画素は右のタイルの (2, 1) に入る
        let last = &pixels[pixels.len() - 3..];
        assert_eq!(
            tiles[1].content.pixels[TILE + 2],
            Color32::from_rgb(last[0], last[1], last[2])
        );
    }
}
