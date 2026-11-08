use std::path::PathBuf;

use eframe::egui;
use egui::FontFamily;
use egui::epaint::text::{FontData, FontInsert, FontPriority, FontTweak, InsertFontFamily};

struct Candidate {
    path: PathBuf,
    /// 既定の英字フォントと縦位置がずれるフォントの補正値（フォントサイズに対する割合、正で下へ）
    y_offset_factor: f32,
}

/// 日本語フォントを OS から読み込む。見つからなければ false を返す
///
/// 配布サイズを抑えるため同梱しない (design §7.3)
/// 英語表示でもファイル名やパスに日本語が出るので、言語に関係なく読み込む
pub fn install_japanese_font(ctx: &egui::Context) -> bool {
    for candidate in candidates() {
        let Ok(bytes) = std::fs::read(&candidate.path) else {
            continue;
        };
        let families = [FontFamily::Proportional, FontFamily::Monospace]
            .into_iter()
            .map(|family| InsertFontFamily {
                family,
                priority: FontPriority::Lowest,
            })
            .collect();
        let tweak = FontTweak {
            y_offset_factor: candidate.y_offset_factor,
            ..FontTweak::default()
        };
        ctx.add_font(FontInsert::new(
            "japanese",
            FontData::from_owned(bytes).tweak(tweak),
            families,
        ));
        return true;
    }
    eprintln!("no Japanese font found; using English");
    false
}

fn candidates() -> Vec<Candidate> {
    let list: Vec<(PathBuf, f32)> = if cfg!(windows) {
        let fonts = std::env::var_os("WINDIR")
            .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
            .join("Fonts");
        // Yu Gothic は既定の英字フォントより上に描かれるので、画面で見て合わせた値で下げる
        vec![
            (fonts.join("YuGothM.ttc"), 0.25),
            (fonts.join("meiryo.ttc"), 0.0),
            (fonts.join("msgothic.ttc"), 0.0),
        ]
    } else if cfg!(target_os = "macos") {
        vec![
            (
                PathBuf::from("/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc"),
                0.0,
            ),
            (
                PathBuf::from("/System/Library/Fonts/Hiragino Sans GB.ttc"),
                0.0,
            ),
        ]
    } else {
        [
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
        ]
        .iter()
        .map(|p| (PathBuf::from(p), 0.0))
        .collect()
    };
    list.into_iter()
        .map(|(path, y_offset_factor)| Candidate {
            path,
            y_offset_factor,
        })
        .collect()
}
