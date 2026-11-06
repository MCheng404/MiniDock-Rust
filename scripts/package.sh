#!/usr/bin/env bash
# 打包：构建一次，把产物按版本号命名后放进 dist/。
#
#   ./scripts/package.sh              # debug（版本形如 0.9.25-dev.58）
#   ./scripts/package.sh --release    # release（稳定版 0.9.25）
#   ./scripts/package.sh --pdb        # 连调试符号一起打包（默认不带）
#
# 产物：
#   dist/MiniDock-<ver>-win64/           minidock-dock.exe + LICENSE + README.md
#   dist/MiniDock-<ver>-win64-debug/     同上；带 --pdb 时另加 minidock_dock.pdb
#   同名 .zip（机器上有 zip 命令时）
#
# 版本号来自构建脚本写出的 `target/<triple>/<profile>/MINIDOCK_VERSION`，
# 不在这里重新推导 —— 产物名必须等于 exe 资源里那个版本号。
#
# pdb 默认不进包：debug 的 PDB 有 200 MB 量级，每个提交都传一遍不划算。

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

MODE=debug
WANT_PDB=0
for arg in "$@"; do
    case "$arg" in
        --release) MODE=release ;;
        --pdb) WANT_PDB=1 ;;
        *) echo "用法: $0 [--release] [--pdb]" >&2; exit 2 ;;
    esac
done

FLAGS=()
[ "$MODE" = release ] && FLAGS=(--release)

# shellcheck source=env.sh
source "$ROOT/scripts/env.sh"

"$ROOT/scripts/build.sh" ${FLAGS[@]+"${FLAGS[@]}"} -p minidock-gpu --bin minidock-dock

VERSION_FILE="target/$TARGET/$MODE/MINIDOCK_VERSION"
if [ ! -f "$VERSION_FILE" ]; then
    echo "缺少 $VERSION_FILE —— 构建脚本没有写出（先跑一次 ./scripts/build.sh）" >&2
    exit 1
fi
VERSION="$(tr -d '[:space:]' < "$VERSION_FILE")"

SUFFIX=""
[ "$MODE" = release ] || SUFFIX="-debug"
NAME="MiniDock-$VERSION-win64$SUFFIX"
OUT="dist/$NAME"

rm -rf "$OUT"
mkdir -p "$OUT"
cp "target/$TARGET/$MODE/minidock-dock.exe" "$OUT/"
if [ "$WANT_PDB" = 1 ] && [ -f "target/$TARGET/$MODE/minidock_dock.pdb" ]; then
    # 名字必须是 `minidock_dock.pdb`：调试器按 exe 里记的路径去找，
    # cargo 把连字符换成了下划线。
    cp "target/$TARGET/$MODE/minidock_dock.pdb" "$OUT/"
fi
cp LICENSE README.md "$OUT/"

if command -v zip > /dev/null 2>&1; then
    (cd dist && zip -qr "$NAME.zip" "$NAME")
    echo "dist/$NAME.zip"
else
    echo "（没找到 zip 命令，只产出目录）" >&2
    echo "$OUT"
fi
