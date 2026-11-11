#!/bin/sh
# Stop フック: 作業ツリーに変更があるときだけ検査を走らせ、失敗したら Claude に差し戻す
hooks_dir=$(cd "$(dirname "$0")" && pwd)
cd "$hooks_dir/../.." || exit 0
stamp_path="$hooks_dir/.verified"

# 差し戻し後の再停止でも失敗し続けると止まれなくなる
if grep -q '"stop_hook_active"[[:space:]]*:[[:space:]]*true'; then
  exit 0
fi

status=$(git status --porcelain)
printf '%s\n' "$status" | grep -Eq '(\.rs|\.toml|\.lock|\.sh)"?$' || exit 0

# 同じ変更内容で合格済みなら、質問だけのターンで待たせない
# 未追跡ファイルは git diff に出ないため中身を直接含める
stamp=$({
  printf '%s\n' "$status"
  git diff HEAD
  git ls-files --others --exclude-standard | while IFS= read -r file; do
    printf '%s\n' "$file"
    cat "$file"
  done
} | sha256sum | cut -d ' ' -f 1)
[ -f "$stamp_path" ] && [ "$(cat "$stamp_path")" = "$stamp" ] && exit 0

if ! output=$({
  cargo fmt --all --check &&
    cargo clippy --workspace --all-targets -- -D warnings &&
    cargo test --workspace &&
    sh scripts/check.sh
} 2>&1); then
  echo "fmt、clippy、test、scripts/check.sh のいずれかが失敗しました。修正してください。" >&2
  printf '%s\n' "$output" | tail -n 60 >&2
  exit 2
fi
printf '%s' "$stamp" >"$stamp_path"
