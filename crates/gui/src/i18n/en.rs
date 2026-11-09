use super::Strings;

pub static STRINGS: Strings = Strings {
    drop_here: "Drop a folder to select it",
    select_folder: "Select folder",
    no_jpegs: "No JPEG files found",
    folder_not_found: |path| format!("Folder not found: {path}"),
    folder_unreadable: |path| format!("Cannot read folder: {path}"),

    preset: "Preset",
    custom: "Custom",
    quality: "Quality",
    long_edge: "Long edge",
    jobs: "Concurrent files",

    column_name: "File name",
    column_original: "Original size",
    column_output: "Output size",
    column_status: "Status",
    status_waiting: "Waiting",
    status_processing: "Processing...",
    status_done: "✓ Done",
    status_done_with_warning: "⚠ Done (warning)",
    status_failed: "✗ Failed",
    status_skipped: "Cancelled",

    start: "Start",
    cancel: "Cancel",
    cancelling: "Cancelling...",
    start_failed: "Could not start processing",
    summary: |count, before, after, seconds, failed| {
        format!("Done: {count} files / {before} -> {after} / {seconds:.1}s  {failed} failed")
    },
    cancelled: "(cancelled)",
    open_output: "Open output folder",
    failed_files: "Failed files",
    warned_files: "Files with warnings",

    preview_open: "Preview",
    preview_open_hint: "Select a file in the list (or double-click a row)",
    preview_title: "Preview",
    preview_hint: "Select a file in the list to compare it before and after conversion here",
    preview_paused: "Updates after the conversion finishes",
    preview_unavailable: "Could not start the preview",
    preview_before: "Before",
    preview_after: "After",
    zoom_fit: "Fit",
    zoom_actual: "100%",
    preview_size: |input, output| format!("File size: {input} -> {output}"),
    preview_too_large: |max| format!("Too large to display (GPU limit {max}px)"),

    warning_exif_removed: "EXIF could not be read, so the output has no EXIF",
    warning_metadata_unreadable: "Metadata could not be read, so the output has no EXIF or color profile",

    error_settings: "Invalid settings",
    error_read: "Cannot read the file",
    error_decode: "Cannot decode the JPEG",
    error_unsupported_color_space: "Unsupported color space",
    error_too_large: |width, height| format!("Image is too large: {width}x{height}"),
    error_resize: "Cannot resize",
    error_encode: "Cannot encode the JPEG",
    error_write: "Cannot write the file",
    error_internal: "Internal error",
};
