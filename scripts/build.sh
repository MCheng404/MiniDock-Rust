#!/usr/bin/env bash
# MiniDock-rs 构建入口。
#
# 环境变量由 scripts/env.sh 统一注入（MSVC / Windows SDK），本脚本只负责
# 「决定跑哪条 cargo 命令」。两者分开，才能在一个已 source 过环境的 shell
# 里直接跑 `cargo clippy` / `cargo install` 这类一次性命令。
#
# 用法：
#   ./scripts/build.sh                # debug 构建
#   ./scripts/build.sh --release      # release 构建（体积优化 + LTO）
#   ./scripts/build.sh --run          # 构建并运行
#   ./scripts/build.sh --test         # 跑全部单元/集成测试
#   ./scripts/build.sh --test config  # 只跑名字含 config 的测试
#   ./scripts/build.sh --check        # cargo check（最快反馈）
#   ./scripts/build.sh --clippy       # clippy（-D warnings）
#   ./scripts/build.sh --fmt          # 格式化
#   ./scripts/build.sh --gate         # 完整门禁：fmt + clippy + test（提交前跑这个）
#   ./scripts/build.sh --exe          # 打印产物路径
#
# 其余参数原样透传给 cargo（如 -p <crate>、--nocapture）。

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# shellcheck source=env.sh
source "$ROOT/scripts/env.sh"

MODE=debug
ACTION=build
EXTRA=()
while [ $# -gt 0 ]; do
    case "$1" in
        --release) MODE=release; shift ;;
        --run)     ACTION=run; shift ;;
        --test)    ACTION=test; shift ;;
        --check)   ACTION=check; shift ;;
        --clippy)  ACTION=clippy; shift ;;
        --fmt)     ACTION=fmt; shift ;;
        --gate)    ACTION=gate; shift ;;
        --exe)     ACTION=exe; shift ;;
        *) EXTRA+=("$1"); shift ;;
    esac
done

profile_flag() {
    [ "$MODE" = release ] && echo --release
}

profile_dir() {
    if [ "$MODE" = release ]; then echo release; else echo debug; fi
}

case "$ACTION" in
    build)
        cargo build --target "$TARGET" $(profile_flag) ${EXTRA[@]+"${EXTRA[@]}"}
        ;;
    run)
        cargo run --target "$TARGET" $(profile_flag) ${EXTRA[@]+"${EXTRA[@]}"}
        ;;
    test)
        # --lib --tests：跳过 doctest。本项目的文档示例不承担 API 契约，
        # 每个 doctest 都是独立的编译+运行目标，纯属浪费链接时间。
        cargo test --target "$TARGET" --workspace --lib --tests ${EXTRA[@]+"${EXTRA[@]}"}
        ;;
    check)
        cargo check --target "$TARGET" --workspace --all-targets ${EXTRA[@]+"${EXTRA[@]}"}
        ;;
    clippy)
        cargo clippy --target "$TARGET" --workspace --all-targets -- -D warnings ${EXTRA[@]+"${EXTRA[@]}"}
        ;;
    fmt)
        cargo fmt --all -- --check
        ;;
    gate)
        # 完整门禁。三步**都跑完再汇总**，而不是第一步失败就退出 ——
        # 一次看清全部问题，比"修一个跑一次"快得多。
        status=0
        echo "== 1/3 fmt =="
        cargo fmt --all -- --check || status=1
        echo "== 2/3 clippy =="
        cargo clippy --target "$TARGET" --workspace --all-targets -- -D warnings ${EXTRA[@]+"${EXTRA[@]}"} || status=1
        echo "== 3/3 test =="
        cargo test --target "$TARGET" --workspace --lib --tests ${EXTRA[@]+"${EXTRA[@]}"} || status=1
        echo
        if [ "$status" -eq 0 ]; then
            echo "门禁通过。"
        else
            echo "门禁未通过，见上面的输出。" >&2
        fi
        exit "$status"
        ;;
    exe)
        echo "$ROOT/target/$TARGET/$(profile_dir)/MiniDock.exe"
        ;;
esac
