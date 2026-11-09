# CLAUDE.md

## Language rules

- Respond to the user in Japanese.
- Commit messages: first line in English, all following lines in Japanese.
- Write code comments in Japanese.
- Use English for variable names, function names, and file names.
- Write `docs/` in English, including comments in sample code.
- Write `crates/gui/src/i18n/ja.rs` values in Japanese and `crates/gui/src/i18n/en.rs` values in English.
- Write `README.md` in English and `README.ja.md` in Japanese, and change both in the same change.

## Project

web-image-optimizer-rs: a tool that batch-resizes and compresses a folder of JPEG photos for the web and removes location data. Rust, with an egui GUI. It succeeds the Node.js (sharp) tool [web-image-optimizer](https://github.com/necotan/web-image-optimizer), called the "old version" below.

The specs are the source of truth for what to build; read the relevant section before implementing. This file covers how to work.

- `docs/requirements.md`: purpose, scope, functional and non-functional requirements, acceptance criteria, phases. Read it when changing what the tool does.
- `docs/design.md`: crates, core API, conversion pipeline, metadata handling, batch, GUI, decision log, open questions. Read the section for the area you change.

## Commands

`nasm` must be on `PATH` for every build. Without it the build still succeeds, but `mozjpeg` loses SIMD (design §4.4).

- `cargo build`: build
- `cargo test --workspace`: tests
- `cargo clippy --workspace -- -D warnings`: lint
- `cargo fmt --all`: format
- `cargo run -p wio-gui`: run the GUI
- `cargo run --release -p wio-core --example convert -- "<folder>" high`: run the conversion without the GUI (development only)
- `cargo build --release -p wio-gui`: distribution build

## After every change

- Run `cargo fmt --all`, `cargo clippy --workspace -- -D warnings`, and `cargo test --workspace`. All must pass before proposing a commit.
- If the change touches location data removal, add a test and run it. See Location data below.
- If behavior or a decision changed, update the spec in the same change. Record decisions and measurements in `docs/design.md` §11.

## Layout

- `crates/core/`: library `wio-core`. All conversion logic. `examples/convert.rs` is a development runner and is not distributed.
- `crates/gui/`: package `wio-gui`, executable `wio-gui`. `src/i18n/` holds all user-facing text, in Japanese and English.
- `crates/core/tests/`: integration tests. Test JPEGs are generated in code.
- `docs/`: only the two specs are tracked; anything else there is git-ignored local notes.
- `.github/workflows/`: CI and the draft release build (design §9.1).
- File-level layout: `docs/design.md` §1.

## Rules

### Phases

- Work one phase at a time (requirements §7). Do not start the next phase's features during a phase.

### Design and dependencies

- MUST consult the user, with the reason, before implementing anything that differs from `docs/design.md`. Record the outcome in §11.
- Ask first before adding a crate that is not in design §2 or replacing one that is.
- Do not pin crate versions by hand. Add crates with `cargo add` to get the latest stable version, and check the API on docs.rs before using it.

### Location data

- Removing location data is the most important feature of this tool (requirements F-05). MUST add and run a test for every change that affects it.
- When EXIF cannot be parsed, write the output without EXIF. NEVER let a parsing failure pass original metadata through (design §4.5).

### core

- `core` MUST NOT depend on any UI, including egui and printing to stdout.
- `core` MUST NOT hold user-facing text. Return typed errors and events.
- Do not use `unwrap()` or `expect()` in `core`, except in test code. Define error types with `thiserror`.

### i18n

- NEVER hard-code user-facing text in the GUI. Add it to `Strings` in `crates/gui/src/i18n/`, with both `ja.rs` and `en.rs` in the same change.
- Keep state as enums and typed errors, not display text, so that a language change applies to what is already on screen.

### Old version

- It is a separate repository. To check its behavior, read [`resize.mjs`](https://github.com/necotan/web-image-optimizer/blob/main/resize.mjs).
- To compare output size and processing time, clone it outside this repository and run it there. NEVER bring Node.js files into this repository.

### Coding

- Rust edition 2024. The Rust version is pinned in `rust-toolchain.toml`; to upgrade, change it there and fix new clippy warnings in the same change.

### Comments

- Write only why: the reason for a choice, workarounds for library or OS quirks, and constraints the code alone does not show.
- Do not write what the code or names already say (e.g. "リサイズ処理", "ファイルを読み込む"). If there is no why to explain, write no comment.
- No decorative characters such as `---`, `===`, `***`.

## Git workflow

- Do not run `git init`, `git checkout -b`, `commit`, `push`, PR creation, `merge`, or `pull`. The user runs them.
- At the start of work, propose a branch name (`feature/xxx` or `fix/xxx`).
- After code changes, run `git status` and `git diff` (plus `git diff --staged` if anything is staged), then propose a commit message based on the actual diff.

## Commit messages

| Prefix | Use |
|---|---|
| fix | Bug fix |
| hotfix | Critical bug fix |
| add | New file or feature |
| update | Feature change (not a bug) |
| change | Spec change |
| clean | Cleanup, refactoring |
| disable | Disable, comment out |
| remove | Remove a file or feature |
| upgrade | Version upgrade |
| revert | Revert a change |

- Line 1: `<prefix>: <summary in English>`.
- The body is optional. For a small change that line 1 fully describes, write line 1 only. Otherwise add a blank line and 1 to about 4 Japanese bullets; do not pad to reach 4.
- Bullets name the concrete files, types, or functions changed and how. Do not explain causes or background unless the user asks.

```
fix: typo in GUI label text
```

```
add: GPS stripping for EXIF

- crates/core/src/metadata.rs に GPS IFD と 0x8825 ポインタの削除を追加
- Orientation を 1 に書き換え、IFD1 を削除
- GPS 入り EXIF の加工結果を確認するテストを追加
```
