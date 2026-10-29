# 变更记录

格式：里程碑为单位，每条写"做了什么 / 为什么 / 已知问题"。
版本号暂不与里程碑绑定（0.1.0 覆盖 M0）。

---

## 0.1.0 — M0 尖刺（2026-10-27）

### 新增工作区

7 个 crate，依赖方向单向：`gpu → settings → core`、`gpu → platform`。

- `minidock-core`：配置模型（76 键 + 夹紧）、INI 编解码（UTF-16LE/BE/UTF-8）、
  条目与分组序列化、布局数学、DPI 换算。**无 Win32 依赖**，48 条单测。
- `minidock-platform`：路径（便携模式 + 迁移）、显示器枚举、系统深浅色、
  开机自启动、单实例互斥。含真机测试（显示器枚举）。
- `minidock-settings`：egui 9 页设置界面，纯 UI 无副作用，产出 `Command`。
  5 条 kittest 无头测试（点击开关、页签切换、派生键映射、命令产出、全页渲染）。
- `minidock-gpu`：wgpu 设备/表面、egui 提交、Win32 输入映射、
  `WM_NCHITTEST` 命中、消息循环。含 `minidock-spike` 可执行。
- `minidock-gfx` / `minidock-ui` / `minidock`：GDI+ 遗留实现，作行为参照（ADR-010）。

### 关键实现决策

- 依赖全部取当前最新：egui 0.37、egui-wgpu 0.37、wgpu 30、windows 0.62。
- **不用 eframe/winit**：自持 Win32 消息循环（ADR-002）。
- 中文字体运行时装载系统 `msyh.ttc`（ADR-004）。
- 逐像素命中用 `WM_NCHITTEST`，不轮询（ADR-005）。

### 实测发现（改变后续计划）

1. **wgpu 的 DX12 表面只报 `Opaque`**，Vulkan 后端同样无 alpha 模式。
   窗口无法直接透明 → 改"离屏渲染 + `UpdateLayeredWindow`"（ADR-008），
   M1 实现；M3 再评估 DirectComposition。
2. **DPI 感知必须自己声明**。未声明时 Windows 拉伸窗口位图，
   表面尺寸与客户区错位（实测 1320×1080 vs 880×720），画面被裁切。
   逻辑→物理尺寸换算放在建窗时完成。
3. **egui 0.37 缺字形直接 panic**（`MissingGlyphPolicy::Panic`），
   不是画豆腐块。字体必须在第一帧之前装好。
4. **`TexturesDelta` 必须在 Drop 前 `clear()`**，否则断言失败 panic。
5. **不能从 `WndProc` 里直接渲染**：会与 `ShowWindow` 等调用重入，
   且 panic 无法跨 FFI 展开（实测 `panic_cannot_unwind`）。
   改为"登记 `needs_frame`，循环里统一出帧"。

### M0 打磨（同日）

- **透明呈现落地**：`offscreen.rs`（纹理 → 回读缓冲）+ `present.rs`
  （BGRA 预乘 DIB → `UpdateLayeredWindow`）。实测不透明面板整窗逐像素
  等于 `(24,25,30)`，圆角外 alpha=0 处透出桌面。
- **颜色修正**：离屏目标必须用 `Rgba8UnormSrgb`。第一版用 `Rgba8Unorm`，
  egui 在非 sRGB 目标上自己做了一次 gamma 编码，配色整体提亮。
- **命中改由分层窗口按 alpha 判定**，删掉手写 `WM_NCHITTEST`。
- **输入补齐**：`WM_MOUSELEAVE` + `PointerGone`、`WM_SETCURSOR` 与
  egui 光标映射（手型/文本/箭头）。
- **尖刺变成测量工具**：性能页实时显示帧耗时 / fps / RSS / 呈现路径；
  新增 `probe.exe` 只建设备不建窗口，用来分离驱动基线与应用开销。
- **质量门禁**：`clippy -D warnings` 与 `cargo fmt --check` 全绿；61 条测试通过。

### 探针补全（同日）

- `probe.exe` 重写为四后端矩阵：`dx12` / `vulkan`（wgpu）、`d3d11` / `d3d11-warp`
  （原生）、`gl`（WGL 兼容上下文），并支持 `PROBE_ADAPTER` **钉住同一块显卡**、
  `PROBE_LIST` 枚举适配器、`PROBE_ALLOC` 分配与真实应用等量的缓冲。
- 新增 `mem.rs`：进程内自采样 WS / 峰值 / 私有提交，界面与探针同一口径。
- 新增 `scripts/probe-mem.sh`：一条命令跑完整矩阵。
- 关键结论：**wgpu 比同卡原生 D3D11 贵 2.5–5 倍**（164 vs 34 MB，核显）；
  OpenGL 52 MB。据此提出 ADR-011（`egui_glow` 替代 `egui-wgpu`），
  同日**否决**：保留 wgpu 单栈，数据留档。
- 更正：早期文档把本机写成 "Intel Iris Xe"，实为 AMD Radeon 610M
  （wgpu 打印的 `device=5710` 是 PCI id `0x164E`，不是名字）。

### M1 第一批：Dock 本体（同日）

- 新增 `minidock-gpu/src/dock.rs`：Dock 布局状态机 + egui 绘制。
  几何逐条照抄 C 版 v1.4.0 的 `Paint()`：栏体、时钟卡片、分割线、图标占位、
  运行指示点、悬停气泡、悬停放大。
- 新增可执行文件 `minidock-dock`：真实 Dock 形态的窗口（不是设置窗），
  复用 M0 的离屏 + 分层窗口管线。Esc 退出。
- **修正命中语义**（ADR-018）：从"最近"改为 C 版的"从左到右第一个"；
  新增 `map_actual_to_base`，放大权重改用基准位置。
- **修正 egui 缩放**（ADR-019）：egui 0.37 用
  `ViewportInfo::native_pixels_per_point` 告知原生缩放。
  之前 M0 的 `Input::take` 与新写的 Dock bin 都漏了它，
  后果是 egui 按 1.0 缩放 —— 画面只占表面的 1/ppp（偏左上、尺寸偏小），
  设置界面的文字也被放大发虚。
- 实测单帧耗时：egui 0.3 ms / 回读 1.3 ms / 呈现 0.2 ms（release，1121×186）。

### 上游 v1.4.0 同步（同日）

上游把 `[Settings]` 从 37 键扩到 76 键，并改了布局公式。本轮同步：

- **键表表驱动**（ADR-017）：41 个"裸 int"键进 `NUMERIC_KEYS` 表，
  `Key::Numeric(i)` 指向表行。加一个上游键的代价从"改 8 处"降到"加一行"。
- 76 键全部建模，顺序与上游 `g_tab[]` 一致（写回 ini 的行序必须相同）。
  夹紧规则比上游更严 —— 上游对这些 int 根本不夹，越界直接进绘制代码。
- `PlateDark`/`PlateLight` → `PlateMode`/`PlateTheme`/`PlateOpacity`。
  上游无兼容处理，我们也不做：旧键留在 ini 里但不再生效。
- **布局公式更新**：分隔线宽度改由 `SepGap` 决定；倒影高度改由
  `ReflSize`/`ReflGap` 决定；时钟卡片宽度改由
  `ClockShow`/`ClockSec`/`DateFmt`/`ClockWeek` 算出（横向 74/92/110/124/146/166）。
- 设置界面补上 20 个与渲染相关的新键（外观 / 图标 / 交互 / 附件 / 性能页）。
- 新增 `cargo run -p minidock-core --example config_table`：
  由代码生成 `CONFIG.md` 的键表，杜绝文档与实现漂移。
- 新增 4 条测试：键名逐个对照上游、顺序与总数、具名常量与表行对齐、数值键默认值。

### M0 打磨：调度、单实例、像素搬运（同日）

- **重绘调度改为按 egui 的 `repaint_delay`**（ADR-014）。原实现是
  `has_requested_repaint() → render → sleep(8ms)` 的空转；现在
  `ZERO` 节流到 16 ms、有限值按点唤醒、`MAX` 直接阻塞等消息。
  实测空闲 30 秒 CPU **0.000 秒**。
- **接入单实例**（ADR-015）。之前没接，双击两次会叠两个分层窗口，
  外部看着像"空闲占 8% CPU"，实际是两份在跑。
- **回读位图按 u32 交换 R/B**（ADR-016），实测 1.16 → 0.80 ms/帧（1.45x）。
- 新增 `MINIDOCK_TRACE=1`：每秒打印帧数、hover 位置、egui 的重绘原因，
  专门用来查"空闲还在重绘"这类问题。

### 内存调优（同日）

- `max_non_sampler_bindings`: 1_000_000 → **4096**。wgpu 文档点名该项只在
  DX12 生效，且是**设备创建时**按此预分配描述符堆。省 31 MB。
- `memory_hints`: `Performance` → **`MemoryUsage`**。再省 31 MB，画面不变。
- 顺带修隐患：原先的 `Limits::downlevel_defaults()` 把
  `max_texture_dimension_2d` 卡在 2048，超宽屏（150% DPI 下 3840 px）
  会建不出离屏纹理。改为 `..adapter.limits()`，只收紧描述符预算。
- 实测：空设备 164 → 102 MB；spike（release，设置窗 1320×1080）
  **271 → 111 MB**（工作集 −59%，私有提交 429 → 113 MB）。
- `probe` 增加 `PROBE_NON_SAMPLER` / `PROBE_HINTS` / `PROBE_LIMITS` 三个开关，
  以及 `wgl` 后端（wgpu 自带的 GL 路径），用来复现这些数字。

### 显卡适配器选择（同日）

- `Config.adapter`：`[Graphics] Adapter`，空 = 自动。存**名字**不存索引
  （索引随驱动/插拔变化），写入前过 `sanitize_adapter`（去空白、剔控制字符、
  限长 128）。5 条单测覆盖默认、往返、清洗、自动往返。
- `minidock-platform::gpu`：DXGI 枚举适配器（名字 / 厂商 / 显存 / 类型），
  按 `(vendor, device_id, name)` 去重并按独显→核显→其它→软件排序。3 条单测。
- `Gpu::new(wanted)`：空 = 自动（`PowerPreference::LowPower`）；非空按名字匹配
  （精确忽略大小写 → 互相包含），**匹配不到回退自动**并把候选写进日志与界面提示。
- 设置页「性能 → 图形」新增显卡下拉，显示"当前实际使用"的真值；
  回退时用警示色说明。3 条 kittest 用例覆盖切换、切回自动、回退提示。
- 实测三条路径：自动 → AMD 610M（核显）；指定 → NVIDIA RTX 5070（独显）；
  指定不存在 → 回退自动并列出候选。

### 已知问题

- Dock 本体未迁移（M1）。
- 设置窗口目前不透明（设计如此，ADR-009）。
- 输入未覆盖 IME、拖拽、`WM_MOUSELEAVE`。
- `minidock-gpu` debug 产物 43 MB；release **7.2 MB**。
- 常驻内存 271 MB；同卡空设备基线 dx12 164 MB / vulkan 103 MB / d3d11 34 MB
  （AMD 610M）。ADR-001 复核已执行完毕，结论见 ADR-011（已否决）/ ADR-012。
- 首帧与稳态画面不同（首帧字体/布局尚未稳定），截图核对必须等第二帧之后。
