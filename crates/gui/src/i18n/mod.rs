mod en;
mod ja;

use serde::{Deserialize, Serialize};
use wio_core::{ProcessError, ScanError, Warning};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lang {
    Ja,
    En,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::Ja, Lang::En];

    pub fn detect() -> Lang {
        Lang::from_locale(sys_locale::get_locale().as_deref())
    }

    fn from_locale(locale: Option<&str>) -> Lang {
        match locale {
            Some(l) if l.to_ascii_lowercase().starts_with("ja") => Lang::Ja,
            _ => Lang::En,
        }
    }

    /// 選択肢は、今どちらの言語で表示していても読めるように各言語自身の名前で示す (L-03)
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::Ja => "日本語",
            Lang::En => "English",
        }
    }

    pub fn strings(self) -> &'static Strings {
        match self {
            Lang::Ja => &ja::STRINGS,
            Lang::En => &en::STRINGS,
        }
    }
}

/// 表示文字列
// ja.rs と en.rs で 1 つずつ定義するので、片方の訳の漏れはコンパイルエラーになる
pub struct Strings {
    pub drop_here: &'static str,
    pub select_folder: &'static str,
    pub no_jpegs: &'static str,
    pub folder_not_found: fn(&str) -> String,
    pub folder_unreadable: fn(&str) -> String,

    pub preset: &'static str,
    pub custom: &'static str,
    pub quality: &'static str,
    pub long_edge: &'static str,
    pub jobs: &'static str,

    pub column_name: &'static str,
    pub column_original: &'static str,
    pub column_output: &'static str,
    pub column_status: &'static str,
    pub status_waiting: &'static str,
    pub status_processing: &'static str,
    pub status_done: &'static str,
    pub status_done_with_warning: &'static str,
    pub status_failed: &'static str,
    pub status_skipped: &'static str,

    pub start: &'static str,
    pub cancel: &'static str,
    pub cancelling: &'static str,
    pub start_failed: &'static str,
    /// 件数、変換前の合計、変換後の合計、秒数、失敗件数
    pub summary: fn(usize, &str, &str, f64, usize) -> String,
    pub cancelled: &'static str,
    pub open_output: &'static str,
    pub failed_files: &'static str,
    pub warned_files: &'static str,

    pub preview_open: &'static str,
    pub preview_open_hint: &'static str,
    pub preview_title: &'static str,
    pub preview_hint: &'static str,
    pub preview_paused: &'static str,
    pub preview_unavailable: &'static str,
    pub preview_before: &'static str,
    pub preview_after: &'static str,
    pub zoom_fit: &'static str,
    pub zoom_actual: &'static str,
    /// 元のサイズ、出力サイズ
    pub preview_size: fn(&str, &str) -> String,
    /// GPU のテクスチャの上限（px）
    pub preview_too_large: fn(usize) -> String,

    pub warning_exif_removed: &'static str,
    pub warning_metadata_unreadable: &'static str,

    pub error_settings: &'static str,
    pub error_read: &'static str,
    pub error_decode: &'static str,
    pub error_unsupported_color_space: &'static str,
    pub error_too_large: fn(u32, u32) -> String,
    pub error_resize: &'static str,
    pub error_encode: &'static str,
    pub error_write: &'static str,
    pub error_internal: &'static str,
}

pub fn warning_text(lang: Lang, warning: Warning) -> &'static str {
    let s = lang.strings();
    match warning {
        Warning::ExifRemoved => s.warning_exif_removed,
        Warning::MetadataUnreadable => s.warning_metadata_unreadable,
    }
}

/// ライブラリや OS から来る詳細は訳さず、訳した理由の後ろに付ける
pub fn process_error_text(lang: Lang, error: &ProcessError) -> String {
    let s = lang.strings();
    match error {
        ProcessError::Settings(_) => s.error_settings.to_string(),
        ProcessError::Read(e) => format!("{}: {e}", s.error_read),
        ProcessError::Decode(detail) => format!("{}: {detail}", s.error_decode),
        ProcessError::UnsupportedColorSpace(space) => {
            format!("{}: {space}", s.error_unsupported_color_space)
        }
        ProcessError::ImageTooLarge { width, height } => (s.error_too_large)(*width, *height),
        ProcessError::Resize(detail) => format!("{}: {detail}", s.error_resize),
        ProcessError::Encode(detail) => format!("{}: {detail}", s.error_encode),
        ProcessError::Write(e) => format!("{}: {e}", s.error_write),
        ProcessError::Panic(detail) => format!("{}: {detail}", s.error_internal),
    }
}

pub fn scan_error_text(lang: Lang, error: &ScanError) -> String {
    let s = lang.strings();
    match error {
        ScanError::NotFound(path) => (s.folder_not_found)(&path.display().to_string()),
        ScanError::Read { path, .. } => (s.folder_unreadable)(&path.display().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_os_locale() {
        assert_eq!(Lang::from_locale(Some("ja-JP")), Lang::Ja);
        assert_eq!(Lang::from_locale(Some("ja")), Lang::Ja);
        assert_eq!(Lang::from_locale(Some("JA_jp")), Lang::Ja);
        assert_eq!(Lang::from_locale(Some("en-US")), Lang::En);
        assert_eq!(Lang::from_locale(Some("fr-FR")), Lang::En);
        assert_eq!(Lang::from_locale(None), Lang::En);
    }

    #[test]
    fn error_text_keeps_details() {
        let error = ProcessError::Decode("Exhausted data".to_string());
        for lang in Lang::ALL {
            let text = process_error_text(lang, &error);
            assert!(text.ends_with(": Exhausted data"), "{text}");
        }
        assert_ne!(
            process_error_text(Lang::Ja, &error),
            process_error_text(Lang::En, &error)
        );
    }

    #[test]
    fn summary_matches_the_old_version_in_japanese() {
        let text = (Lang::Ja.strings().summary)(2, "47.9MB", "3.4MB", 2.14, 0);
        assert_eq!(text, "完了: 2件 / 47.9MB -> 3.4MB / 2.1秒  失敗 0件");
    }
}
