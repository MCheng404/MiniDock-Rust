#!/usr/bin/env bash
# 打包：构建一次，把 exe 与校验和按版本号命名后放进 dist/。
#
#   ./scripts/package.sh              # debug（版本形如 1.0.0-dev.60）
#   ./scripts/package.sh --release    # release（稳定版 1.0.0）
#   ./scripts/package.sh --pdb        # 附带调试符号（只有 debug 有）
#
# 产物是**裸文件**，不压包：
#   dist/MiniDock-<ver>-win64[-debug].exe
#   dist/MiniDock-<ver>-win64[-debug].exe.sha256
#   dist/MiniDock-<ver>-win64[-debug].pdb          （--pdb 时）
#
# 版本号来自构建脚本写出的 `target/<triple>/<profile>/MINIDOCK_VERSION`，
# 不在这里重新推导 —— 产物名必须等于 exe 资源里那个版本号。

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

mkdir -p dist
EXE="dist/$NAME.exe"
rm -f "$EXE" "$EXE.sha256" "dist/$NAME.pdb"
cp "target/$TARGET/$MODE/minidock-dock.exe" "$EXE"

if [ "$WANT_PDB" = 1 ] && [ -f "target/$TARGET/$MODE/minidock_dock.pdb" ]; then
    # 调试器按 exe 里记的路径找 pdb，改名只影响"自动发现"；
    # 需要符号时手动加载同名 pdb 即可。
    cp "target/$TARGET/$MODE/minidock_dock.pdb" "dist/$NAME.pdb"
fi

# `sha256sum` 的格式是「哈希 + 两个空格 + 文件名」，与 GitHub Release 上
# 其它项目一致：`sha256sum -c MiniDock-....exe.sha256` 直接能验。
# `--text` 不能省：MSYS 默认按二进制模式写 `*文件名`，不少校验工具不认。
(cd dist && { sha256sum --text "$NAME.exe" 2>/dev/null || sha256sum "$NAME.exe"; } > "$NAME.exe.sha256")

cat "$EXE.sha256"
