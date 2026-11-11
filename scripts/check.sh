#!/bin/sh
# clippy では調べられない CLAUDE.md のルールを検査する。pre-commit と CI から呼ぶ
cd "$(dirname "$0")/.." || exit 1
status=0
fail() {
  echo "check: $1" >&2
  status=1
}

# なくてもビルドは通り、mozjpeg が SIMD なしになって遅くなるだけなので気づけない
command -v nasm >/dev/null 2>&1 || fail "nasm が PATH にありません"

ui=$(cargo tree -p wio-core -e normal --prefix none | grep -E '^(egui|eframe|egui_extras|rfd|winit) ' | sort -u)
[ -z "$ui" ] || fail "wio-core が UI のクレートに依存しています:
$ui"

pinned=$(grep -nE '"=[0-9]' Cargo.toml crates/*/Cargo.toml)
[ -z "$pinned" ] || fail "クレートのバージョンを = で固定しないでください (cargo add で追加する):
$pinned"

node=$(git ls-files | grep -E '(^|/)(package(-lock)?\.json|node_modules/)|\.(js|mjs|cjs|ts)$')
[ -z "$node" ] || fail "Node.js のファイルを持ち込まないでください (旧版は別のリポジトリで実行する):
$node"

exit $status
