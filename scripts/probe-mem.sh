#!/usr/bin/env bash
# 内存探针矩阵：同机、同口径比较各图形后端。
#
# 为什么要矩阵而不是单点：GPU 内存的大头是驱动/设备，不是我们的缓冲。
# 只有把后端放在同一口径下并排，才知道换后端能省多少。
#
# **必须按适配器分组**：混合显卡笔记本上 wgpu 默认挑核显、D3D11 默认挑独显，
# 不分组的对比里有一半差距是"换了块显卡"。脚本对每组都显式钉住适配器。
#
# 用法：
#   scripts/probe-mem.sh                       # 默认 8 秒/格
#   PROBE_HOLD=20 scripts/probe-mem.sh         # 拉长采样窗口（更稳）
#   PROBE_ADAPTERS="AMD" scripts/probe-mem.sh  # 只测核显
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/env.sh

HOLD="${PROBE_HOLD:-8}"
ADAPTERS="${PROBE_ADAPTERS:-AMD NVIDIA}"
cargo build --release -p minidock-gpu --bin probe >/dev/null
BIN="target/x86_64-pc-windows-msvc/release/probe.exe"

echo "采样窗口 ${HOLD}s / 单位 MB / 适配器组: ${ADAPTERS}"
printf '%-10s %-8s %-6s %8s %9s %9s\n' 适配器 后端 缓冲 WS 私有 峰值
printf '%-10s %-8s %-6s %8s %9s %9s\n' ---------- -------- ------ -------- --------- ---------

row() { printf '%-10s %-8s %-6s %8s %9s %9s\n' "$1" "$2" "$3" "$4" "$5" "$6"; }

run_row() {
    local label="$1" backend="$2" pin="$3" alloc="$4"
    local out line ws priv peak
    out=$(PROBE_BACKEND="$backend" PROBE_ADAPTER="$pin" PROBE_ALLOC="$alloc" PROBE_HOLD="$HOLD" "$BIN" 2>&1 || true)
    line=$(printf '%s\n' "$out" | grep -m1 'RESULT' || true)
    if [ -z "$line" ]; then
        row "$label" "$backend" "$alloc" 失败 - -
        printf '%s\n' "$out" | tail -2
        return
    fi
    # RESULT 三个数固定顺序，用 read 拆开避免再引一层引号。
    read -r ws priv peak <<<"$(printf '%s\n' "$line" \
        | sed -n 's/.*ws_mb=\([0-9.]*\) private_mb=\([0-9.]*\) peak_ws_mb=\([0-9.]*\).*/\1 \2 \3/p')"
    row "$label" "$backend" "$alloc" "$ws" "$priv" "$peak"
}

for pin in $ADAPTERS; do
    for backend in dx12 vulkan d3d11; do
        for alloc in 0 1; do
            run_row "$pin" "$backend" "$pin" "$alloc"
        done
    done
done

# GL 的上下文由驱动默认渲染器决定（钉显卡要走 WGL_NV_gpu_affinity，
# 不是标准扩展），所以单独列一组，不参与按适配器分组。
for alloc in 0 1; do
    run_row "GL默认" gl "" "$alloc"
done

# WARP 是软件光栅器，没有适配器可钉，单独跑一行作为下界参考。
for alloc in 0 1; do
    run_row "软件光栅" d3d11-warp "" "$alloc"
done
