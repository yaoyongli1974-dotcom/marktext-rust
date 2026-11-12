#!/usr/bin/env bash
# 下载并校验 v0.1.1 的发布产物。
#
# 校验两件事：
#   1. SHA256 与 Release 上的 SHA256SUMS.txt 一致；
#   2. 包里确实有该有的文件（三个平台的产物、README、LICENSE、NOTICE）。
set -euo pipefail

TAG="${1:-v0.1.1}"
REPO="yaoyongli1974-dotcom/marktext-rust"
WORK="target/release-verify-${TAG}"

# 用 PATH 里的 gh；没有 gh 时给出可执行的安装提示，而不是在别的机器上
# 因为一个写死的 Windows 路径而失败。
GH="${GH:-gh}"
if ! command -v "$GH" >/dev/null 2>&1; then
  echo "需要 GitHub CLI（gh）才能下载产物：https://cli.github.com/" >&2
  exit 1
fi

rm -rf "$WORK"
mkdir -p "$WORK"
cd "$WORK"

echo "=== 下载 $TAG 的产物 ==="
"$GH" release download "$TAG" --repo "$REPO" --clobber
ls -la

echo
echo "=== 校验 SHA256 ==="
sha256sum -c SHA256SUMS.txt

echo
echo "=== 解包检查 ==="
for archive in *.tar.gz *.zip; do
  [ -e "$archive" ] || continue
  echo "--- $archive ---"
  case "$archive" in
    # 发布时的 tar 用 `tar -C stage ... .` 生成，条目带 `./` 前缀；
    # 去掉它，后面才能按文件名比较。
    *.tar.gz) listing=$(tar -tzf "$archive" | sed 's#^\./##') ;;
    *.zip)    listing=$(unzip -Z1 "$archive") ;;
  esac
  printf '%s\n' "$listing"

  for want in README.md CHANGELOG.md LICENSE NOTICE.md; do
    if ! grep -qx "$want" <<<"$listing"; then
      echo "缺少 $want：$archive" >&2
      exit 1
    fi
  done

  case "$archive" in
    *linux-gnu*)    want_bins=(marktext marktext-app) ;;
    *linux-musl*)   want_bins=(marktext) ;;
    *windows-msvc*) want_bins=(marktext.exe marktext-app.exe) ;;
    *)              want_bins=() ;;
  esac
  for bin in "${want_bins[@]}"; do
    if ! grep -qx "$bin" <<<"$listing"; then
      echo "缺少可执行文件 $bin：$archive" >&2
      exit 1
    fi
  done
done

echo
echo "=== 全部产物校验通过 ==="
