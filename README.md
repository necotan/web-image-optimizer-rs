<h1 align="center">web-image-optimizer-rs</h1>

<p align="center">
  English | <a href="README.ja.md">日本語</a>
</p>

<p align="center">
  Batch-resize and compress full-size JPEGs for the web, in a desktop app.<br>
  Location data is removed automatically, while capture date, camera, and exposure settings are kept.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Rust-2024-000000?logo=rust&logoColor=white">
  <img src="https://img.shields.io/badge/GUI-egui-4D4D4D">
  <img src="https://img.shields.io/badge/Windows-10%20%7C%2011-0078D4">
  <br>
  <img src="https://img.shields.io/badge/Privacy-Location%20Removal-E05D44">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-green"></a>
</p>

> This is the successor to [web-image-optimizer](https://github.com/necotan/web-image-optimizer), the Node.js command-line version. It does the same conversion as a single executable with a GUI, so Node.js, `git clone`, and `npm install` are no longer needed.

## Overview

- Drop a folder onto the window, and it processes all `.jpg` / `.jpeg` files directly inside it.
- Images are resized and compressed to the chosen preset (`low` / `default` / `high`) or your own values, and written with the same names to an `export/` subfolder.
- The original files are never modified.
- Location data is removed from the output. Other EXIF data and the color profile are kept.
- A preview window shows the image before and after compression side by side, at 100% if you like, with the original and output file sizes.
- The display language can be switched between Japanese and English.

## Download

Download `web-image-optimizer-<version>-windows-x64.exe` from [Releases](https://github.com/necotan/web-image-optimizer-rs/releases/latest). It is a single file: no installer and no runtime.

The executable is not code-signed, so Windows SmartScreen may show "Windows protected your PC" on the first run. Click **More info**, then **Run anyway**.

## Usage

1. Start the executable.
2. Drop a folder of photos onto the window, or click **Select folder**. The JPEGs in it are listed.
3. Choose a preset. Quality and long edge can also be changed individually.
4. Click **Start**. The converted files are written to `export\` inside the selected folder.
5. When it finishes, the totals and any failed files are shown. **Open output folder** opens `export\`.

To check the result before converting, select a file in the list and click **Preview**, or double-click the file. The preview window updates when you select another file or change the settings.

Settings, the display language, and the window size are restored on the next launch. They are stored in `%APPDATA%\web-image-optimizer-rs\`.

## Presets

| Preset | Long edge | Quality |
|---|---|---|
| `low` | 1600px | 72 |
| `default` | 2400px | 82 |
| `high` | 3200px | 90 |

Images whose long edge is already at or below the setting are not enlarged; they are only re-encoded.

## What is kept and what is removed

| | |
|---|---|
| Kept | Capture date and time, camera and lens, exposure settings, other EXIF tags, the ICC color profile |
| Removed | GPS data, XMP, IPTC, Photoshop data, the maker note, the embedded thumbnail |

XMP, IPTC, and the maker note are removed because they can also contain location. The old version kept XMP, including any location written there.

Photos taken in portrait are rotated so they display upright everywhere.

To check that no location data remains, run [ExifTool](https://exiftool.org/) on the output. It should print nothing:

```
exiftool -a -G1 -gps:all -xmp:all export\*.jpg
```

## Notes

- Only `.jpg` / `.jpeg` files directly inside the folder are processed. Subfolders are not.
- Files in `export\` with the same name are overwritten.
- If a file cannot be converted (for example, a broken file or a CMYK JPEG), the others are still processed, and the failure is listed with its reason.
- Conversion can be cancelled. Files in progress are finished, and no partial file is left behind.

## Building from source

Requirements: [Rust](https://rustup.rs/) (stable) and [NASM](https://www.nasm.us/) on `PATH`. Without NASM the build still succeeds, but JPEG encoding is about twice as slow.

```
git clone https://github.com/necotan/web-image-optimizer-rs.git
cd web-image-optimizer-rs
cargo build --release -p wio-gui
```

The executable is `target\release\wio-gui.exe`. The specification is in [`docs/requirements.md`](docs/requirements.md) and [`docs/design.md`](docs/design.md).

To commit changes, enable the Git hooks once after cloning. `pre-commit` runs `cargo fmt --check`, `cargo clippy`, and `scripts/check.sh`, and `commit-msg` checks the format of the first line.

```
git config core.hooksPath .githooks
```

## License

[MIT](LICENSE)
