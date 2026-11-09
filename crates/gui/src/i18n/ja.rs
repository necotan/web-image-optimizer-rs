use super::Strings;

pub static STRINGS: Strings = Strings {
    language: "言語",
    drop_here: "ここにフォルダをドロップ",
    select_folder: "フォルダを選択...",
    no_folder: "フォルダが選択されていません",
    jpeg_count: |n| format!("（JPEG {n}件）"),
    no_jpegs: "対象の JPEG ファイルが見つかりませんでした",
    folder_not_found: |path| format!("フォルダが見つかりません: {path}"),
    folder_unreadable: |path| format!("フォルダを読み込めません: {path}"),

    preset: "プリセット",
    custom: "カスタム",
    quality: "quality",
    long_edge: "長辺",
    jobs: "同時処理数",

    column_name: "ファイル名",
    column_original: "元サイズ",
    column_output: "出力サイズ",
    column_status: "状態",
    status_waiting: "待機",
    status_processing: "処理中…",
    status_done: "✓ 完了",
    status_done_with_warning: "⚠ 完了（警告あり）",
    status_failed: "✗ 失敗",
    status_skipped: "キャンセル",

    start: "変換開始",
    cancel: "キャンセル",
    cancelling: "キャンセル中…",
    start_failed: "処理を開始できませんでした",
    summary: |count, before, after, seconds, failed| {
        format!("完了: {count}件 / {before} -> {after} / {seconds:.1}秒  失敗 {failed}件")
    },
    cancelled: "（キャンセルしました）",
    open_output: "出力先を開く",
    failed_files: "失敗したファイル",
    warned_files: "警告のあったファイル",

    preview_open: "プレビュー",
    preview_open_hint: "リストでファイルを選んでください（行のダブルクリックでも開けます）",
    preview_title: "プレビュー",
    preview_hint: "リストでファイルを選ぶと、変換前後をここに表示します",
    preview_paused: "変換が終わったら更新します",
    preview_unavailable: "プレビューを開始できませんでした",
    preview_before: "変換前（縮小のみ）",
    preview_after: "変換後",
    zoom_fit: "全体",
    zoom_actual: "100%",
    preview_estimate: |output, input, change| {
        format!("出力サイズの見込み: {output}（元 {input}、{change:+.1}%）")
    },
    preview_too_large: |max| format!("画像が大きすぎて表示できません（GPU の上限 {max}px）"),

    warning_exif_removed: "EXIF を読み取れなかったため、EXIF なしで出力しました",
    warning_metadata_unreadable: "メタデータを読み取れなかったため、EXIF と色プロファイルなしで出力しました",

    error_settings: "設定が正しくありません",
    error_read: "ファイルを読み込めません",
    error_decode: "JPEG を読み込めません",
    error_unsupported_color_space: "対応していない色空間です",
    error_too_large: |width, height| format!("画像が大きすぎます: {width}x{height}"),
    error_resize: "縮小できません",
    error_encode: "JPEG を書き出せません",
    error_write: "ファイルを書き込めません",
    error_internal: "内部エラー",
};
