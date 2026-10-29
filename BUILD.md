# 构建

## 一、实测环境（2026-10-27 核对）

| 组件 | 版本 | 位置 |
|---|---|---|
| rustc / cargo | 1.99.1 | `C:\Users\Cookies\.cargo\bin` |
| 工具链 | `stable`（默认 gnu，另装 msvc） | `rustup toolchain list` |
| MSVC 工具集 | 14.52.36629（VS 18 Insiders） | `D:\Program Files\Microsoft Visual Studio\18\Insiders\VC\Tools\MSVC\` |
| Windows SDK | 10.0.28000.0（另有 26100.0） | `C:\Program Files (x86)\Windows Kits\10` |
| MinGW-w64 | 16.2.0（仅用于编译 C 版对照） | WinGet `BrechtSanders.WinLibs.POSIX.UCRT` |

目标三元组固定 `x86_64-pc-windows-msvc`，`rustflags = ["-C","target-feature=+crt-static"]`。

**为什么是 MSVC 而不是 GNU**：系统 DLL（GDI+/DWM/shell32）都是 MSVC ABI 的第一等公民；
`+crt-static` 把 CRT 静态链进去，产物不依赖 `VCRUNTIME140.dll`；崩溃转储与 WinDbg 配合最好。
代价是本机 VS 装在 `D:\Program Files`（非常规路径），系统 PATH 里没有 `cl.exe`。

## 二、构建命令

```bash
source scripts/env.sh              # 必须。注入 INCLUDE/LIB/PATH + CC/CXX/AR
./scripts/build.sh                 # debug
./scripts/build.sh --release       # release（opt-level=z + fat LTO + strip）
./scripts/build.sh --test          # 全部测试（跳过 doctest）
./scripts/build.sh --clippy        # -D warnings
./scripts/build.sh --fmt           # 仅检查格式
./scripts/build.sh --exe           # 打印产物路径
```

PowerShell 等价入口：`powershell -ExecutionPolicy Bypass -File scripts\build.ps1 -Mode test`。

`env.sh` 的三条注入路径（按优先级）：
1. PATH 里已有 `cl.exe`（CI 的 `msvc-dev-cmd`、已初始化的 vcvars shell）→ 只补 `CC/CXX/AR`，不动 `INCLUDE/LIB`；
2. `vswhere` 定位任意 VS 实例 → 取该实例下版本号最大的工具集；
3. 目录扫描 `Program Files / D:\Program Files` 兜底。

SDK 版本**自动取最新的**，不写死小版本号——写死是换机后最先失效的东西。

## 三、依赖版本

| crate | 版本 | 说明 |
|---|---|---|
| `egui` | 0.37 | 只开 `default_fonts`；中文字体运行时装载 |
| `egui-wgpu` | 0.37 | 提供 `Renderer` 与 `wgpu` **重导出** |
| `wgpu` | 30.0.1 | **不单独声明版本**，用 `egui_wgpu::wgpu`，避免与 egui 侧漂移 |
| `egui_kittest` | 0.37 | 仅 dev-dependency，不进产物 |
| `windows` | 0.62 | 官方绑定，feature 并集写在 workspace |
| `rayon` / `crossbeam-channel` / `arc-swap` | 1.12 / 0.5 / 1.9 | 并行与跨线程数据 |
| `raw-window-handle` | 0.6 | 给 wgpu 建 surface 用（必须与 wgpu 期望的版本一致） |

**没有 eframe、没有 winit**。理由见 [`DECISIONS.md`](DECISIONS.md) ADR-002。

## 四、产物

| 目标 | 路径 | 说明 |
|---|---|---|
| `minidock-spike` | `target/x86_64-pc-windows-msvc/debug/minidock-spike.exe` | M0 尖刺，Esc 退出 |
| `MiniDock` | 同目录 `MiniDock.exe` | 遗留 GDI+ 版，M2 后替换 |

首次 `cargo build` 要编译 wgpu（naga、wgpu-hal、DX12 后端），**冷启动 3–6 分钟**属正常；
之后增量在 10–30 秒。release 首次更久（fat LTO）。

## 五、内存基线（2026-10-27 · AMD Radeon 610M + NVIDIA RTX 5070 Laptop · 2560×1600@150%）

体积：`minidock-spike.exe` release **7.2 MB**（`opt-level=z` + fat LTO + strip），
debug 43 MB（符号占绝大部分）。

内存：`probe.exe` 只建设备、不建窗口、不跑 egui，由进程内 `mem.rs` 自采样
（`PROCESS_MEMORY_COUNTERS_EX`，与界面上的 RSS 同一口径）。
`PROBE_ALLOC=1` 时再分配与真实应用等量的三块：1320×1080 渲染目标（8.4 MB）
+ 同尺寸回读（5.4 MB）+ 同尺寸 DIB（5.4 MB）。

| 适配器 | 后端 | 设备 WS | 设备+缓冲 WS | 设备 私有 | 设备+缓冲 私有 |
|---|---|---|---|---|---|
| AMD 610M（核显） | dx12 / wgpu | 164.2 | 227.7 | 358.8 | 423.5 |
| AMD 610M | vulkan / wgpu | 103.2 | 169.1 | 279.7 | 349.8 |
| AMD 610M | **d3d11 原生** | **33.7** | 33.9 | 24.8 | 44.1 |
| NVIDIA 5070（独显） | dx12 / wgpu | 86.8 | 150.8 | 328.6 | 396.1 |
| NVIDIA 5070 | vulkan / wgpu | 79.5 | 144.7 | 258.0 | 329.0 |
| NVIDIA 5070 | **d3d11 原生** | **39.1** | 40.5 | 54.8 | 72.2 |
| NVIDIA 5070 | OpenGL（WGL） | 51.9 | 51.9 | 47.3 | 58.1 |
| 软件光栅 | d3d11 WARP | 13.8 | 14.0 | 3.1 | 19.5 |

**结论一：wgpu 比原生 D3D11 贵 2.5–5 倍。** 同一块 AMD 核显 164 MB vs 34 MB，
同一块 NVIDIA 独显 87–102 MB vs 39 MB。差价在 wgpu-hal 与 D3D12 运行时，
不在业务代码 —— 这组数字是在一行渲染都没跑的空进程里测的。

**结论二：19 MB 的缓冲在 wgpu 下被放大成 +64 MB 工作集**（DX12 / Vulkan 都是）。
D3D11 下同样三块只让私有提交 +19 MB、工作集几乎不动（没写过的页不驻留）。

**结论三：OpenGL 是"保住 egui 又省内存"的中间档。** 52 MB，约 wgpu-DX12 的 1/3，
原生 D3D11 的 1.3 倍。据此曾提议 ADR-011（换 `egui_glow`），**已否决**。


### 调优：两个参数把 DX12 打下来（2026-10-27，AMD 610M）

不改框架、不换后端，只调 `DeviceDescriptor` 的两个字段：

| 配置 | 工作集 | 私有提交 |
|---|---|---|
| 默认（`Limits::downlevel_defaults` + `MemoryHints::Performance`） | 164.1 | 356.7 |
| `max_non_sampler_bindings: 4096` | 133 | 266 |
| 再加 `MemoryHints::MemoryUsage` | **102** | 74–110 |
| 实际 spike（release，设置窗 1320×1080） | **271 → 111** | 429 → 113 |

两条依据都来自 wgpu 自己的文档：

- **`max_non_sampler_bindings` 默认 1_000_000**。“This limit only affects the
  d3d12 backend… at the cost of a large up-front allocation at device creation”，
  核显共享系统内存，所以这笔预分配直接算在进程上。压到 4096 省 31 MB，
  再往下（256/64）没有额外收益。
- **`memory_hints` 默认 `Performance`**，会让分配器按性能多留块。
  Dock 每帧只有一两个 bind group，`MemoryUsage` 又省 31 MB，且不改变渲染结果
  （像素核对：面板 `(29,30,34)`、圆角外透出壁纸，与调优前一致）。

**不要用 `downlevel_webgl2_defaults()` 省内存**：它的
`max_texture_dimension_2d` 只有 2048，而 2560 宽的 Dock 在 150% DPI 下
就要 3840 px 的离屏纹理。实测三档限制的内存完全一致，没有收益只有风险。

**也顺带纠正一个隐患**：原先用 `Limits::downlevel_defaults()` 同样把纹理上限
卡在 2048，只是设置窗只有 1320 宽才没暴露。现在改成 `..adapter.limits()`，
只收紧描述符预算。

**同一个探针还测了另外三条路**（都不如上面这条划算）：

| 方案 | 工作集 | 说明 |
|---|---|---|
| wgpu 自带 GL 后端（`Backends::GL`，wgpu-hal 的 WGL 实现） | 117 | 保留 egui-wgpu 只换后端，但比调优后的 DX12 还高，且落在独显上 |
| 原生 OpenGL 上下文 | 52 | 要换 `egui_glow`（ADR-011，已否决） |
| 原生 D3D11 | 34 | 要自研 UI，失去 kittest |

外部佐证：wgpu 上游 issue 与 Slint #13470 记录的同量级现象（D3D12 路径比
OpenGL 路径多 ~110 MB），以及一个独立项目把 wgpu-DX12 的空设备地板测成
~300 MB 并最终放弃压缩 —— 结论一致：**这笔钱在 D3D12 栈里，不在业务代码**。

### CPU 基线

| 场景 | 结果 |
|---|---|
| 静止（单实例，光标不在窗口上） | **0.000 秒 / 30 秒** —— 线程阻塞在 `MsgWaitForMultipleObjectsEx(INFINITE)` |
| 悬停 / 动画 | 按 `repaint_delay` 排帧，`ZERO` 时节流到 60 fps |
| 单帧 CPU（1320×1080，含 GPU 回读与呈现） | release 下 1–2 ms（见性能页） |

查"为什么还在重绘"：`MINIDOCK_TRACE=1` 运行，每秒一行，
含累计帧数、`hover` 位置与 `repaint_causes()`。

### 方法学（不写清楚就会得出错误结论）

- **必须按适配器分组。** 本机是 NVIDIA + AMD 双显卡：wgpu 的 `LowPower`
  默认选核显，`D3D11CreateDevice(NULL)` 默认选独显 —— 不钉适配器的对比里
  "87 vs 164"有一半是换了块显卡。用 `PROBE_ADAPTER=AMD|NVIDIA` 钉住，
  `PROBE_LIST=1` 先看清单。
- **工作集 ≠ 私有提交。** D3D11 下缓冲几乎不体现在工作集里（惰性提交），
  看私有列才是"承诺了多少"；wgpu 两列都高，因为它同时抬高驻留与提交。
- **驱动有 ±15 MB 抖动**（NVIDIA dx12 稳态 87 / 峰值 102），比较时看峰值列。
- **混合显卡上 GL 上下文落在独显**（本机是 NVIDIA），所以 GL 行不与 AMD 组同卡。
- 早期文档把本机写成 "Intel Iris Xe"，那是把 wgpu 打印的 `device=5710`
  （PCI id）当成了名字：`5710 = 0x164E` 是 AMD Radeon 610M。

复现：`scripts/probe-mem.sh`（`PROBE_HOLD=20` 更稳）。单点：
`PROBE_BACKEND=d3d11 PROBE_ADAPTER=NVIDIA PROBE_ALLOC=1`。探针源码
`crates/minidock-gpu/src/bin/probe.rs`，采样逻辑 `src/mem.rs`。

## 六、排错

| 症状 | 原因 | 处理 |
|---|---|---|
| `linker 'link.exe' not found` | 没 source `env.sh` | `source scripts/env.sh` |
| egui 画面偏左上、只占表面 1/ppp | `RawInput` 少了 `viewport_info.native_pixels_per_point`（egui 0.37 改用它传原生缩放） |
| `/usr/bin/link: extra operand` | Git Bash 的 GNU `link` 抢在 MSVC `link.exe` 前面 | `env.sh` 已把 cl.exe 目录**前置**进 PATH；若手工配置需照做 |
| `找不到 MSVC 工具集` | VS 没装「使用 C++ 的桌面开发」 | 装工作负载，或用已初始化的 vcvars shell |
| 测试 panic `No glyph for "常"` | 没装中文字体，egui 0.37 缺字形直接 panic | 确认 `C:\Windows\Fonts\msyh.ttc` 存在；测试用 `with_context_setup` 预装 |
| 窗口透明但画面全黑 | 交换链 alpha 模式退化成 `Opaque` | 看启动日志；考虑 DirectComposition 方案 |
| `cargo fetch` 卡住 | 首次拉取 wgpu/egui 依赖树 | 需要网络；缓存后离线可构建 |
| RDP/虚拟机里没画面 | DXGI 复制或 DX12 不可用 | M3 会加 CPU 模糊 fallback；M0 仅作提示 |

## 六、测试分层

| 层 | 命令 | 依赖 |
|---|---|---|
| 逻辑层（配置/INI/布局/条目） | `cargo test -p minidock-core` | 无 |
| 设置界面（kittest 无头） | `cargo test -p minidock-settings` | 需要系统中文字体 |
| 平台层（显示器/注册表） | `cargo test -p minidock-platform` | 需要桌面会话 |
| GPU 尖刺 | 手动运行 | 需要 D3D12 适配器 |

`core` 的测试**不依赖 Windows**，可以放到任意 CI runner 上跑；这是把纯逻辑
单独拆 crate 的主要动机。
