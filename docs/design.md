# Design

How web-image-optimizer-rs is built. Requirements are in `docs/requirements.md`.

Crate names and APIs are assumptions from the time of writing. **Check the latest documentation when implementing, and update this file if they differ.**

## 1. Structure

A Cargo workspace separates the processing core and the GUI.

```
web-image-optimizer-rs/
├── Cargo.toml              # workspace definition, shared release profile
├── CLAUDE.md
├── docs/
│   ├── requirements.md
│   └── design.md
├── crates/
│   ├── core/               # library: wio-core
│   │   ├── examples/
│   │   │   └── convert.rs      # development runner, not distributed
│   │   ├── tests/
│   │   │   └── conversion.rs   # integration tests with generated JPEGs
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── preset.rs       # presets and settings validation
│   │       ├── scan.rs         # target file listing
│   │       ├── pipeline.rs     # conversion of one file
│   │       ├── orient.rs       # rotation and flipping by Orientation
│   │       ├── metadata.rs     # EXIF/ICC extraction, EXIF rebuild
│   │       ├── encode.rs       # Encoder trait, mozjpeg settings
│   │       ├── batch.rs        # parallel execution, progress, cancellation
│   │       └── error.rs
│   └── gui/                # package: wio-gui (executable: wio-gui)
│       └── src/
│           ├── main.rs
│           ├── app.rs          # screen, batch worker
│           ├── preview.rs      # preview worker and window (phase 3)
│           ├── fonts.rs        # Japanese font loading
│           └── i18n/
│               ├── mod.rs      # Lang, Strings, OS language detection
│               ├── ja.rs       # Japanese text
│               └── en.rs       # English text
```

Dependencies point only `gui -> core`. `core` knows nothing about the UI or the display language.

### 1.1 Language Support (crates/gui/src/i18n)

All user-facing text lives in `crates/gui/src/i18n/` (requirements §4.5).

```rust
pub enum Lang { Ja, En }

pub struct Strings {
    pub start: &'static str,
    pub file_count: fn(usize) -> String,
    // one field per text
}

impl Lang {
    pub fn detect() -> Lang;                  // from the OS language
    pub fn strings(self) -> &'static Strings;
}

pub fn process_error_text(lang: Lang, e: &ProcessError) -> String;
```

- `ja.rs` and `en.rs` each define one `Strings` value. A text missing from either one is a compile error, which a key-based file format would only show at runtime.
- Text with placeholders is a `fn` field, so each language can order the words as it needs.
- `Lang::detect()` reads the OS language with `sys-locale`. A value starting with `ja` means `Lang::Ja`; anything else, including no value, means `Lang::En`.
- `core` has no user-facing text. It returns typed errors and events, and `process_error_text` turns them into text. The `thiserror` messages in `core` are English and are for developers only.
- Details that come from libraries or the OS, such as decoder and I/O messages, cannot be translated. The screen shows only the translated reason, and the detail appears as it is when the pointer is over the reason.

## 2. Crates

| Purpose | Crate | Notes |
|---|---|---|
| JPEG decode | `zune-jpeg` | Pure Rust and fast. Can also read ICC. |
| Resize | `fast_image_resize` | SIMD. Filter is Lanczos3, the sharp default. |
| JPEG encode | `mozjpeg` | See §4.4. |
| JPEG segment reading | `img-parts` | Read APP1 (EXIF) and APP2 (ICC). |
| EXIF editing | None | Hand-written in `metadata.rs` (§4.5). `little_exif` was rejected (§11). |
| Parallelism | `rayon` | A dedicated thread pool limits concurrency. |
| Errors | `thiserror` (core), `anyhow` (gui) | |
| GUI | `eframe` / `egui` | Rendering backend is **glow**, lighter than wgpu. |
| File dialog | `rfd` | |
| Settings storage | eframe `persistence` feature + `serde` | |
| OS language detection | `sys-locale` | See §1.1. |

## 3. core Public API

```rust
pub struct Settings {
    pub long_edge: u32,     // 1 or more
    pub quality: u8,        // 1..=100
}

pub enum Preset { Low, Default, High }
impl Preset {
    pub fn settings(self) -> Settings;
}

/// Lists the JPEGs directly inside the folder, sorted by name (excludes export/)
pub fn scan_dir(dir: &Path) -> Result<Vec<PathBuf>, ScanError>;

/// Converts one file and writes it to the output path
pub fn process_file(input: &Path, output: &Path, s: &Settings) -> Result<FileReport, ProcessError>;

/// Converts in memory and returns the encoded bytes
pub fn process_to_memory(input: &Path, s: &Settings) -> Result<Vec<u8>, ProcessError>;

/// Converts in memory and returns both images for display (phase 3)
pub fn preview(input: &Path, s: &Settings) -> Result<Preview, ProcessError>;

pub struct Preview {
    pub reference: RgbImage,  // resized and rotated, before encoding
    pub output: RgbImage,     // the encoded JPEG decoded again
    pub input_bytes: u64,
    pub output_bytes: u64,    // equals the size process_file writes
    pub warnings: Vec<Warning>,
}

/// 8-bit RGB, rows top to bottom; grayscale is expanded to RGB
pub struct RgbImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub struct FileReport {
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub warnings: Vec<Warning>,
}

pub enum Warning {
    ExifRemoved,          // EXIF could not be parsed
    MetadataUnreadable,   // JPEG segments could not be read; no EXIF or ICC in the output
}

/// min(CPU cores, 4)
pub fn default_jobs() -> usize;

/// Reports progress through the callback
pub fn run_batch(
    files: &[PathBuf],
    out_dir: &Path,
    s: &Settings,
    jobs: usize,
    cancel: &AtomicBool,
    on_event: impl Fn(BatchEvent) + Sync,
) -> BatchSummary;

pub enum BatchEvent {
    Started { index: usize },
    Finished { index: usize, report: FileReport },
    Failed { index: usize, error: ProcessError },
    Cancelled,
}

pub struct BatchSummary {
    pub succeeded: usize,
    pub failed: usize,
    pub skipped: usize,   // not started because of a cancel
    pub input_bytes: u64,
    pub output_bytes: u64,
}
```

## 4. Conversion Pipeline

Each file goes through these steps in order:

```
read -> extract metadata -> decode -> correct orientation -> resize -> encode
     -> reinsert metadata (GPS removed) -> write to temporary file -> rename
```

### 4.1 Reading and Metadata Extraction

- Read the whole file into memory. Open the original for reading only.
- Use `img-parts` to take APP1 (EXIF) and APP2 (ICC).
- Nothing else is copied. The output is newly encoded, so XMP, IPTC, MPF, comments, and data after EOI never reach it.
- Decode in strict mode, so a truncated JPEG fails instead of producing a partly gray image.

### 4.2 Orientation

- Read EXIF Orientation (1 to 8) and rotate or flip the pixels (`orient.rs`).
- Provide tests for all 8 values.

### 4.3 Resize

- Shrink only when the long edge exceeds `long_edge`. Round the new size to the nearest integer, with a minimum of 1px.
- Do not convert the color space. Resize as is, assuming sRGB. The ICC profile is restored unchanged in §4.5.

### 4.4 Encode

- Use `mozjpeg` with `set_fastest_defaults()`, `set_optimize_coding(true)`, baseline, and 4:2:0. The scan data is then byte-identical to sharp's defaults (§11), so N-06 needs no quality mapping.
- Do not use the crate defaults. They are smaller but 7 to 23 times slower, and progressive.
- `nasm` must be on `PATH` when building. Without it the build still succeeds, with SIMD off and encoding about half as fast.
- Keep the encoder behind `trait Encoder` so it can be swapped.
- Write the ICC profile with `write_marker`. `write_icc_profile` numbers the chunks from 0, and decoders ignore such a profile (§11).
- Grayscale stays grayscale. CMYK and YCCK are rejected (§8).

### 4.5 Metadata Reinsertion (Location Removal)

`metadata.rs` builds a **new** TIFF and inserts it into the output as APP1. It reads IFD0, ExifIFD, and InteropIFD into memory and copies only the kept entries, in the original endianness. Bytes not copied this way cannot reach the output, including areas no IFD references. It never follows an IFD `next` link, so pointer cycles cannot occur.

| Target | Handling |
|---|---|
| GPS IFD | **Remove entirely**, including the pointer to it in IFD0 (0x8825). |
| Orientation | Rewrite to 1, because the pixels are already rotated. |
| PixelXDimension / PixelYDimension | Rewrite to the output size. Do not add them if absent. |
| IFD1 (embedded thumbnail) | Remove. It is the unrotated image and may contain location. |
| MakerNote (0x927C) | Remove. Its offsets break when EXIF is rebuilt, and it can hold location text. |
| XMP (0x02BC), IPTC (0x83BB), Photoshop (0x8649) | Remove. These tags can carry location inside EXIF. |
| Tags that hold file offsets (strips, tiles, free space, SubIFDs, thumbnail, DNGPrivateData) | Remove. The offsets would point to the wrong place. |
| Entries of an unknown type | Remove. Their size is unknown. |
| Everything else | Keep, with type, count, and value unchanged. |

- Restore the ICC profile unchanged as APP2.
- If EXIF parsing fails, **output without EXIF** and report the file as a warning. Losing EXIF is preferable to leaving location data.

### 4.6 Writing

- Write to `export/<file name>.tmp`, then rename to `export/<file name>`.
- This leaves no partially written file after a cancel or an error. Delete the `.tmp` on failure.

## 5. Parallelism and Cancellation (batch)

- Create a thread pool with `jobs` threads using `rayon::ThreadPoolBuilder`.
- Check `cancel` before each task starts. If set, the task does not start. Running tasks finish.
- Report progress through the `on_event` callback. The GUI sends events to an `mpsc::Sender` there and calls `egui::Context::request_repaint()`.
- Memory estimate: a 24-megapixel image is about 72MB decoded to RGB. A running task also holds the resized image, encoder buffers, and the file bytes, so each task measured 1.2 to 1.6 times its decoded image (§11). `jobs = 4` with 24-megapixel photos peaked at 383MB. Do not make a heavier configuration the default.

## 6. Development Example (crates/core/examples)

`crates/core/examples/convert.rs` runs `run_batch` on a folder without the GUI. It exists for the comparison with the old version and for `exiftool` checks in phase 1.

- It is not distributed and is not part of the product.
- It takes a folder, an optional preset name, and an optional job count, read with `std::env::args`. It uses no argument parsing crate.
- It prints one line per file and the totals, in English, with no translations.

## 7. GUI (crates/gui)

### 7.1 Screen Layout

The window is a single screen with four areas, from top to bottom. Labels below are the English text. A language selector, `日本語` and `English` with no label, sits at the top left, above the Input area.

1. **Input**: a drop area with a `Select folder` button to its right. The drop area shows `Drop a folder to select it` until a folder is set, then the folder path (e.g. `C:\Photos\2026-08-04`), shortened if it does not fit. Below them, only a folder error or `No JPEG files found` is shown.
2. **Settings**: radio buttons for the presets `low`, `default`, and `high`, followed by a `Custom` marker shown when the values match no preset. Below them, a quality slider with its value, then a number input for the long edge in px, labeled `Long edge`, and a number input labeled `Concurrent files`.
3. **File list**: a table with the columns `File name`, `Original size`, `Output size`, and `Status`. `Output size` is empty until the file is done. `Status` shows `Waiting`, `Processing...`, `Done`, `Done (warning)`, `Failed`, or `Cancelled`, without symbols: `✗` is in neither egui's font nor the Japanese font and showed as a box. Hovering a warning or failure shows the reason.
4. **Progress and actions**: a progress bar with the processed and total counts (e.g. `12 / 42`), with the `Start` button, or the `Cancel` button while running, to its right. Below them, on completion, the summary (e.g. `Done: 42 files / 980MB -> 71MB / 18.3s  0 failed`) and an `Open output folder` link. Files that failed or had warnings are listed below the summary with their reasons.

- Phase 3 adds a `Preview` button to the left of `Start` and opens the preview in a separate window (§7.5).
- The file list uses `egui_extras::TableBuilder` and draws only the visible rows.
- The progress bar fill is transparent while the count is 0, because egui draws a fill as wide as the bar is tall even at 0.

### 7.2 State

```rust
struct App {
    lang: Lang,
    has_japanese_font: bool,
    input_dir: Option<PathBuf>,
    scan_error: Option<ScanError>,
    rows: Vec<FileRow>,           // path, name, input_bytes, output_bytes, status
    settings: Settings,
    jobs: usize,
    running: Option<Running>,     // receiver, cancel flag, start time
    finished: Option<Finished>,   // BatchSummary, elapsed time, cancelled
    selected: Option<usize>,      // row shown in the preview
    preview: PreviewWindow,       // open flag, preview thread, textures, zoom (§7.5)
    // Persist only settings / jobs / lang
}
```

- The preset is not stored. The selected preset is the one whose settings equal the current values; if none matches, the settings are custom. This keeps the radio buttons and the values from disagreeing.
- A worker thread started with `std::thread::Builder` calls `wio_core::run_batch` and sends each `BatchEvent`, then the `BatchSummary`, through an `mpsc` channel. It calls `request_repaint()` after each message, so the UI repaints only when something changes. The UI thread drains the channel with `try_recv()` every frame.
- While `running` is set, the conversion settings and input controls are disabled. The language selector stays enabled.
- `FileRow` keeps its status as an enum and failures as `ProcessError`, not as display text, so a language change also applies to rows already shown.
- On the first launch, `lang` comes from `Lang::detect()`. After that, the value saved through eframe persistence is used.

### 7.3 Japanese Fonts

- egui's default fonts have no Japanese, so **load an OS font at startup**. Not bundling one keeps the distribution small.
- Load it in English mode too, because file names and paths can contain Japanese.
- Windows: look in `C:\Windows\Fonts\` for `YuGothM.ttc`, `meiryo.ttc`, `msgothic.ttc`, in that order.
- macOS / Linux: look through the usual paths (Hiragino, Noto Sans CJK).
- If none is found, start with the default fonts, use English regardless of the selected language, and log a warning.
- Add the font as the lowest-priority fallback, so Latin text keeps egui's font. Yu Gothic draws higher than egui's Latin font, so it gets `FontTweak::y_offset_factor = 0.25`, a value matched on screen.

### 7.4 Other

- Put `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` at the top of `main.rs` so release builds show no console.
- Receive drag and drop with `ctx.input(|i| i.raw.dropped_files.clone())`. If something other than a folder is dropped, use its parent folder as the input.
- "Open output folder" uses `explorer` on Windows, `open` on macOS, and `xdg-open` on Linux.

### 7.5 Preview (Phase 3)

- Clicking a row in the file list selects it. The `Preview` button, enabled while a row is selected, or a double-click on a row opens the preview in a separate window (an egui immediate viewport, 1200×800 at first). The main window keeps the phase 2 layout, and the preview window can be resized or maximized on its own. While it is open, selecting another row or changing the settings updates it. The close button or Esc closes it. With no row selected, it shows a hint to select a file.
- An earlier version placed the preview inside the main window, on the right or below the list. It was dropped: the panes were too small for a 100% view, and a small window made the preview overlap the list.
- For the selected file, run `wio_core::preview` on a dedicated preview thread. It returns the image before encoding (resized and rotated) and the encoded JPEG decoded again, both at the output size. Comparing these two shows only the JPEG compression loss. The original file's pixels are not used: at a different scale, the same 100% view would cover a different area.
- Show the two images, before first and after second, side by side or stacked, whichever shows the image larger in the window. `Fit` shows the whole image; `100%` maps one image pixel to one screen pixel. In `100%`, dragging either image moves both to the same position. Clicking an image in `Fit` switches to `100%` centered on that point.
- Show the original size and the output size, in the same `before -> after` form as the batch summary (e.g. `File size: 5.6MB -> 1.1MB`). The output size is the exact size `process_file` would write.
- The preview thread runs one request at a time. When a request finishes, it takes only the newest waiting request and drops the older ones. Results for an outdated request are discarded. This keeps memory at one conversion (N-04).
- Selecting another file starts a new request at once. Changing quality or long edge starts one 300ms after the last change, to avoid recalculating continuously while the slider moves.
- During a batch, no new request is sent; the window keeps the last result and says it will update after the batch. A request already running when the batch starts finishes.
- The preview thread also splits each image into 512×512 `ColorImage` tiles, padding the edge tiles, so the UI thread only uploads them. All textures have the same size because the GPU driver keeps freed texture memory per size: one texture per photo grew memory with every photo of a different size, while same-size tiles are reused (§11). When a request is sent, the textures on screen are released, and until the result arrives each pane shows a skeleton: a pulsing rectangle at the image's position and size, with the previous image's aspect ratio when the file changed. The file size line stays. Holding two sets of textures cost about 80MB more (§11). A result is loaded in the frame after it arrives. Textures use no mipmaps: they smooth the `Fit` view but cost about 30MB more. Closing the window drops all textures and results. An image larger than the GPU's maximum texture size is not shown, because it would use too much memory; the window says so and still shows the file sizes.

## 8. Error Handling

| Situation | Handling |
|---|---|
| Folder does not exist or cannot be read | Show an error without starting. |
| JPEG decoding fails | Record the file as failed and continue. |
| EXIF parsing fails, or the rebuilt EXIF does not fit in APP1 | Output without EXIF and warn (§4.5). |
| JPEG segments cannot be read but the image decodes | Output without EXIF and ICC, and warn. |
| CMYK or YCCK JPEG | Record the file as failed. A plain conversion to RGB would change the colors and invalidate the ICC profile. |
| More than 500 megapixels | Record the file as failed before allocating. A failed allocation stops the process instead of unwinding. |
| Writing fails (disk full, permissions, etc.) | Record the file as failed and delete the `.tmp`. |
| Panic | `core` does not panic (no `unwrap`). `mozjpeg` reports errors by unwinding, so encoding runs inside `catch_unwind`. `run_batch` also wraps each file in `catch_unwind` and treats a panic as a per-file failure. |

## 9. Build Settings

Put a release profile that keeps distribution size small in the workspace `Cargo.toml`.

```toml
[profile.release]
opt-level = 3        # prioritize image processing speed
lto = "fat"
codegen-units = 1
strip = true
panic = "unwind"     # mozjpeg reports errors by unwinding
```

- `.cargo/config.toml` sets `+crt-static` for `x86_64-pc-windows-msvc`, so the executable needs no VC++ runtime (N-02).
- Set `default-features = false` on eframe and enable only what is needed, such as `glow`, `default_fonts`, and `persistence`.

### 9.1 CI and Release

Both workflows run on `windows-latest`, install the Rust version pinned in `rust-toolchain.toml`, and install NASM with Chocolatey, adding `C:\Program Files\NASM` to `PATH`.

- `.github/workflows/ci.yml` runs on pushes to `main` and on pull requests: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace`.
- `.github/workflows/release.yml` runs when a tag `v*` is pushed. It fails if the tag differs from `v` plus the workspace version, runs the tests, and builds `wio-gui` in release mode.
- Without NASM, mozjpeg still builds and only prints a build script warning that Cargo hides for registry dependencies. The release workflow therefore checks that `target/release/build/mozjpeg-sys-*/out/include/jconfig.h` contains `#define WITH_SIMD 1`, and fails otherwise.
- It copies the executable to `web-image-optimizer-<version>-windows-x64.exe` and creates a **draft** release with generated notes. The author publishes it after checking.
- The Rust version is pinned so that lints added in a new stable release do not break CI unannounced. To upgrade, change `rust-toolchain.toml` and make `cargo clippy` pass in the same change.
- To release: bump `version` in the workspace `Cargo.toml`, merge, then push the tag (e.g. `git tag v0.1.0` and `git push origin v0.1.0`).

## 10. Tests

- **Unit tests**: presets, settings validation, resized dimensions (no enlargement, 1px minimum), all 8 Orientation conversions, OS language mapping in `Lang::detect()`.
- **Metadata tests**: after processing EXIF containing GPS, confirm the GPS IFD and its pointer are gone, Orientation is 1, IFD1 is gone, and capture date and similar fields remain, in both endians. Also confirm that marker strings placed in each removed area do not remain in the output bytes, and that pointer cycles and randomly corrupted EXIF end without a panic.
- **Integration tests** (`crates/core/tests/conversion.rs`): convert generated JPEGs, reload the output, and confirm the above. Also confirm the originals are unchanged, that cancelling leaves no partial file, and that the ICC profile survives.
- **Test images**: generated by test code, so they are free of copyright and personal information. At least five kinds: with GPS, with location in XMP, Orientation=6, small long edge, corrupt file.
- **Manual check**: `exiftool -a -G1 -gps:all -xmp:all export/*.jpg` prints nothing.

## 11. Decisions and Measurements

Append decisions and measurement results made during implementation.

| Date | Content |
|---|---|
| 2026-11-06 | Build check: `mozjpeg` 0.10.13 (`mozjpeg-sys` 2.2.3) builds on Windows 11 with Rust 1.96 and Visual Studio Community 2026, from a plain shell. CMake is not needed. |
| 2026-11-06 | nasm: without it the build still succeeds, but SIMD is silently off (`cargo:warning=NASM not installed`, `jsimd_none` is linked). The speed with SIMD has not been measured. |
| 2026-11-06 | Encoder output, same RGB pixels, quality 72/82/90, one 1600×1067 photo and one synthetic 2400×1600 image: `set_fastest_defaults()` + `set_optimize_coding(true)` + 4:2:0 gives scan data byte-identical to sharp 0.35.3 defaults; the file is 18 bytes larger (JFIF header). The crate defaults are progressive with trellis, 7.2 to 15.9% smaller than sharp, and 7 to 23 times slower than the fastest settings. sharp bundles mozjpeg 5.0.0 but its defaults turn the mozjpeg features off. |
| 2026-11-06 | Encode time without SIMD, 1600×1067, quality 82: fastest settings 29.6ms, crate defaults 234.4ms, sharp (with its SIMD, `concurrency(1)`) 14.8ms. |
| 2026-11-06 | DLLs (N-02): a default build imports `VCRUNTIME140.dll` and `api-ms-win-crt-*`. With `-C target-feature=+crt-static`, the `mozjpeg` test binary imports only OS DLLs (`KERNEL32`, `ntdll`, `api-ms-win-core-synch`), and grows from 563KB to 708KB. |
| 2026-11-06 | `little_exif` 0.6.23, checked with generated fixtures and an independent TIFF parser. Removing the tags leaves the 0x8825 pointer and an empty GPS IFD and IFD1, because no public API removes an IFD. Copying the wanted IFDs into a new `Metadata` removes all three. The copy turns big-endian into little-endian unless a minimal TIFF is read first to seed the endianness. Orientation can be rewritten. All 18 other tags in IFD0, ExifIFD, and InteropIFD keep their type, count, and value when the endianness is kept. |
| 2026-11-06 | `little_exif` robustness: re-serializing moves the MakerNote, so absolute offsets inside it break with every method. An IFD `next` pointer to itself never returns (no visited check in `Metadata::decode`; `ifds` grows without limit). 2 of 30,000 randomly corrupted EXIF blocks panicked (`assert_eq!` at `exif_tag/decode.rs:78`). |
| 2026-11-06 | `mozjpeg` reports errors by unwinding; its README says any JPEG error aborts the process under `panic=abort`. |
| 2026-11-06 | Spec change: the GUI and CLI display either Japanese or English (requirements §4.5). Text lives in the new crate `wio-i18n` (§1.1). `BatchEvent::Failed` carries `ProcessError` instead of `String`, and `clap` uses the builder API instead of derive. |
| 2026-11-06 | Spec change: the CLI is removed; the GUI is the only interface. This replaces the `clap` and `wio-i18n` plans in the row above. Text moves to `crates/gui/src/i18n/`, the language is saved with eframe persistence, and phase 1 uses the development example in §6. Concurrency and the failed file list move to the GUI (G-04, G-07). |
| 2026-11-06 | Decision: encode with `mozjpeg` using `set_fastest_defaults()` + `set_optimize_coding(true)`. |
| 2026-11-06 | Decision: build with nasm. nasm 3.02 works with `nasm-rs` 0.3.2. Its installer does not add it to `PATH`. Encode time on the 1600×1067 photo, typical runs: about 37ms without SIMD, 18 to 22ms with SIMD, 19 to 20ms for sharp. Output size is unchanged. Timings on this PC vary between runs. |
| 2026-11-06 | Decision: drop `little_exif` and write the EXIF handling in `metadata.rs`. |
| 2026-11-06 | Decision: remove MakerNote from the output. Also remove the XMP, IPTC, and Photoshop tags in IFD0, which the first design missed. |
| 2026-11-06 | Decision: `panic = "unwind"` with `catch_unwind`, and `+crt-static`. |
| 2026-11-06 | `mozjpeg` 0.10.13 bug: `CompressStarted::write_icc_profile` numbers chunks from 0, but ICC numbering starts at 1. `zune-jpeg` ignores such a profile, so colors could silently change. The ICC profile is written with `write_marker` instead. |
| 2026-11-06 | Test images are generated in code, so `tests/fixtures/` is not used. The `Warning` type and `BatchSummary` are defined in §3. |
| 2026-11-06 | A truncated JPEG stops the old version at that file, without processing the rest. This tool records it as failed and continues. |
| 2026-11-08 | Comparison with the old version on 13 real photos (FUJIFILM X-T5 exported from Lightroom, 129.2MB, 4273×5341 to 7728×5152, 2 of them Display P3). Total output size against the old version: low -13.9%, default -5.5%, high -2.8%. Most of this is metadata: the old version keeps XMP and IPTC (about 38KB per file), this tool about 4KB. Image data alone (SOS to EOI): low +0.9%, default +0.6%, high -0.1%, per file -1.2% to +2.4%. Output dimensions match in every file. |
| 2026-11-08 | Time on the same 13 photos, two runs each, old version / this tool at 4 jobs / this tool at 1 job: low 7.9s / 1.6 to 1.9s / 6.1 to 6.6s, default 10.6 to 11.1s / 1.7 to 2.3s / 6.0 to 7.1s, high 15.0s / 2.0s / 6.4 to 9.0s. Originals were unchanged. |
| 2026-11-08 | Location check with exiftool 13.61 on copies of the same photos, with fake GPS written to EXIF and XMP and Orientation set to 6 and 8 on two files: `exiftool -a -G1 -gps:all -xmp:all` prints nothing for this tool's output, and IPTC, Photoshop, IFD1, and MakerNotes are absent. Model, DateTimeOriginal, ExposureTime, FNumber, ISO, and LensModel remain in all 13 files, and both Display P3 profiles remain. The old version's output keeps 24 XMP GPS tags, because it removes only the EXIF GPS IFD. The rotated files match the old version's orientation. |
| 2026-11-08 | `exiftool -validate` reports the same 4 warnings for this tool's output as for the originals (missing YCbCrPositioning, ComponentsConfiguration, ExifImageWidth, ExifImageHeight in Lightroom exports). Absent tags are not added (§4.5). |
| 2026-11-08 | GUI release build: 10.4MB (N-03), Windows GUI subsystem (G-10), imports only OS DLLs. Idle memory on this PC (Intel GPU): working set 146MB, private 106MB, above the 50MB target of N-04. A minimal eframe 0.37 glow app with nothing but a label uses 127 to 133MB working set and 92MB private; the Intel OpenGL driver DLLs alone map 84MB. The Japanese font adds about 13MB private, and the rest of this app about 6MB. |
| 2026-11-08 | Peak memory on the same photos (up to 40 megapixels): 181MB at 1 job, 563MB at 4 jobs. With 24-megapixel files: 113MB and 383MB. This is above the 300MB estimate in §5 and about 1.2 times concurrency × one decoded image (N-04). |
| 2026-11-08 | Spec change: N-04 no longer sets an absolute idle target. eframe with glow loads the GPU driver at startup, so 50MB cannot be met, and the absolute value depends on the GPU and driver. Idle is now compared with a minimal eframe app on the same PC (this app: +14MB private). The peak bound is 2 × concurrency × one decoded image, because the measured ratio is 1.2 at 4 jobs and up to 1.6 at 1 job. |
| 2026-11-09 | Spec change: the preview opens in a separate window from a `Preview` button or a row double-click (G-11, §7.5). A first version placed it inside the main window, on the right or below the list; the panes were too small for a 100% view, and a small window made the preview overlap the list. |
| 2026-11-09 | Preview memory on the ZN6 photos (3200 px long edge, so two 3200×2133 textures), private bytes: idle 105 to 106MB. Holding the old textures while generating: about 249MB with an image shown, 496MB at peak. Releasing them when a request is sent did not lower the measured peak (590MB), which led to the texture experiment below. Closing the preview window returned to 109 to 116MB. |
| 2026-11-09 | Texture experiment: a minimal eframe 0.37 glow app loads two 3200×2133 RGBA textures (55MB) once a second, 40 times, on the Intel GPU of this PC (minimal app idle: 92MB private). Loading the new pair before releasing the old one: 304MB floor, 377MB peak. Releasing first: 224MB floor, 296MB peak. Releasing first without mipmaps: 196MB floor, 268MB peak. After all textures are released the value stays at the floor: the driver keeps the memory and reuses it, so it does not grow across cycles. Shown textures cost about 2.4 times their RGBA size. This led to adding the texture term to N-04. |
| 2026-11-09 | Same experiment at 300ms intervals, without mipmaps, releasing first. Fixed 3200×2133: 197 to 270MB. Reusing the same two textures with `TextureHandle::set`: 302 to 376MB. Sizes changing every cycle as in ZN6 (3200×2133, 3200×2128, 2133×3200, 2560×3200): 535 to 672MB, 469MB after release. Fixed 3200×3200 textures updated with `set_partial`: 391 to 556MB. The same changing sizes split into 512×512 tiles: 239 to 329MB, peak 341MB; 256×256 tiles: 246 to 345MB. `wio_core::preview` alone, 78 calls on the ZN6 copies: 198MB peak, back to 2MB after each call. |
| 2026-11-09 | Decision on NASM in CI (was an open question): both workflows install it with Chocolatey, and the release workflow fails unless mozjpeg's `jconfig.h` has `#define WITH_SIMD 1` (§9.1). The first release run for `v0.1.0` passed every step on `windows-latest`: tests 107s, release build 267s, then the draft release. |
| 2026-11-09 | The first CI run used stable 1.99.1 while development used 1.96.0, and failed on the new clippy lint `chunks_exact_to_as_chunks`. Decision: pin Rust 1.99.1 with `rust-toolchain.toml` for both. |
| 2026-11-09 | Decision: upload preview images as 512×512 tiles. The GUI, driven through the same file and quality changes 59 times at 700ms intervals on the ZN6 copies: one texture per image peaked at 681MB with a floor that climbed to 368MB; tiles peaked at 498MB (392MB over idle) with a floor of 252 to 311MB, falling after the window closed. The N-04 bound for this case is 2 × 119MB + 4 × 55MB = 458MB over idle. |
| 2026-11-10 | The published `v0.1.0` executable, downloaded from Releases, started on another Windows 11 PC without development tools (acceptance criterion: no Node.js needed). |

## 12. Open Questions

Move an item to §11 once it is decided.

- **App icon**: the user will provide it later. Until then the window keeps eframe's default icon and the executable has none.
