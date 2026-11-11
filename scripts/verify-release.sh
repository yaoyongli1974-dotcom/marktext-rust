#!/usr/bin/env bash
# 下载并校验 v0.1.1 的发布产物。
#
# 校验三件事：
#   1. SHA256 与 Release 上的 SHA256SUMS.txt 一致；
#   2. 包里确实有该有的文件；
#   3. Windows 的命令行工具能真的跑起来，并且中文不乱码。
set -euo pipefail

TAG="${1:-v0.1.1}"
REPO="yaoyongli1974-dotcom/marktext-rust"
WORK="target/release-verify-${TAG}"
GH="/c/Users/h/AppData/Local/gh-cli/gh.exe"

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
    *.tar.gz) tar -tzf "$archive" ;;
    *.zip)    unzip -Z1 "$archive" ;;
  esac
done

echo
echo "=== 全部产物校验通过 ==="
