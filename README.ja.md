<h1 align="center">web-image-optimizer-rs</h1>

<p align="center">
  <a href="README.md">English</a> | 日本語
</p>

<p align="center">
  フルサイズの JPEG を Web 用にまとめてリサイズ・圧縮するデスクトップアプリです。<br>
  位置情報は自動で削除し、撮影日時・カメラ・露出設定などは残します。
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Rust-2024-000000?logo=rust&logoColor=white">
  <img src="https://img.shields.io/badge/GUI-egui-4D4D4D">
  <img src="https://img.shields.io/badge/Windows-10%20%7C%2011-0078D4">
  <br>
  <img src="https://img.shields.io/badge/Privacy-Location%20Removal-E05D44">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-green"></a>
</p>

> Node.js のコマンドライン版 [web-image-optimizer](https://github.com/necotan/web-image-optimizer) の後継です。同じ変換を、GUI つきの実行ファイル 1 つで行います。Node.js のインストール、`git clone`、`npm install` は不要になりました。

## 概要

- フォルダをウィンドウにドロップすると、その直下の `.jpg` / `.jpeg` をすべて処理します。
- 選んだプリセット（`low` / `default` / `high`）または指定した値でリサイズ・圧縮し、`export/` サブフォルダに同じファイル名で書き出します。
- 元のファイルは変更しません。
- 出力からは位置情報を削除します。それ以外の EXIF と色プロファイルは残します。
- プレビューウィンドウで、圧縮前と圧縮後を並べて確認できます。100% 表示にも対応し、元のファイルサイズと変換後のファイルサイズも表示します。
- 表示言語は日本語と英語を切り替えられます。

## ダウンロード

[Releases](https://github.com/necotan/web-image-optimizer-rs/releases/latest) から `web-image-optimizer-<バージョン>-windows-x64.exe` をダウンロードしてください。ファイル 1 つだけで動き、インストールやランタイムは不要です。

実行ファイルにはコード署名がないため、初回の起動時に Windows SmartScreen の「Windows によって PC が保護されました」が表示されることがあります。その場合は **詳細情報** を押してから **実行** を押してください。

## 使い方

1. 実行ファイルを起動します。
2. 写真のフォルダをウィンドウにドロップするか、**フォルダを選択** を押します。フォルダ内の JPEG が一覧に表示されます。
3. プリセットを選びます。quality と長辺は個別に変えることもできます。
4. **変換開始** を押します。変換したファイルは、選んだフォルダの中の `export\` に書き出されます。
5. 終わると、合計と失敗したファイルが表示されます。**出力先を開く** で `export\` を開けます。

変換する前に結果を確認したいときは、一覧でファイルを選んで **プレビュー** を押すか、ファイルをダブルクリックします。プレビューウィンドウは、別のファイルを選んだり設定を変えたりすると更新されます。

設定、表示言語、ウィンドウの大きさは次の起動時に復元されます。保存先は `%APPDATA%\web-image-optimizer-rs\` です。

## プリセット

| プリセット | 長辺 | quality |
|---|---|---|
| `low` | 1600px | 72 |
| `default` | 2400px | 82 |
| `high` | 3200px | 90 |

長辺がすでに設定値以下の画像は拡大せず、圧縮し直すだけにします。

## 残すものと削除するもの

| | |
|---|---|
| 残す | 撮影日時、カメラとレンズ、露出設定、そのほかの EXIF タグ、ICC 色プロファイル |
| 削除する | GPS 情報、XMP、IPTC、Photoshop のデータ、メーカーノート、埋め込みのサムネイル |

XMP、IPTC、メーカーノートにも位置情報が入ることがあるため、まとめて削除します。旧版は XMP を残していたので、そこに書かれた位置情報も残っていました。

縦向きで撮った写真は、どこで見ても正しい向きになるよう画素を回転します。

位置情報が残っていないことは、出力に [ExifTool](https://exiftool.org/) を実行して確認できます。何も表示されなければ問題ありません。

```
exiftool -a -G1 -gps:all -xmp:all export\*.jpg
```

## 注意

- 処理するのはフォルダ直下の `.jpg` / `.jpeg` だけです。サブフォルダは対象外です。
- `export\` に同じ名前のファイルがあると上書きします。
- 変換できないファイル（壊れたファイルや CMYK の JPEG など）があっても、ほかのファイルは処理し、失敗したファイルを理由とともに一覧に表示します。
- 変換はキャンセルできます。処理中のファイルは最後まで処理し、書きかけのファイルは残しません。

## ソースからのビルド

[Rust](https://rustup.rs/)（stable）と、`PATH` の通った [NASM](https://www.nasm.us/) が必要です。NASM がなくてもビルドはできますが、JPEG の書き出しが約 2 倍遅くなります。

```
git clone https://github.com/necotan/web-image-optimizer-rs.git
cd web-image-optimizer-rs
cargo build --release -p wio-gui
```

実行ファイルは `target\release\wio-gui.exe` にできます。仕様は [`docs/requirements.md`](docs/requirements.md) と [`docs/design.md`](docs/design.md) にあります。

## ライセンス

[MIT](LICENSE)
