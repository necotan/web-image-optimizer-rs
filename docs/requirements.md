# Requirements Specification

This document is the source of truth for what web-image-optimizer-rs does and why. How it is built is in `docs/design.md`. How to work in this repository is in `CLAUDE.md`.

## Glossary

| Term | Code | Meaning |
|---|---|---|
| Old version | | The Node.js (sharp) tool [web-image-optimizer](https://github.com/necotan/web-image-optimizer), `resize.mjs`. |
| Input folder | | The folder the user specifies. Only JPEGs directly inside it are processed. |
| `export/` | | The subfolder of the input folder that receives the output. |
| Long edge | `Settings::long_edge` | The longer of width and height, in pixels. |
| Preset | `Preset` | A named pair of long edge and quality (see 4.2). |
| Custom | | The display name of the settings when the long edge or quality is overridden. |
| Location data | | The EXIF GPS IFD, and XMP, which may contain location (see F-05). |
| Display language | `Lang` | The language of user-facing text: Japanese (`ja`) or English (`en`) (see 4.5). |

## 1. Purpose

Rewrite in Rust a tool that batch-resizes and compresses full-size JPEG photos for web publishing. This repository is the **successor** to the old version. It keeps the old version's conversion features, replaces its CLI with a GUI, and adds three things:

1. **Distribution as a single executable**: no Node.js install, `git clone`, `npm install`, or `cd`.
2. **Operation through a GUI (egui)**: folder drag and drop, preset selection, progress display, preview.
3. **Light weight**: small distribution size and small runtime memory.

## 2. Users and Usage

- The main user is the author. Friends may also receive the executable and use it.
- A folder of photos (e.g. `C:\Photos\2026-08-04`) is converted in one batch before posting to blogs, social media, or photo sharing sites.
- Not publishing location data (such as a home address) by mistake is important.

## 3. Scope

### In scope

- Input: `.jpg` / `.jpeg` files **directly inside** the specified folder (extension case-insensitive)
- Output: JPEGs with the original file names in the `export/` subfolder of the input folder
- GUI app (egui / eframe)
- Display in Japanese and English, selectable by the user

### Out of scope

- Recursing into subfolders
- Input formats other than JPEG (HEIC, PNG, RAW, etc.)
- WebP / AVIF output
- Image editing (cropping, color correction, watermarks, etc.)
- An installer (the executable is distributed on its own)
- Display languages other than Japanese and English
- A user-facing command line (see 4.4)

## 4. Functional Requirements

### 4.1 Conversion (common)

| ID | Requirement |
|---|---|
| F-01 | Shrink so the long edge is at or below the specified pixel count, keeping the aspect ratio (fit inside). |
| F-02 | If the original long edge is at or below the specified value, do **not enlarge** (re-encode only). |
| F-03 | Re-encode as JPEG at the specified quality (1 to 100). |
| F-04 | **Rotate the pixels** according to EXIF Orientation and set the output Orientation to 1 (upright). |
| F-05 | **Completely remove location data** from the output (remove the entire EXIF GPS IFD; do not output XMP because it may contain location). |
| F-06 | Keep EXIF other than location data (capture date, camera model, lens, exposure settings, etc.). |
| F-07 | Keep the ICC profile so colors do not change. |
| F-08 | **Never modify the original files.** Treat them as read-only. |
| F-09 | Create `export/` if it does not exist. Overwrite a file with the same name if one exists (same as the old version). |
| F-10 | A failure on one file does not stop the whole run. Record failed files with their reasons and list them at the end. |
| F-11 | Process multiple files in parallel. Concurrency is configurable, with a default of `min(CPU cores, 4)` to limit memory use. |
| F-12 | Processing can be cancelled. It stops when the files in progress finish, and leaves no partially written file. |

### 4.2 Presets

| Preset | Long edge | Quality |
|---|---|---|
| `low` | 1600px | 72 |
| `default` | 2400px | 82 |
| `high` | 3200px | 90 |

Quality and long edge can be overridden individually. When overridden, the display name is `Custom`.

### 4.3 GUI

| ID | Requirement |
|---|---|
| G-01 | The input folder can be set by dragging and dropping a folder onto the window. |
| G-02 | The input folder can be set from a dialog opened with a "select folder" button. |
| G-03 | Show the list of target files (file name, original size, output size, status). |
| G-04 | A preset can be selected. Quality can be overridden with a slider and long edge with a number input. Concurrency (F-11) can be set with a number input. |
| G-05 | A "start" button begins processing and shows an overall progress bar and each file's status. |
| G-06 | Show a "cancel" button during processing. Conversion settings cannot be changed and a rerun cannot be started. |
| G-07 | On completion, show the count, total size (before and after), elapsed time, failure count, and the failed files with their reasons (F-10), plus a button that opens the output folder. |
| G-08 | Restore the last preset, quality, long edge, concurrency, display language, and window size on the next launch. |
| G-09 | Display Japanese correctly. |
| G-10 | On Windows, do not show a console window when the GUI starts. |
| G-11 | **(Phase 3)** For one file selected in the list, open a separate preview window from a button that shows the image before encoding and the result converted with the current settings next to each other. Support a 100% crop view, and show the estimated output size. |

### 4.4 Command Line

The tool has no user-facing command line; the GUI is the only interface. `core` stays independent of the UI, so a CLI can be added later if needed.

### 4.5 Language

| ID | Requirement |
|---|---|
| L-01 | All user-facing text, including failure reasons, is available in Japanese and English. |
| L-02 | Until the user chooses a language, the display language follows the OS: Japanese on a Japanese OS, otherwise English. |
| L-03 | The GUI has a language selector that shows each language in its own name. A change applies immediately, without a restart, and is possible during processing. |
| L-04 | A chosen language is saved and stays in effect until the user changes it (G-08). |
| L-05 | The display language does not affect the output files. |

## 5. Non-functional Requirements

| ID | Item | Target |
|---|---|---|
| N-01 | Supported OS | Windows 10/11 (x64) is primary. Keep the project buildable on macOS and Linux (running there is optional). |
| N-02 | Distribution | A single executable with no additional runtime or DLL. |
| N-03 | Distribution size | GUI release build **15MB or less** (Japanese fonts are not bundled; OS fonts are loaded). |
| N-04 | Runtime memory | Idle: private memory stays within **30MB** of a minimal eframe app (same eframe features, one label) on the same PC. For reference, the development PC shows about 150MB working set and 106MB private, most of it eframe and the GPU driver. Peak during processing: the increase over idle stays within **2 ×** concurrency × one decoded image (width × height × 3 bytes). While the preview window (G-11) is open, add the display textures, about **4 ×** the two output images in RGBA (2 × width × height × 4 bytes), because the GPU driver keeps freed texture memory for reuse. |
| N-05 | Speed | Faster than the old version (sequential) on the same PC with the same photos. |
| N-06 | Quality and compression | At the same quality value, output size stays within about **±15%** of the old version. Quality scales differ between encoders, so tune by measurement. |
| N-07 | Responsiveness | The GUI does not freeze during conversion (processing runs on a worker thread). |
| N-08 | Safety | Verify location data removal with automated tests. Also provide a manual check procedure using `exiftool`. |

## 6. Acceptance Criteria

- [ ] Converting a JPEG containing GPS data and checking the result with `exiftool -a -G1 -gps:all -xmp:all <output>` prints nothing.
- [ ] The same output, checked with `exiftool`, still has capture date, camera model, and exposure settings.
- [ ] A photo taken in portrait (Orientation=6, etc.) displays in the correct orientation in a browser.
- [ ] An image whose long edge is at or below the specified value is not enlarged.
- [ ] The hash of each original file is unchanged before and after processing.
- [ ] When a corrupt JPEG is mixed in, the other files are processed normally and it appears in the failure list.
- [ ] Cancelling during processing leaves no partially written file in `export/`.
- [ ] On a Windows PC without Node.js, double-clicking the executable starts it.
- [ ] Switching the language in the GUI changes all text immediately, and the choice is restored on the next launch.

## 7. Development Phases

| Phase | Content | Done when |
|---|---|---|
| 1 | Implement `core` with the old version's conversion features, and a development example that runs it | The example and tests meet the conversion criteria in §6. Output size and processing time are compared with the old version and recorded. |
| 2 | Implement `gui` (egui) | G-01 to G-10 and §4.5 are met. All acceptance criteria are met through the GUI. |
| 3 | Add the preview feature | G-11 is met. Memory stays within N-04 during preview. |
| 4 | Prepare distribution | GitHub Actions builds the Windows binary and attaches it to Releases. The README is complete and states at the top that this is the successor to the old version, with a link. The old version's README points to this repository, and the old version is archived. |
