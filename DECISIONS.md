# 技术决策记录（ADR）

每条记录：背景 → 决定 → 代价 → 何时重新考虑。
新决策追加在末尾，不删除历史（错误决策留着才有人知道为什么不能重犯）。

---

## ADR-001 · 渲染后端选 wgpu，不选 D3D11/D2D

**背景**：原版 GDI+ 是 CPU 光栅 + 每次 `UpdateLayeredWindow` 整块 blit。
产品要求 GPU 渲染与多线程管线。

**决定**：`wgpu 30`（经 `egui_wgpu::wgpu` 重导出，不单独锁版本）。

**理由**：
- Rust 生态已在此层收敛（Firefox WebGPU、Bevy、egui、iced、Dioxus 原生端）。
- DX12/Vulkan/Metal 的驱动怪癖由上游吸收，我们只维护业务。
- D3D11/D2D 更轻（30–50 MB vs 60–120 MB），但 Rust 侧没有框架用它：
  等于连文本 shaping、IME、控件、测试全部自己写，且没有同行排雷。

**代价**：常驻内存与体积上一个量级；Dock 的"极轻"定位正式放弃。

**实测（2026-10-27，`scripts/probe-mem.sh`，按适配器分组）**：

| 后端 | AMD 610M（核显） | NVIDIA 5070（独显） |
|---|---|---|
| dx12 / wgpu | 164 MB | 87 MB（峰值 102） |
| vulkan / wgpu | 103 MB | 80 MB |
| d3d11 原生 | **34 MB** | **39 MB** |
| OpenGL（WGL） | — | 52 MB |

**判据已触发，且已归因**：不是显卡驱动的固有成本，而是 wgpu-hal + D3D12 运行时。
同一块卡上原生 D3D11 只需 1/3–1/5。附加的 19 MB 缓冲在 wgpu 下还会放大成
+64 MB 工作集。

**结论修正**：ADR-001 里"D3D11 更轻、但 Rust 侧没人用它做 UI"这两句
前半被证实（轻 2.5–5 倍），后半依然成立（没有成熟的 D3D11 UI 框架）。
所以曾提议换渲染后端（ADR-011）。**该提议已被否决**：决定保留
`egui-wgpu` 单栈，适配器选择另立 ADR-012。

---

## ADR-002 · UI 框架选 egui，且**不用 eframe**

**决定**：`egui 0.37` + 自持 Win32 消息循环 + `egui-wgpu`。

**理由**：
- Dock 是画布：状态 → 每帧重画，即时模式天然贴合；iced 的 Elm 架构要把
  缓动/拖拽塞进 `update`，是逆着框架用。
- eframe 会接管事件循环，而托盘、全局热键、AppBar、`WM_NCHITTEST`
  都必须在 `WndProc` 里说话。
- 附带好处：一个循环、一个 device、原生弹窗（菜单/组/DWM 预览）可共存。

**代价**：输入翻译自己写（约 300 行）；IME 需桥接（见 ADR-006）。

**重新考虑**：若 IME 桥接评估后发现成本过高，退路是设置窗口另起
eframe 线程（独立循环 + IPC），但内存会在设置打开时翻倍。

---

## ADR-003 · 设置界面独立成 crate，不依赖 wgpu

**决定**：`minidock-settings` 只依赖 `egui` + `minidock-core`。

**理由**：这是"可测试性"的全部来源。9 页、约 40 个控件，如果与 wgpu 耦合，
就只能靠人眼看；拆开后 kittest 可以在无显示器环境点完每一页。

**代价**：多一层 crate 边界与 `Command` 枚举（UI 不能直接调副作用）。

**重新考虑**：不会。这条边界是廉价的，收益随设置项增长。

---

## ADR-004 · 中文字体用系统 TTC，不打包子集

**决定**：运行时读 `C:\Windows\Fonts\msyh.ttc`（`FontData.index = 0`），
按 `msyh → Deng → simhei → simsun` 顺序尝试。

**理由**：打包 Noto Sans SC 子集要 +1–3 MB；系统字体本来就在，
且 `epaint::FontData` 支持 TTC 的 face 索引。

**代价**：无字体的机器会崩 —— **egui 0.37 缺字形是 `panic` 而不是豆腐块**
（`MissingGlyphPolicy::Panic`），所以必须在第一帧之前装字体，失败时明确报错。

**重新考虑**：若要做绿色便携版到无中文字体的环境，再打包子集。

---

## ADR-005 · 逐像素命中用 `WM_NCHITTEST`，不用轮询

**决定**：在窗口过程里按布局算命中，返回 `HTTRANSPARENT` / `HTCLIENT`。

**理由**：GPU 窗口没有 `UpdateLayeredWindow` 那种"alpha=0 自动穿透"，
`winit::set_cursor_hittest` 只有整窗开关。`WM_NCHITTEST` 是系统每次点击
都会问的同步钩子，天然没有"关掉穿透才收得到消息"的鸡生蛋问题。

**代价**：命中判定要自己维护（圆角、图标形状、气泡溢出区）。

**重新考虑**：不会。M1 要把矩形近似换成圆角 SDF 判定。

**修订（M0 实测，2026-10-27）——本条已作废。**
ADR-008 改用离屏渲染 + 分层窗口之后，`UpdateLayeredWindow` 的逐像素 alpha
重新生效：**alpha=0 的像素自动穿透，不需要 `WM_NCHITTEST`**。
`overlay.rs` 里手写的命中代码已删除，改由系统按位图 alpha 判定。
保留本条是因为它是"GPU 窗口 + 不分层"分支上的正确结论 —— 一旦将来换成
DirectComposition 自建交换链而放弃分层窗口，这条会重新适用。

---

## ADR-006 · IME 先叠原生 `EDIT`，不硬啃 `WM_IME_*`

**决定**：设置页需要键盘输入的地方（添加网址/命名）先叠一个原生 `EDIT`
控件；`WM_IME_*` → `egui::Event::Ime` 的桥接放到 M4 之后。

**理由**：原版 C 就是这么做输入框的；IME 组合串、候选窗定位、TSF 兼容
是典型的"看起来简单、边界极多"的活，不值得在 M1 就赌。

**代价**：视觉上输入框不是 egui 画的，需要对齐主题（圆角/内边距/字体）。

**重新考虑**：M4 之后若输入场景变多，再评估自绘 + IME 桥接。

---

## ADR-007 · CPU 并行用 rayon + 专用 IO 线程，不引入 tokio

**决定**：数据并行（缩放/预乘/模糊 fallback）用 rayon；
窗口扫描、favicon 下载、缩略图抓取用固定线程 + `crossbeam-channel`。

**理由**：这里没有大量并发连接，只有几条长驻线程 + 一个线程池。
async 运行时的调度开销与每任务栈占用都不划算，调试也更难。

**跨线程数据**：`ArcSwap` 存只读快照（UI 线程无锁读），
通道传任务与结果。GPU 上传只在 UI 线程做（纹理与 device 绑定）。

---

## ADR-008 · 透明窗口：wgpu 表面做不到，改离屏 + 分层窗口

**背景（M0 实测，2026-10-27）**：
- 默认 `wgpu::Instance` 在本机选了 Vulkan，`alpha_modes` 无
  `PreMultiplied`/`PostMultiplied`。
- 强制 DX12 后仍然只有 `Opaque`。**wgpu 的 DX12 表面不提供 alpha 模式。**
- 直接 `LoadOp::Clear(TRANSPARENT)` 的结果是黑色，不是透出桌面。

**状态**：第 1 步已在 M0 实现并实测通过（`present.rs`）。

**决定（分两步）**：
1. **M1 走离屏 + 分层窗口**：wgpu 渲染到 `Rgba8UnormSrgb` 纹理 → copy 到
   buffer → 预乘后写入 DIB → `UpdateLayeredWindow`。
   优点：逐像素 alpha 与命中免费获得，立刻可用。
   代价：每帧一次回读（Dock 尺寸约 1–6 MB，桌面级带宽可接受）。
2. **M3 评估 DirectComposition**：`WS_EX_NOREDIRECTIONBITMAP` + DComp visual
   + 自建 DXGI 交换链（经 `wgpu-hal` 或原生 D3D12）。得到真正的零拷贝
   GPU 合成，但集成深、调试成本高。

**为什么不是 D3D11+D2D**：那会推翻 ADR-001 的生态选择，
且 wgpu 的渲染代码要重写；离屏回读只是加法。

**风险**：分层窗口与 DXGI 交换链不能共存，所以 M1 的 `Surface` 逻辑要
整体替换为"离屏纹理 + 回读"，`overlay.rs` 的呈现路径会重写（渲染逻辑不动）。

**重新考虑**：若 M1 实测回读导致掉帧（>2 ms/帧），提前上 DComp。

---

## ADR-009 · 设置窗口可以是不透明的普通窗口

**背景**：ADR-008 之后，透明能力只对 Dock 必需。

**决定**：设置窗口用普通无边框窗口（不透明、可拖动、带阴影），
不追求毛玻璃背景。M0 尖刺就是这种形态。

**理由**：设置界面是模态性质的临时窗口，透明不带来任何功能收益，
却会强迫它继承 Dock 的分层窗口复杂度。

---

## ADR-010 · 保留 GDI+ 实现到 M2 结束

**决定**：`minidock-gfx` / `minidock-ui` 暂不删除，作为行为参照与 fallback。

**理由**：图标形状、圆角、文字基线这些细节，最可靠的对照物是能跑的旧实现。
M2 图标迁移完成后，两份实现并排截图差分，然后删除。

**代价**：工作区同时构建 wgpu 与 GDI+，编译时间变长。

---

## ADR-011 · 渲染后端改用 `egui_glow`（OpenGL）替代 `egui-wgpu`【已否决】

**决定（2026-10-27）：不换。保留 `egui-wgpu`。**
理由是维持单栈：换 OpenGL 要自写 FBO 离屏回读（约 250 行）、
并为 RDP/虚拟机落到 GL 1.1 再准备一条回退路径；
换来的是内存从 150–270 MB 降到 55–80 MB，但当前优先级是
减少维护面而不是压内存。**数据保留**，将来若内存重新成为硬指标
（例如要进低配设备），这条可以直接翻案。

以下是原提议正文，留档：

**背景**：ADR-001 的复核条件已触发。同卡实测：wgpu-DX12 空设备 87–164 MB，
同一块卡的原生 D3D11 只有 34–39 MB，OpenGL 52 MB（`BUILD.md` 有完整矩阵）。

**提议**：把 `egui-wgpu` 换成 `egui_glow 0.37`，**其余不动**：
egui 本体、kittest 用例、9 页设置界面、离屏 + `UpdateLayeredWindow` 呈现全部保留。

**收益**：常驻从 150–270 MB 落到 55–80 MB 量级；UI 代码与测试零改动；
仍然是 GPU 渲染 + CPU 多线程。

**代价**：
- 要自己写 GL 版离屏 → 回读（FBO + `glReadPixels`/PBO），约 250 行，
  替换 `offscreen.rs` 的 wgpu 实现。
- 远程桌面 / 虚拟机可能落到 `Microsoft GDI Generic`（GL 1.1）——
  必须运行时探测 GL 版本并提供回退（软件光栅或 D3D11）。
- 混合显卡上 GL 上下文可能落在独显（本机就是），功耗高于核显渲染；
  钉核显要 `WGL_NV_gpu_affinity`（仅 NVIDIA）。
- GL 的驱动差异更"古老"：`GL_UNPACK_ALIGNMENT`、sRGB 帧缓冲、
  Y 轴翻转都要逐个验证。

**不选的替代**：D3D11 + D2D（39 MB，但 UI 必须自研，失去 kittest）；
继续 wgpu（代码不动，但常驻翻倍）。

**何时重新评估**：目标是老旧 Intel 核显、或必须支持 RDP 会话时——
先跑 `PROBE_BACKEND=gl` 看 `glGetString(GL_VERSION)`，拿不到 3.3+ 就回退。

---

## ADR-012 · 显卡选择按名字匹配，默认自动

**背景**：需要在设置里选显卡，默认自动。候选方案三种：存索引、存名字、
用系统级 `UserGpuPreferences` 注册表项。

**决定**：`[Graphics] Adapter` 存**适配器名字**，空串 = 自动；运行时在 DX12
实例里按名字匹配（先精确忽略大小写，再退化到互相包含）；匹配不到**回退自动**，
并把候选写进日志与设置界面提示。

**理由**：

- **不存索引**：DXGI 与 wgpu 的枚举顺序都不保证稳定，换驱动、插拔外接显卡、
  系统更新都会变。存索引迟早出现"选了 A 实际用了 B"，且无法从配置里看出来。
- **不用 `UserGpuPreferences`**：它是系统级、只在进程启动时读取，还要写 HKCU
  （便携版不该留这种痕迹）。wgpu 的适配器枚举已经能精确到具体设备，
  没有理由绕一圈。
- **失配必须回退而不是报错**：配置跟着 ini 走，换机器后名字必然失效；
  因为一个显示偏好而起不来是最差的选择。

**代价**：

- 两块同名同型号的卡无法区分（SLI/多卡工作站）。当前不做处理。
- 名字是驱动上报的字符串，跨语言/跨版本可能变（例如驱动更新后加了后缀）。
  已经用"互相包含"兜住常见情况，但极端的重命名仍会回退。

**何时重新评估**：需要"多 GPU 分工"（Dock 用核显、缩略图用独显）时，
名字不够用，得直接传 `IDXGIAdapter` 或改用 LUID。

---

## ADR-013 · 内存调优只动 `DeviceDescriptor`，不换框架

**背景**：实测 wgpu-DX12 空设备 164 MB（AMD 610M），而同一块卡的原生 D3D11
只要 34 MB。用户要求在不换 egui-wgpu 的前提下把内存降下来。

**决定**：只改两处，均已落地在 `Gpu::device_limits` 与 `request_device`：

1. `max_non_sampler_bindings`: 1_000_000 → **4096**
2. `memory_hints`: `Performance` → **`MemoryUsage`**

实测空设备 164 → 102 MB（−38%），实际 spike 271 → 111 MB（−59%）。

**理由**：两条都在 wgpu 文档里被点名会吃内存，且都是"设备创建时"的一次性成本，
和渲染逻辑无关，改了不影响画面（像素核对一致）。

**代价**：`MemoryUsage` 让分配器少留余量，理论上每帧分配会更频繁地落到
新块上。本项目的每帧分配量是几个 KB 级（egui 顶点/索引 + 一次纹理上传），
影响应在噪声内；M1 接上性能页后要用帧耗时复核一次。

**同时修掉的隐患**：原先请求 `Limits::downlevel_defaults()`，它的
`max_texture_dimension_2d` 只有 2048，2560 宽的 Dock 在 150% DPI 下需要
3840 px 的离屏纹理 —— 会在超宽屏上建纹理失败。改为 `..adapter.limits()`。

**否决的替代**（都已实测，见 `BUILD.md`）：wgpu 自带 GL 后端 117 MB（更差，
且落独显）；原生 GL 52 MB、原生 D3D11 34 MB（都要换渲染层，ADR-011 已否决
换 `egui_glow`）。

**何时重新评估**：内存重新成为硬指标时（例如要进 4 GB 设备或远程桌面场景），
按 `BUILD.md` 的表从低到高换：`egui_glow` + FBO 回读 → 原生 D3D11 + 自研控件。

---

## ADR-014 · 重绘由 egui 的 `repaint_delay` 驱动，不用固定间隔轮询

**背景**：M0 的消息循环是

```rust
if ctx.has_requested_repaint() {
    render();
    std::thread::sleep(Duration::from_millis(8));
    continue;
}
```

只要 egui 想动画，就以 8 ms 为间隔空转：既不知道"下一帧该在什么时候"，
也不知道"其实根本没活干"。

**决定**：用 `FullOutput.viewport_output[ROOT].repaint_delay` 作为调度信号：

| egui 给的 delay | 含义 | 我们的动作 |
|---|---|---|
| `Duration::ZERO` | 尽快再画一帧 | 排到 `now + 16ms`（60 fps 节流） |
| 有限值 | 动画进行中 | 排到 `now + delay` |
| `Duration::MAX` | 没有待办 | `None` → `MsgWaitForMultipleObjectsEx(INFINITE)` |

等待用 `MsgWaitForMultipleObjectsEx(None, timeout, QS_ALLINPUT, MWMO_INPUTAVAILABLE)`：
消息来了立刻醒，否则睡到下一帧截止时刻。输入（鼠标/键盘）不走这条路 ——
`WndProc` 直接置 `needs_frame`，下一轮立即出帧，不牺牲响应速度。

**每轮先把消息队列抽干再渲染**，避免"一条消息一帧"的抖动。

**实测**（release，单实例，AMD 610M）：空闲 30 秒 CPU 增量 **0.000 秒**；
设置窗静止时进程完全阻塞在 `INFINITE` 上。

**代价**：`Duration::ZERO` 被节流到 16 ms。如果将来需要 120 Hz 或
无节流的拖拽动画，需要把 `FRAME_PACING` 改成按显示刷新率取值。

**踩过的坑**：判定写成 `next_repaint_at.is_none_or(|at| now >= at)` 会
把"没有待办"当成"立刻要画"，于是每来一条消息就重绘一次，
空闲 CPU 从 0% 涨到 1.5%。正确写法是 `is_some_and`。

---

## ADR-015 · 单实例用命名互斥体，第二次启动只唤起窗口

**背景**：M0 的尖刺没有接单实例，双击两次就有两个分层窗口重叠。
外部观测时表现为"空闲还占 8% CPU"，一度被误判成渲染循环的问题 ——
实际是两个进程各画一份。

**决定**：启动时 `SingleInstance::acquire()`：

- `Ok(Some(guard))` → 正常启动，guard 活到进程结束。
- `Ok(None)` → 已有实例，`find_window("MiniDockOverlay")` 后
  `SetForegroundWindow` 并退出（退出码 0）。
- `Err(_)` → 互斥体本身失败，**照常启动**：宁可多一份 Dock，也不要起不来。

**代价**：类名成了跨进程契约（`overlay::CLASS_NAME`），改名要同步。

---

## ADR-016 · 回读位图按 u32 搬运，不逐字节赋值

**背景**：`UpdateLayeredWindow` 要 BGRA，而 egui 输出 RGBA，
每帧要交换 1320×1080 = 140 万像素的 R/B 通道。

**决定**：把两个缓冲都按 `&[u32]` 处理，用位运算一次完成交换：

```rust
*d = (*s & 0xFF00_FF00) | ((*s & 0x0000_00FF) << 16) | ((*s & 0x00FF_0000) >> 16);
```

**实测**（release，同机）：`0.80 ms/帧` vs 原逐字节写法 `1.16 ms/帧`，**1.45x**。

**为什么不用 SIMD**：1.45x 已把这块压到 1 ms 以内，
而真正的开销在 GPU 回读与 `UpdateLayeredWindow`（数毫秒级）。
再用 `std::simd` 属于优化错对象，且会引入 nightly 依赖。

**注意**：基准测试里"旧写法"必须原样搬原代码。第一版用
`to_le_bytes`/`from_le_bytes` 构造替身，测出来只有 1.1x —— 那是在
和另一段代码比，不是和自己比。

---

## ADR-017 · 配置键表表驱动，跟上游同步的代价是"加一行"

**背景**：上游在 2026-10-28 发布了 v1.4.0，一次把 `[Settings]` 从 37 键扩到
**76 键**（新增 41 个、删掉 `PlateDark`/`PlateLight`），并改了布局公式
（`SepGap`、倒影尺寸、时钟宽度规则）。上游还在持续迭代，同步是长期工作。

**决定**：把 41 个"裸 int"键放进静态表 [`NUMERIC_KEYS`]，
`Key::Numeric(i)` 指向表里的行。类型化的 35 个键（主题、边缘、枚举…）
仍然是普通变体。

**为什么是这个切法**：

- 那 41 个新键在上游 C 版里就是裸 `int`（`&cfg.xxx` 直接读写），没有枚举语义，
  做成枚举变体纯属自找麻烦。
- 表驱动之后，**加一个上游键 = 加一行数据**，而不是改 8 处 match
  （枚举、ALL、name、range、is_bool、get、apply、defaults）。
- 键的**顺序**仍然显式写在 `Key::ALL` 里，因为写回 ini 的行序必须与 C 版一致；
  这一点由 `key_table_matches_c_g_tab` 测试守住（它把上游 76 个键名逐个列出）。

**代价**：

- 具名常量（`Key::SEP_GAP` 等）里写死了表下标，表一重排就全错。
  用 `numeric_constants_match_table_rows` 测试守住。
- 数值键的取值域是我们自己定的（上游对这些 int **完全不夹紧**，
  越界直接进绘制代码）。值域取自上游设置行（`RSl` 上下限、`RCmb` 选项数、
  `RTog` = 布尔、`RCol` = 颜色）。

**同步流程**（下次上游再改时照做）：

1. `git -C upstream fetch && git diff HEAD..origin/main -- MiniDock-source.zip`
2. 解包对比 `g_tab[]` / `DefaultCfg()` / 设置行。
3. 键有增减 → 改 `NUMERIC_KEYS` 表 + `Key::ALL` + 具名常量 + 两个测试。
4. 布局公式变了 → 改 `core::layout::calc_metrics` 并补测试。
5. `cargo run -p minidock-core --example config_table` 重新生成 `CONFIG.md` 的键表。

**已同步的内容**（v1.4.0）：

- 76 键全部建模，顺序与上游一致，夹紧规则比上游更严。
- `PlateDark`/`PlateLight` → `PlateMode`/`PlateTheme`/`PlateOpacity`。
  **上游没有做兼容处理**，我们也不做：旧键留在 ini 里但不再生效（不丢数据）。
- 布局公式更新：分隔线宽度改由 `SepGap` 决定、倒影高度改由
  `ReflSize`/`ReflGap` 决定、时钟卡片宽度改由
  `ClockShow`/`ClockSec`/`DateFmt`/`ClockWeek` 算出。
- 设置界面补上与渲染相关的 20 个新键（外观 / 图标 / 交互 / 附件 / 性能页）。

---

## ADR-018 · 命中与放大映射按 C 版语义，不按"直觉"

**背景**：M1 要与 C 版几何对齐（≤1 px）。核对我们早期实现与上游 v1.4.0 的
`Paint()` 时，发现两处语义不同。

**决定**：照抄 C 版。

1. **命中取"从左到右第一个"**，不是"最近的那个"。
   C 版 `HitIndex()` 遍历槽位，命中 `[slotL - g/2, slotR + g/2)` 就返回；
   我们早期实现遍历所有槽位取距离最小的。
   放大后相邻图标会重叠，两个都覆盖鼠标点时两者会选**不同的图标**。
2. **放大权重用"基准位置"算**。
   C 版 `MapActualToBase()` 把鼠标的**实际**位置（此时槽位已被放大挪动）
   映射回**基准**位置（未放大时的槽位），再算余弦权重。
   直接用实际位置会让"鼠标属于哪个图标"随放大程度漂移。

**代价**：`hit_index` 的参数变多（9 个），但每一个都对应 C 版的一个全局量，
拆开反而更难对照。

**顺带修正**：`map_actual_to_base` 在条目数量变化时放弃映射（对应 C 版
`prevN != nItems` 的保护），否则新旧槽位会被错误地插值。

---

## ADR-019 · egui 的原生缩放走 `ViewportInfo::native_pixels_per_point`

**背景**：egui 0.37 不再从 `RawInput` 直接读 `pixels_per_point`，
改由 `RawInput.viewports[ROOT].native_pixels_per_point` 告知。

**决定**：凡是自持 Win32 循环的地方（`input.rs`、`minidock-dock`），
构造 `RawInput` 时都补上：

```rust
raw.viewports.insert(
    egui::ViewportId::ROOT,
    egui::ViewportInfo {
        native_pixels_per_point: Some(ppp),
        ..Default::default()
    },
);
```

**为什么必须补**：不补时 egui 按 1.0 缩放 —— 界面只占表面的 1/ppp，
表现为"整体偏左上、尺寸偏小"；在 150% DPI 下文字被放大后发虚。
M0 的设置界面一直有这个问题，本轮才定位到。

**教训**：`screen_rect` 决定画布大小、`native_pixels_per_point` 决定缩放，
**两者缺一不可**，而且症状不同（一个偏、一个虚），容易只修一个。
