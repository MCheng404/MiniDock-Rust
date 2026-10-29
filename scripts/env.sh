#!/usr/bin/env bash
# MiniDock-rs 构建环境引导（只导出环境变量，不执行构建）。
#
# 用法：
#   source scripts/env.sh          # 在当前 shell 生效
#   source scripts/env.sh --print  # 只打印结果，便于排查
#
# 为什么必须有这个文件：
#   `.cargo/config.toml` 把默认目标锁成 x86_64-pc-windows-msvc，而
#   rustc 链接阶段要调用 MSVC 的 link.exe。本机 Visual Studio 装在
#   `D:\Program Files`（非常规路径）且**没有**出现在系统 PATH 里，
#   直接 `cargo build` 会报 "linker `link.exe` not found"。
#
# 三条注入路径（按优先级）：
#   1. 已有 cl.exe 在 PATH（CI 的 msvc-dev-cmd、vcvarsall、开发者 shell）
#      → 只补 CC/CXX/AR，不动 INCLUDE/LIB。重复注入会把上游环境覆盖坏。
#   2. 本机常见安装位置（Program Files / Program Files (x86)，任意 VS 版本）
#      → 用 vswhere 或目录扫描定位，取**最新的 MSVC 工具集**。
#   3. 已知的非常规路径（D:\Program Files\Microsoft Visual Studio\...）
#      → 兜底。
#
# 路径约定：Git Bash 会把 `/d/...` 形式的参数转换后再传给子进程，
# 导致 cl.exe 找不到头文件。本文件统一用**原生 Windows 路径**，
# 并用 MSYS_NO_PATHCONV=1 阻止自动转换。

_ENV_SH_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_ENV_SH_ROOT="$(cd "$_ENV_SH_DIR/.." && pwd)"

export MSVC_NO_PATHCONV=1
export MSYS_NO_PATHCONV=1
export CC=cl.exe
export CXX=cl.exe
export AR=lib.exe
export TARGET=x86_64-pc-windows-msvc

MSVC_KIND="unknown"
SDK_VER=""

# ---- 路径 1：环境已就绪 ---------------------------------------------------
if command -v cl.exe >/dev/null 2>&1; then
    MSVC_KIND="ambient"
    SDK_VER="ambient"
    # CAUTION: 必须把 cl.exe 目录**前置**进 PATH。Git Bash 的 /usr/bin
    # 里有一个 GNU coreutils 的 `link.exe`（硬链接工具），它排在 MSVC
    # 的 link.exe 前面时 rustc 会拿它当链接器，报
    # "/usr/bin/link: extra operand" 且 exit 101。
    # cl.exe / link.exe / lib.exe 同目录，前置一次全部归位。
    _cl_dir="$(dirname "$(command -v cl.exe)")"
    export PATH="${_cl_dir}:${PATH}"
else

    _to_win_path() {
        # /d/foo/bar -> D:\foo\bar
        printf '%s' "$1" | sed -e 's|^/\([a-zA-Z]\)/|\1:/|' -e 's|/|\\|g'
    }

    _find_msvc() {
        local candidate=""
        # 1) vswhere（任意 VS 版本、任意安装盘符）
        local vswhere="/c/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe"
        if [ -x "$vswhere" ]; then
            local inst
            inst="$("$vswhere" -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>/dev/null | tr -d '\r')"
            if [ -n "$inst" ]; then
                local win_inst
                win_inst="$(_to_win_path "$(printf '%s' "$inst" | sed -e 's|\\|/|g' -e 's|^\([a-zA-Z]\):|/\1|')")"
                # 取该实例下版本号最大的工具集
                local latest
                latest="$(ls -1 "$win_inst/VC/Tools/MSVC" 2>/dev/null | sort -V | tail -1)"
                [ -n "$latest" ] && candidate="$win_inst/VC/Tools/MSVC/$latest"
            fi
        fi
        # 2) 目录扫描（覆盖 VS 17/18 Insiders 等 vswhere 定位不到的情况）
        if [ -z "$candidate" ]; then
            local base
            for base in "/d/Program Files/Microsoft Visual Studio" "/c/Program Files/Microsoft Visual Studio" \
                        "/c/Program Files (x86)/Microsoft Visual Studio"; do
                [ -d "$base" ] || continue
                local latest
                latest="$(ls -1d "$base"/*/*/VC/Tools/MSVC/*/ 2>/dev/null | sort -V | tail -1)"
                [ -n "$latest" ] && { candidate="$(printf '%s' "$latest" | sed -e 's|/$||')"; break; }
            done
        fi
        printf '%s' "$candidate"
    }

    MSVC_ROOT_POSIX="$(_find_msvc)"
    if [ -z "$MSVC_ROOT_POSIX" ] || [ ! -f "$MSVC_ROOT_POSIX/bin/Hostx64/x64/cl.exe" ]; then
        echo "ERROR: 找不到 MSVC 工具集。请安装 Visual Studio 的「使用 C++ 的桌面开发」工作负载，" >&2
        echo "       或在已初始化 vcvars 的 shell 里重新 source 本文件。" >&2
        return 1 2>/dev/null || exit 1
    fi

    SDK_ROOT_POSIX="/c/Program Files (x86)/Windows Kits/10"
    if [ ! -d "$SDK_ROOT_POSIX/Include" ]; then
        echo "ERROR: 未找到 Windows SDK（$SDK_ROOT_POSIX）。" >&2
        return 1 2>/dev/null || exit 1
    fi
    # 自动挑最新的 SDK，避免写死小版本号后换机失效。
    SDK_VER="$(ls -1 "$SDK_ROOT_POSIX/Include" | sort -V | tail -1)"

    MSVC_ROOT="$(_to_win_path "$MSVC_ROOT_POSIX")"
    SDK_ROOT="$(_to_win_path "$SDK_ROOT_POSIX")"

    export INCLUDE="${MSVC_ROOT}\\include;${SDK_ROOT}\\Include\\${SDK_VER}\\ucrt;${SDK_ROOT}\\Include\\${SDK_VER}\\um;${SDK_ROOT}\\Include\\${SDK_VER}\\shared;${SDK_ROOT}\\Include\\${SDK_VER}\\winrt"
    export LIB="${MSVC_ROOT}\\lib\\x64;${SDK_ROOT}\\Lib\\${SDK_VER}\\ucrt\\x64;${SDK_ROOT}\\Lib\\${SDK_VER}\\um\\x64"
    export PATH="${MSVC_ROOT_POSIX}/bin/Hostx64/x64:${PATH}"

    MSVC_KIND="injected"
fi

if [ "${1:-}" = "--print" ]; then
    echo "ROOT=$_ENV_SH_ROOT"
    echo "TARGET=$TARGET"
    echo "MSVC_KIND=$MSVC_KIND"
    echo "MSVC_ROOT=${MSVC_ROOT:-<ambient>}"
    echo "SDK_VER=$SDK_VER"
    echo "CC=$CC"
    echo "INCLUDE=${INCLUDE:-<ambient>}"
    echo "LIB=${LIB:-<ambient>}"
fi
