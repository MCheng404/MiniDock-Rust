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

---

## ADR-020 · 圆角 + 渐变要自己搭网格

**背景**：Dock 栏体的圆角填充画成了方角，只有描边是圆的。用户直接看出来了。

**根因**：`egui::epaint::Shape::gradient_rect()` 只是**一个 4 顶点四边形**
（`shape.rs:329`），没有半径参数；而支持圆角的 `Painter::rect_filled()`
只接受单色。两者不可叠加，于是"填充方角 + 描边圆角"。

**决定**：`dock.rs` 里的 `rounded_gradient()` 自己产出 `Mesh`：
从矩形中心向圆角轮廓拉三角扇，顶点色按渐变方向线性插值。

**为什么三角扇就够了（不是近似）**：线性渐变的颜色是位置的**仿射函数**，
三角形内的重心插值对仿射函数是精确的。所以只要顶点色取在该点，插值结果
与"真渐变"逐像素一致。

**代价与补偿**：`Shape::Mesh` 走 tessellator 的直通分支，**不做** epaint 的
1px 羽化，边缘会锯齿。因此在轮廓外再放一圈同色全透明顶点模拟羽化 ——
这就是 epaint 自己抗锯齿的做法。

**验证**：角点像素 `(117,119,124)` → `(255,255,255)`，与 C 版完全一致；
栏体内部颜色不变（`(177,219,228)`）。回归测试
`rounded_gradient_leaves_the_corners_empty` 断言四角 0.3·r 范围内没有顶点。

---

## ADR-021 · 搬 C 的颜色一律走 `argbc()`

**背景**：原版颜色全写成 `ARGBC(alpha, r, g, b)`（alpha 在前），
而 Rust 的 `Color32::from_rgba_unmultiplied(r, g, b, a)` 是 alpha 在后。
逐条手抄时按字面顺序写进去，得到的是**另一个颜色**——不崩溃、不报警，
只是安静地难看。

**实际造成的错误**（本轮全部修掉）：

| 位置 | 本意 | 实际画成 |
|---|---|---|
| 图标间分割线 | 白 110 alpha | 青绿不透明 |
| 时钟分割线 | 白 90 alpha | 青绿 |
| 运行指示点外圈 | 黑 120 alpha | 纯红的半透明 |
| 气泡底 | 深灰 228 alpha | 红色 38 alpha |
| 图标底盘（纯色档） | 浅灰 (244,245,249) | 粉白 |

**决定**：新增 `fn argbc(a, r, g, b) -> Color32`，参数顺序与 C 一致。
以后凡是搬 C 的颜色只走它，顺序问题只在一个地方存在。
配一条测试 `argbc_keeps_alpha_first` 把语义钉死。

**教训**：这类错误不会让程序出错，只会让界面"说不出哪里不对"。
凡是"两边参数顺序不同"的 API，都应该包一层适配函数而不是逐处小心。

---

## ADR-022 · 时钟文字归 `core::clock`，日期有横竖两套写法

**背景**：Rust 版把时钟文字写在 `bin/dock.rs` 里，只有一句
`format!("{}月{}日 周{}", …)`。与 C 版并排截图时发现**少了年份**：
C 版是 `2026年10月30日 周五`，我们是 `10月30日 周五`。

**根因**：原版 `ClockLines()` 按停靠方向分两套：

| 方向 | `DateFmt=0` | 说明 |
|---|---|---|
| 横（上/下） | `2026年10月30日 周五` | 卡片宽，一行放得下 |
| 竖（左/右） | `10月30日`，周几**单独一行** | 卡片一行只有 21 px，塞年份会挤成一团 |

我们抄了竖排那套，横排就少了年份。

**决定**：新增 `core::clock`：

- `ClockParts`：时间快照，无 Win32 依赖（字段对应 `SYSTEMTIME`）。
- `clock_lines(cfg, parts) -> Vec<ClockLine>`：`ClockShow` / `ClockSec` /
  `ClockWeek` / `DateFmt` / `ClockOrder` 与停靠方向全部处理，返回行序。
- `ClockLine::is_date` 供绘制时决定"组间是否画分割线"。

绘制端改成**按行数均分卡片高度**（原版 `lh = (ch0 - 8) / nl`），并在
时间组与日期组之间画一条细线。原实现把两行位置写死成 12%–56% / 54%–86%，
只显示时间或只显示日期（`ClockShow=1/2`）时会错位。

**代价**：`Strftime` 之类的现成格式化库用不上（原版就是手拼），
多语言日期格式要自己加分支。当前只做中文，与原版一致。

---

## ADR-023 · Dock 的输入层：鼠标用物理像素、脏标记驱动重布局、点击认"同一图标"

**背景**：Dock 能画出来、悬停会放大，但点图标没有任何反应。接输入时连着
踩了三个问题，都是"能编译、能运行、只是不工作"这一类。

### 一、鼠标坐标必须与几何同单位

`Dock::mouse` 存的是逻辑点（`x / ppp`），而槽位、栏体、`lh` 全是**物理像素**
（`metrics` 与 C 版一致）。在 150% DPI 下命中区整体偏移 1.5 倍 ——
表现为"鼠标移到图标右边才放大"。

**决定**：`mouse` 改存物理像素，只在绘制时经 `Proj::k` 换算一次。
`WM_*MOUSE*` 给的本就是客户区物理像素，不需要转换。

**顺带记一个判据**：`metrics.lw/lh` 是物理像素，因为窗口尺寸用的就是它
（`CreateWindowExW` 收物理像素）。凡是与 `slot_l` / `bar_h` 比较的值，
都必须是物理像素。

### 二、重布局要由输入事件驱动，不能只看 `anim`

主循环原先只写 `if dock.anim { dock.update(dpi) }`，而 `hot` 只在 `update`
里算。动画一收敛 `anim == false`，鼠标再进来也不会重算 `hot` ——
**悬停放大在静止状态下完全不工作**。

**决定**：`Dock` 增加 `dirty` 标记，`WM_MOUSEMOVE` / `WM_MOUSELEAVE` /
`WM_LBUTTONDOWN` / `WM_LBUTTONUP` 都调 `mark_dirty()`；循环条件是
`dirty || anim`。

### 三、点击必须"按下与抬起在同一个图标上"

原版是 `pressIdx = HitIndex(down)`，抬起时 `if (HitIndex(up) == pressIdx)`。
少了这条，从图标上按下、拖到别处松手也会启动。

**决定**：`Dock` 提供 `press_at` / `release_at`，后者返回 `Option<usize>`
并存进 `click`。主循环用这个下标启动，**不用 `hot`** ——
两者在消息乱序或注入输入时会不一致。

### 四、Dock 也要单实例

尖刺有单实例，Dock 没有。结果调试时很容易同时跑起两个 Dock，
叠出两个分层窗口，而 `FindWindow` 只返回其中一个 ——
外部工具（截图、自动化）会操作错的那个，排查时极具误导性。

**决定**：`minidock-dock` 接 `SingleInstance::acquire()`，第二次启动直接退出。
互斥体本身失败时不阻止启动（与尖刺一致）。

### 排查手法（值得复用）

`PostMessage` 模拟点击时，投递的客户区坐标被乘了 1.5。原因是
**投递方进程不是 DPI 感知的**，Windows 对它的坐标做了虚拟化。
工具里加一行 `SetProcessDPIAware()` 即解决。

教训：跨进程传坐标时，两端的 DPI 感知状态必须一致，否则数字会"自己变"。

---

## ADR-024 · 拖拽重排：状态收进一个结构体，重排只挪一格

**背景**：Dock 之前点了能启动，但图标不能拖动。

**决定**：

1. **拖拽状态收进 `DragState`**（`index / pt / active / press_pt`）。
   原版对应 `pressIdx` / `pressPt` / `dragging` / `dragIdx / dragPtA` 五个全局量。
   拆成多个字段时很容易出现"`dragging` 为真但 `dragIdx` 还是 -1"的半截状态，
   收进一个 `Option<DragState>` 后这种状态在类型上就不存在。

2. **起拖阈值 6 px**（原版 `abs(dx) > 6`）。没有它，每次点击都会先用几像素
   的"拖动"把图标挪一下，观感很差。

3. **重排一次只挪一格**，由"数一数有多少条目的实际中心在光标左边"得出目标下标。
   这样每帧最多 `remove`+`insert` 一次，位移变化是连续的，`slide` 动画才自然。

4. **`place_for` 必须显式排除被拖项**。第一版从 `self.drag` 里读下标，
   而 `drag_to` 开头已经 `take()` 了状态 —— 读到 `None`，于是把自己也算进
   "左边有几个"，每次多挪一格。改成传参。

5. **拖动后松手不算点击**。`release_at` 里先判断"拖过没有"，否则每次重排
   都会顺带启动一个应用。

6. **重排要同步给调用方**。`Dock` 只持有名称与类型，启动用的完整 `Item`
   在主循环那边。用 `pending_move: Option<(from, to)>` 单向下发，
   主循环逐条镜像到自己的列表，最后写回 ini。

**代价**：拖出栏体删除、合并成组还没做（原版有）。`removing` 状态位先不加，
避免留下"能显示却不会生效"的字段。

**排查教训**：残影一开始"看不见"，查了很久才发现是**测试工具的时序**问题 ——
`PostMessage` 造的拖拽只有约 420 ms，自动化截图几乎必然落在松手之后。
工具里加 4 s 停顿后立刻正常。先怀疑工具，再怀疑实现。

---

## ADR-025 · 自动隐藏：必须轮询光标，且不能每轮都重绘

### 一、收起态只能靠轮询

收起后栏体被推到窗口可视区之外，那片区域的 alpha 是 0。**分层窗口的
鼠标命中是按 alpha 判定的**，于是鼠标事件直接穿透，永远等不到
`WM_MOUSEMOVE` —— 只看消息的话 Dock 收起来就再也弹不回来。

原版就是用定时器 + `GetCursorPos()` 轮询解决的，这里照做：开启自动隐藏时
把等待超时从 `INFINITE` 改成 100 ms，每轮取一次光标位置。

### 二、两套"靠近"判据不能混用

| 状态 | 判据 |
|---|---|
| 已收起 | 光标贴住 Dock 所在的那条**屏幕边**（±1~2 px），且落在窗口的横向范围内 |
| 已显示 | 光标落在栏体外扩一圈的范围内（含放大溢出区） |

收起态若仍按"靠近栏体"判断，光标永远够不着 —— 栏体已经不在屏幕上了。

### 三、宽限期 450 ms

鼠标划过屏幕底边去点任务栏时不该把 Dock 弹出来；刚离开就立刻缩回去也很烦。
原版是 450 ms，照抄。

### 四、拖拽/菜单期间强制显示

`busy` 为真时一律保持显示并刷新计时。拖到一半 Dock 缩回去是最糟的体验。

### 五、按需出帧（本项目自己的问题）

自动隐藏要 100 ms 轮询，而主循环原本**每轮都重绘** —— 空闲时会变成
10 fps 的无谓渲染，M0 好不容易压到 0% 的 CPU 立刻失守。
改成用 `need_frame` 标记：收到消息 / `update` 跑过 / 隐藏状态变化时才画。

实测收起态空闲 10 秒 CPU 增量 **0.000 秒**。

### 未实现

`SmartHide`（`Mode=2`，被别的窗口遮挡才收起）需要枚举可见窗口并求交，
本轮没做。当前按常显处理，并在启动日志里明说 —— 不假装支持。

---

## ADR-026 · 桌面预留走 AppBar，且必须 RAII 注销

**背景**：`Reserve`（最大化窗口避让 Dock）之前完全没实现，配置项存在但不生效。

**决定**：`platform::appbar::AppBar` 封装 `SHAppBarMessage` 四步
（`ABM_NEW` → `ABM_QUERYPOS` → 修厚度 → `ABM_SETPOS`），
`Drop` 时 `ABM_REMOVE`。

### 为什么必须 RAII

AppBar 登记是**系统级**的：登记后 `rcWork` 永久少一块。若进程不注销，
用户桌面会一直缺一条。实测确认 Windows 会在**进程正常或异常退出后**
回收登记（强杀之后 `rcWork` 也恢复了），所以 RAII 不是为了兜底崩溃，
而是为了"退出前就归还"——避免退出瞬间到系统回收之间那段窗口里，
别的程序按缩小后的工作区排列窗口。

### 为什么不用 C 版的 hash key 缓存

C 版用 `hash(边, 厚度, 显示器矩形)` 判断要不要重新提交。问题在于
**key 之外的因素也会改变结果**：别的 AppBar 退出、任务栏换边、
分辨率变化。这些都不在 key 里，于是 Dock 会卡在旧位置。

改成每次都走 `ABM_QUERYPOS`（纯查询，无副作用），结果真的变了才
`ABM_SETPOS`。多一次调用换来"上层变化时也能跟上"。

### 一个容易误解的地方

正常预留时 **Dock 自己的窗口位置并不变**：AppBar 是从 `rcWork` **内部**
切走一块，两者共享同一条外边。变的是**别的窗口能不能用那块地方**。
（`layout::window_origin` 的测试里专门钉住了这条，因为很容易被误解成
"预留后 Dock 会挪位置"。）

真正会让窗口挪位置的是 `ABM_QUERYPOS` 把请求矩形挤开的情况 ——
已有别的 AppBar 占了同一侧时会发生。那时必须用返回的 `ab.rc` 定位，
用 `rcWork` 会两个 AppBar 叠在一起。

### 实测

底边停靠、`barH=76 / margin=12`：

| 状态 | `rcWork` |
|---|---|
| 未运行 | `(0,57)-(2560,1600)` |
| 运行中（Reserve=1） | `(0,57)-(2560,1508)` ← 少 92 px |
| 正常退出后 | `(0,57)-(2560,1600)` |
| 强杀后 | `(0,57)-(2560,1600)`（系统回收） |

窗口位置 `(918,1414)`：底边 1600 − 高 186，水平居中。

---

## ADR-027 · 智能隐藏的遮挡判定必须枚举窗口

**背景**：`Mode=2`（被遮挡才收起）之前只打一句"尚未实现"的日志。

**决定**：`platform::occlusion` 用 `EnumWindows` 枚举顶层窗口，
过滤后与栏体的屏幕矩形求交。任一相交即视为被遮挡。

### 为什么不能用 `WindowFromPoint`

分层窗口在命中测试里算"在上面"，`WindowFromPoint` 会返回 Dock 自己，
拿不到被它盖住的那个窗口。必须自己枚举 + 求交。

### 过滤清单（每一条都有具体理由）

| 过滤项 | 不过滤的后果 |
|---|---|
| 自身窗口 | 自己挡自己，永远"被遮挡" |
| `!IsWindowVisible` / `IsIconic` | 最小化窗口的矩形仍在原位，会被误判 |
| `WS_EX_TOOLWINDOW` / `WS_EX_NOACTIVATE` | 输入法候选、悬浮工具栏浮在最上层，会让 Dock 莫名躲起来 |
| 有 `GW_OWNER` | 对话框/弹出菜单的宿主不算独立应用窗口 |
| 标题为空 | 不可见的容器窗口 |
| DWM cloaked | UWP 幽灵窗口，用户看不见但矩形有效 |
| `Progman` / `WorkerW` / `Shell_TrayWnd` | 桌面与任务栏铺满全屏，会让 Dock 永远处于被遮挡 |

`GetWindowTextLengthW == 0` 这条会误杀少数无标题全屏窗口（个别游戏/播放器），
上游同样如此。取舍是：**宁可 Dock 不躲，也不要莫名其妙地躲**。

### 与自动隐藏共用同一套宽限期

智能隐藏与自动隐藏的唯一差别是"什么算靠近"，所以走同一条
`tick_auto_hide` 逻辑，只在入口把 `covered=false` 折算成 `forced near`。
另开一套状态机必然出现两套阈值各自演化。

**关键**：`covered` 参数在 `AutoHide` 模式下必须被**忽略**。
反过来会让两者互相污染成"有时候收有时候不收"，既难复现也难描述。

### 可观测性

`occlusion::covering_window()` 返回盖住者的标题，`MINIDOCK_GEOM=1` 时
打印成 `[hide] target=true ... by "窗口标题"`。实测输出直接点名了
用户正在用的 Edge —— 这类"为什么它躲起来了"的问题，没有点名只能靠猜。

`MINIDOCK_FAKE_COVERED=0/1` 可覆盖判定结果：桌面环境不可控（用户随时
开着窗口），要验证"遮挡消失就展开"这条分支，不能真去最小化别人的窗口。

---

## ADR-028 · 运行状态：精确匹配 exe，不做文件名猜测

**背景**：运行指示点之前是**无条件绘制**的 —— `DockItem` 根本没有
`running` 字段，`paint_running_dot` 对每个图标都画一遍。视觉上表现为
"所有应用都亮着"，用户无法分辨哪个真在运行。

这也是"点击已运行的应用会新开一个实例"的根因：没有运行状态就无从判断
该切还是该开。

**决定**：分三层。

| 层 | 职责 |
|---|---|
| `platform::app_window::running_apps()` | 枚举顶层窗口 → `exe 小写路径 → 窗口` |
| `core::running::running_key(item)` | 某个条目该拿什么键去查表（纯逻辑，可单测） |
| `core::running::refresh_running()` | 刷新条目状态，返回"是否有变化" |

### 匹配规则：只认精确路径，不猜

1. `item.target` 非空 → 用它（`.lnk` 解析出来的真实 exe，最准）
2. `item.path` 以 `.exe` 结尾 → 用它
3. 其余（未解析的 `.lnk`、文件夹、网址、分割线）→ **不参与匹配**

第 3 条是刻意的。退化成"按文件名猜测"会把 `网易云音乐.lnk` 与
`网易云音乐.exe` 硬凑到一起，而它实际指向 `cloudmusic.exe`。
猜测错的结果是**给一个没运行的程序点亮指示点** —— 比"该亮没亮"更糟，
因为它给出了错误信息。

**已知局限**（实测发现）：Win11 的记事本、计算器、画图等已是 Store 应用，
`notepad.exe` 只是应用执行别名，真实进程是
`C:\Program Files\WindowsApps\Microsoft.WindowsNotepad_*\Notepad.exe`。
这类条目匹配不上，指示点不亮、点击会新开（实际由系统转发到已有实例，
用户观感尚可）。要支持得解析 `AppExecutionAlias` 的包信息，本轮不做。

### `refresh_running` 必须返回"是否有变化"

主循环每 1.2 秒扫一次。若无条件返回 true 就会无条件重绘，空闲 CPU 从
0% 直接失守（M0 花了不少力气才压到 0）。这条约束有专门的回归测试
`reports_change_only_when_state_flips`。

### 扫描周期参与等待超时

常显模式下主循环原本 `INFINITE` 阻塞。加了扫描之后必须把 1200 ms
放进超时计算，否则"时间过去"不是事件，`MsgWaitForMultipleObjectsEx`
永远不返回，指示点就再也不更新。

---

## ADR-029 · `.lnk` 解析绕过 windows crate 的 wrapper，直接调 vtable

**背景**：判断"应用是否在运行"需要把 `.lnk` 还原成真实 exe 路径。

**问题**：`windows` 0.62 给 `IShellLinkW::GetPath` / `GetArguments`
生成的 wrapper 签名与 COM vtable **不符**：

| | 签名 |
|---|---|
| vtable | `(this, PWSTR, cch: i32, *mut WIN32_FIND_DATAW, flags: u32)` |
| wrapper | `(&self, flags: u32) -> Result<PWSTR>` |

wrapper 只传了两个参数。照它调用是未定义行为（栈上的 `cch` 与
`WIN32_FIND_DATAW*` 会是随机值），所以不能用。

**决定**：用 `Interface::vtable(&link)` 取出 vtable，按 COM ABI 直接调：

```rust
let vt = Interface::vtable(&link);
(vt.GetPath)(Interface::as_raw(&link), PWSTR(buf.as_mut_ptr()),
             buf.len() as i32, std::ptr::null_mut(), 0);
```

**代价**：这段代码依赖 vtable 布局，升级 `windows` 大版本要重新核对。
用一个 `#[ignore]` 的真实解析测试兜住：

```text
cargo test -p minidock-platform --lib shortcut -- --ignored
```

它会去开始菜单里找一个 `.lnk`，断言解析出的 exe 真实存在。实测输出
`Access.lnk -> C:\Program Files\Microsoft Office\root\Office16\MSACCESS.EXE`。

**COM 初始化**：用 `Once` 保证只初始化一次（STA），不 `CoUninitialize`
——提前反初始化会让之后所有 COM 调用失败。若调用方已是 MTA，
`CoInitializeEx` 返回 `RPC_E_CHANGED_MODE`，此时 IShellLink 仍可用，
所以忽略该错误。

---

## ADR-030 · 四边停靠，以及"配置不生效"的排查顺序

### 一、停靠边不能硬编码

之前窗口定位写死在底部（`x = 居中; y = work.bottom - win_h`），
`Edge` 配其余三边会跑到屏幕外。现在抽成纯函数：

- `layout::window_origin(edge, work, reserved, win_w, win_h)` —— 定位
- `layout::physical_size(edge, lw, lh)` —— 竖排要**转置**尺寸

`physical_size` 单列出来是因为它被"建窗口"和"AppBar 厚度"两处共用，
写错一处就会出现竖排 Dock 只有一条细缝。四个方向都有测试。

### 二、`Reserve` 与隐藏模式冲突时要出声

`Reserve` 只在 `Mode=0`（常显）生效，与上游一致 —— Dock 会滑走的模式下
留一块空白更怪。但用户既然打开了开关，就该知道它没被采纳：
启动日志会打印"`Reserve` 只在常显模式（Mode=0）下生效，当前已忽略"。
**不静默忽略**是这一系列改动的共同原则。

### 三、排查顺序：先证明"输入是对的"

这轮有一次 20 分钟的弯路：`Edge=1` 写进 ini 后 Dock 仍停在底部。
我先怀疑 `Key::Edge` 的映射，再怀疑 UTF-16 解码 —— 都不对。
最后用一个读 ini 的探针五分钟定位：**ini 被整份覆盖回默认值了**
（`raw Edge=Some("0")`、条目变回 4 个默认项），与解析毫无关系。

教训：配置不生效时，第一步该确认"程序读到的到底是什么"，
而不是从解析代码往回猜。探针已固化为：

```text
cargo run -q -p minidock-core --example read_ini -- <ini 路径>
```

它同时打印**解析值**与**原样字符串**，能区分"没读到"（raw=None）
与"读了但认不出"（raw=Some 但解析值是默认）。

### 四、如实记录的验证缺口

`SmartHide` 的 `covered=true` 分支在本机实测过：诊断日志点名了盖住者
（`by "…Microsoft Edge Beta"` / `by "WorkBuddy"`），状态机随之收起。

`covered=false` 的**端到端**场景没拿到 —— 验证期间桌面上始终有最大化窗口。
该分支由 `smart_hide_stays_visible_when_nothing_covers_it`（状态机）、
`overlap_requires_positive_area` 与 `offscreen_target_is_not_covered`
（判定层）覆盖，但单元测试不等于真机验证，不假装验过。

---

## ADR-031 · 补齐缺失的交互：焦点、捕获、强制新实例、弹跳、快捷键、全屏

对照上游 `DockProc` 的 `case` 列表逐条补齐。这一轮全是"用户天天能碰到
但代码里根本没有"的东西，没有架构性改动，但每一处都有具体的坑。

### 一、`WM_MOUSEACTIVATE` → `MA_NOACTIVATE`

Dock 是常驻工具窗。不拦这条消息，点图标时会先把焦点从用户正在用的窗口
抢到 Dock 上，接着启动的应用反而拿不到预期的前台位置 —— 观感是
"点一下整个桌面闪一下"。上游同样返回 `MA_NOACTIVATE`。

### 二、`WM_CAPTURECHANGED`：拖拽必须能被中断

鼠标捕获被抢走（UAC 弹窗、Alt+Tab、别的程序 `SetCapture`）时，
抬起事件永远不会到。不清掉 `press` 状态的话，下一次点击会带着上一次的
"按下"，表现为**点一下启动了上一个图标** —— 很难复现也很难描述。

`cancel_drag()` 清状态且**不**产出点击。

### 三、中键 / Shift+点击 = 强制新实例

上游是 `LaunchItem(i, force)`。`force` 的作用是**跳过"激活已运行窗口"**：
已经开着一个记事本时，Shift+点击本该开出第二个，被"激活"吃掉的话
用户会以为快捷键坏了。

实现上把两种点击收进同一条 `click: Option<(usize, bool)>` 通道，
主循环只处理一次。中键没有"按下-抬起配对"语义，由 `force_click()`
直接产出。

**Shift 必须在按下时读**：抬起那一刻用户可能已经松开。

### 四、弹跳：`Bounce` 配置项此前完全没实现

配置里一直有 `Bounce`，也出现在设置界面上，但 Dock 侧没有任何代码读它
—— 设了等于没设。曲线照抄上游 `BounceOff()`：

```text
x = t / 1100
off = |sin(x·2π)| · (1 − x·0.55) · S · 0.48
```

`|sin|` 给出两次弹跳、`1−0.55x` 线性衰减。`S` 参与计算，所以高 DPI 下
弹得更高（与图标同比，观感才一致）。偏移沿"栏体厚度方向"往外，在逻辑
坐标的 `v` 上减 —— 与上游 `iconBottom - sz - bo` 等价。

实测：中键点击后 170 ms，图标顶端从 y=114 抬到 y=102（12 物理像素）。

### 五、`Ctrl+Alt+D` 强制收起 —— 以及一个把 Dock 弄丢的 bug

`force_hide` 与 `hide_target` 分开存：前者是用户手动的，优先级更高、
不受鼠标位置影响（上游 `ht = gForceHide ? 1 : gHideTgt`）。

**踩到的 bug**：主循环的推进条件一开始写成 `auto_hide() || force_hide`。
强制收起后再按一次恢复，两个标志都变 false，而 `hide_cur` 还停在 1 ——
动画再也不会被推进，**Dock 永久消失，只能重启进程**。
修复是把判断收进 `Dock::needs_hide_tick()`，条件加上 `hide_cur > 0.0`，
并配了一条专门的回归测试。

教训：**"要不要继续动画"的判断应该由持有动画状态的那一方给出**，
散在调用点写 `flag_a || flag_b` 迟早漏掉一个中间态。

### 六、全屏时整体隐藏

前台是"覆盖整个显示器且没有 `WS_CAPTION`"的窗口时 `SW_HIDE`。
`WS_CAPTION` 这一条是关键：普通最大化窗口也覆盖全屏，但不该让 Dock 消失，
否则用户会以为 Dock 崩了。无边框全屏才是沉浸态的信号。

**代价**：为了每秒检测一次，空闲等待从 `INFINITE` 改成了 1000 ms。
实测空闲 10 秒 CPU 增量仍然是 **0.000 秒**（只做两个只读查询、不出帧），
M0 的验收标准没有被破坏。

实测：全屏探针覆盖屏幕期间，Dock 所在像素从 `(70,70,70)` 变成全屏窗口的
`(20,24,32)`；探针关闭后恢复 `(70,70,70)`，日志出现
`[fullscreen] 隐藏 Dock` / `恢复 Dock`。

### 七、`WM_SETTINGCHANGE`：主题切换立即生效

系统深浅色切换会广播 `ImmersiveColorSet`。只在启动时读一次主题的话，
用户切到深色后 Dock 会一直是浅色直到重启 —— 典型的"看着像 bug"。

### 诊断：判定逻辑只能有一份

`is_fullscreen_foreground()` 与诊断用的 `fullscreen_trace()` 如果各写一遍
判定条件，排查时看到的诊断会与真实行为不一致，**比没有诊断更糟**。
现在两者共用 `check_fullscreen()`：一个函数查、一个结构体装结果、
`verdict` 是唯一结论。

---

## ADR-032 · 右键菜单必须有自己的窗口

### 一、为什么不能画在 Dock 窗口里

菜单向上（或向屏幕内侧）弹出到栏体之外，而 Dock 窗口只有栏体那么大。
分层窗口**不会**因为内容画到矩形外就被裁剪 —— 画出去的部分直接不可见。
所以菜单必须是一个独立的顶层窗口。上游 C 版也是独立窗口（`ShowMenu()`）。

### 二、第二个渲染器，但共用 GPU 设备

菜单尺寸随项数变化（1 项 43 px，5 项 200 px 左右），而
`FrameRenderer` 的离屏纹理与 DIB 在构造时就固定了。

三个选择：

1. 每次开菜单重建 `FrameRenderer` —— 不行。里面最贵的是
   `egui::Context::default()` + 装 CJK 字体（解析字体文件）与
   wgpu 管线编译，几十毫秒，菜单会明显卡一下。
2. 固定一个最大尺寸 —— 菜单内容只占上部，下方留透明区。
   透明区鼠标穿透（分层窗口按 alpha 命中），用户点"看不见的空白"会
   穿透到下面的窗口，语义混乱。
3. **加一个 `FrameRenderer::resize()`**：只重建离屏目标与 DIB，
   保留 ctx 与管线。菜单窗口懒建一次，之后每次只 resize。

选了 3。`resize` 在尺寸不变时直接返回。

### 三、模态用 `SetCapture`，不激活窗口

菜单要能被"点外面"关掉。两条路：

- 激活菜单窗口，靠 `WM_ACTIVATE(WA_INACTIVE)` 感知外部点击；
- **不激活**，用 `SetCapture` 把鼠标消息全部劫持到菜单窗口。

选后者。原因：激活会抢焦点，用户关掉菜单后原来在用的窗口已经失焦，
还得再点一次才能继续输入 —— 一个右键菜单不该有这个副作用。
捕获期间点在任何地方，消息都进菜单窗口，坐标落在菜单矩形外即取消。

配套：`WS_EX_NOACTIVATE` + `WM_MOUSEACTIVATE → MA_NOACTIVATE` 双重保证不抢焦点。

**依赖这条链的地方还有一个**：捕获被系统取消时会收到
`WM_CAPTURECHANGED`（例如点击了别的线程的窗口）。所以它也当作"取消"处理
—— 否则菜单会挂在屏幕上等一个永远不来的取消事件。

### 四、只放当前真能执行的命令

第一版菜单只有三项能力：打开 / 在资源管理器中打开（文件夹） /
退出应用（运行中）/ 退出 Dock（空白处）。

**没有**"从 Dock 移除"和"添加分割线"，虽然那是右键菜单最核心的功能。
原因：它们会改变条目数量 → 改变栏体宽度 → 改变窗口尺寸 → 需要重建
离屏目标与 DIB、重新定位窗口、写回 ini。那是一套完整的"配置热更新"，
和菜单本身是两件事。混在一起做，两边都做不干净。

分割线上右键**不弹菜单**（唯一能做的删除也需要热更新）。
弹一个空窗口是对用户的欺骗：看着能点，实际什么也没有。

### 五、验证

| 场景 | 结果 |
|---|---|
| 图标右键 | 弹出 1 项菜单，深色圆角面板、白字、位置在光标上方 65 px 处 |
| 点菜单项 | `cmd=Some(Open)`，计算器进程 `CalculatorApp` 启动 |
| 空白处右键 | 弹出"退出 Dock"，**红色**文字 |
| 点"退出 Dock" | `cmd=Some(ExitDock)`，进程正常退出（AppBar 与单实例锁都释放） |
| 点菜单外 | `cmd=None`，菜单收起，Dock 恢复 |

**一个坑**：自动化测试里"右键后立刻点击外部"偶尔收不到消息 ——
`SetCapture` 还没来得及建立。真实使用间隔在几百毫秒以上，不受影响。
排查时用 `MINIDOCK_MENU_TRACE=1` 打印菜单窗口收到的每条消息。

### 六、已知缺口

- 移除条目 / 添加分割线 / 重命名 / 设置图标：等配置热更新那一轮。
- 菜单没有键盘导航（上下键 + 回车）。现在只有 Esc 取消。
- 应用组、速记、文件夹扇面的子菜单。

---

## ADR-033 · 配置热更新：窗口尺寸是会变的

上一轮做右键菜单时，把「从 Dock 移除」「添加/删除分割线」三项留了下来，
理由是要改条目数量就得改窗口尺寸，而**窗口尺寸从建窗起就是固定的**。
这一轮把这条链打通。

### 一、链路顺序不能乱

```text
改 raw（真源）→ 写回 ini → dock.replace_items() → 下一帧 update() 重算几何
                                                        ↓
                            主循环发现 physical_size 变了 → frame.resize()
                                                        ↓
                                       SetWindowPos（尺寸 + 重新居中）
```

关键是**谁都不直接改窗口**。菜单命令只碰 `raw` 与 `dock.items`，
窗口尺寸由主循环在 `update()` 之后比对 `physical_size_i32()` 得出。
理由：尺寸是几何的**结果**，几何由 `update()` 算。谁想改窗口就得先算几何，
而算几何是 `update()` 的职责 —— 让调用方去算就会有两套几何来源。

### 二、真正危险的是离屏纹理与 DIB

窗口变小但离屏目标还是旧尺寸，`UpdateLayeredWindow` 提交的仍是一块
旧尺寸的位图 —— 表现为"只有左上角一部分被画出来"或"右边留下上一帧的残影"。

所以 `frame.resize()` 必须和 `SetWindowPos` 一起做，且**先** resize
（先改纹理再改窗口，避免中间出现一帧尺寸不匹配）。

`resize()` 是上一轮为右键菜单加的：只重建离屏目标与 DIB，
保留 egui 上下文与 wgpu 管线。

### 三、图标必须按路径迁移

`replace_items` 一开始只是整份替换 `items`，结果每次删一个条目，
**所有图标都退回"首字占位"**再重新加载 —— 用户看到整条 Dock 闪一下。
实测截图确认过这个现象。

修法：`DockItem` 加 `path` 字段，替换时按路径把旧 `IconTex` 搬到新下标。
重复路径（用户放了两份同一个快捷方式）只保住第一份，另一份重新加载
—— 不影响正确性。

### 四、最后一项不能删

`raw.len() <= 1` 时拒绝删除并打印原因。Dock 变成一个空壳后，用户没有
任何可点的东西能把它恢复回来（拖放进来的入口还没做）——那是一个
"自杀式"的操作，不如直接不让做。

### 五、实测

底边停靠、3 个条目、窗口初始 `724×186 @(918,1414)`：

| 操作 | 窗口 | ini |
|---|---|---|
| 移除「计算器」 | `664×186 @(948,1414)` | `Count=2`，计算器消失 |
| 加一条分割线 | `678×186 @(941,1414)` | `Count=3`，`I1=3\|0\|\|\|\|0\|` |
| 删除该分割线 | `664×186 @(948,1414)` | `Count=2` |
| 再移除「记事本」 | `604×186 @(978,1414)` | `Count=1` |
| 移除最后一项 | （不变） | 打印「这是最后一项，不能移除。」 |

每一步的截图都确认了：栏体完整居中、圆角未被裁、没有残影，
图标保持原纹理（不退回首字占位）。

### 六、还没做的

- **设置窗口尚未接入 Dock 进程**（`Overlay` 只在 spike 里用过）。
  所以「改设置 → 即时生效」这条还差最后一环：设置写回 ini 后，
  Dock 需要重新读配置。目前只有条目列表这一条路是热的。
- 拖入新条目（从资源管理器拖文件到 Dock）—— 这会复用同一套热更新，
  但需要 `WM_DROPFILES` / `IDropTarget`。

---

## ADR-034 · 配置格式 v2：按语义分段，键名自己定

「去除对上游的依赖」的第一步。上游耦合里最实在的一条是配置：
76 个键全挤在 `[Settings]` 里、键名是驼峰、`Mode` 与 `BgMode` 看不出谁管什么。

### 一、保留 ini 语法，只换键名与分区

先说为什么**不**换成 TOML/JSON：

`minidock-core` 是**零依赖**且 `unsafe_code = "forbid"` 的。引 serde + toml
会打破这条 —— 而这条是整个分层的基础（core 能在任何环境下单测，
不需要 Windows SDK）。自己写一个 TOML 子集解析器要几百行外加一堆边角
（多行字符串、日期、内联表……），换来的只是语法更"现代"。

真正要脱钩的是**键名兼容**，不是语法。`Ini` 是个通用 key=value 分区格式，
它不属于上游。所以保留它，换掉键与段。

### 二、语义分段

| 段 | 内容 |
|---|---|
| `[meta]` | 版本号 |
| `[appearance]` | 尺寸、不透明度、边框、分隔、指示点 |
| `[appearance.background]` | 填充方式、颜色、渐变 |
| `[appearance.plate]` | 图标底盘 |
| `[appearance.reflect]` | 倒影 |
| `[layout]` | 停靠边、显示器、预留、隐藏方式 |
| `[behavior]` | 缩放、标签、弹跳、时钟、扇面 |
| `[behavior.group]` | 应用组（多数待实现） |
| `[theme]` | 两套主题、底盘圆角、点样式 |
| `[widgets.clock]` | 时钟 |
| `[menu]` | 右键菜单开关 |
| `[advanced]` | 浏览器路径、热键 |

`[Items]` 与 `[Graphics]` 不变。

### 三、两处改名不是洁癖

- `Mode` → **`hide_mode`**：原名 `Mode` 实际是"隐藏方式"（常显 / 自动隐藏 /
  智能隐藏），和 `BgMode` 摆在一起时完全看不出哪个管背景、哪个管行为。
- `BgMode` → **`fill`**：它控制的是背景**填充方式**（纯色 / 渐变 / 磨砂），
  叫 `BgMode` 会被读成"背景开关"。

其余按小写下划线规范化。

### 四、数值键只分区、先不改名

`Key::Numeric` 那 41 个键大多对应**尚未实现**的功能（扇面 `FanArc`、
应用组 `GroupFrost`、磨砂 `GroupBg`）。给还没写过一行实现的功能定名字，
大概率等实现时又要改一次。所以它们按**名字前缀**归到对应段
（`section_of_numeric`），名字暂留上游的。

等做扇面/组的时候连同语义一起定名 —— 那时才知道这个参数到底在调什么。

### 五、迁移是"读到就升级"，不是独立工具

```text
读 → 若没有 [meta] version 且 [Settings] 非空 → 读全量值 → 删旧段 → 按新格式写回
```

用这种方式而不是"跑一个 migrate 命令"的理由：

- 用户不需要知道有这回事，也**不会忘记跑**；
- 不存在"迁移失败但旧文件已被覆盖"的窗口 —— 只有一次写盘；
- `from_ini` 本来就两种格式都认，所以即使写盘失败，本次运行照常。

判定条件特意是"**没有** `[meta] version` **且** `[Settings]` 非空"：
只看 `[Settings]` 会把新格式文件误判 —— `[Items]` 段的段名两代共用。

### 六、核心测试：新旧逐键等价

只测"能读"是不够的：键名映射错一位、分区写错一处，现象都是
"升级后某个设置悄悄变回默认值"，而用户可能几天后才发现。

`legacy_and_new_formats_agree_on_every_key` 的做法：
把**每一个键**都改成非默认值（避免"恰好等于默认值"掩盖错误），
分别用旧格式与新格式写出来、读回来，逐键比对。
`upgrade_legacy_keeps_every_value` 再把升级路径也逐键比一遍，
并断言 `[Items]` 未被触碰。

### 七、实测

写一份上游格式的 ini（`IconSize=64 Mode=2 BgMode=0 Edge=1 Opacity=80 Clock=1`），
启动 Dock：

```text
配置已从上游格式升级到 v2（各设置保持不变）。
GEOM dpi=144 S=64.0 Z=100.0 ... clockW=249     ← 全部取自旧配置
```

落盘内容：`[meta] version=2`、`[appearance] icon_size=64 ...`、
`[Settings]` 已消失、`[Items]` 保留。再次启动不重复迁移（日志 0 次），
度量与第一次完全一致。

### 八、遗留

- `write_ini` 此前**从未被调用**（配置从来没写回过）。现在只有"升级"这一条
  路径会写。设置窗口接进来之后才有连续的写回。
- 段顺序按"首次出现"排列，所以升级后的文件里 `[Items]` 会排在 `[meta]` 前面。
  不影响解析，但不美观 —— 等设置窗口落地时再统一排序。

---

## ADR-035 · 删除 GDI+ 老实现（minidock / gfx / ui）

### 一、ADR-010 当时为什么留

ADR-010 的决定是「`minidock-gfx` / `minidock-ui` 暂不删除，作为行为参照与
fallback」。在 M0 阶段这是对的：wgpu 路线还没证明可行，
GDI+ 那套至少能跑，留个退路。

### 二、现在为什么删

三条理由，按分量排：

1. **它从来不是行为基准。** 真正的基准是上游 C 版源码（`reference/` 下的
   `minidock.c`，本地对照用）。这三个 crate 只是 M0 之前的一次**移植尝试**，
   而且是不完整的 —— 它没有应用组、没有扇面、没有磨砂，配置也只接了
   一部分。拿一个半成品当参照，比没有参照更容易误导。
2. **两份实现已经开始漂移。** `layout.rs` 的注释里记着一例：GDI+ 版算基准
   宽度时用了动画中的 `cur`，导致基准槽位每帧漂移；GPU 版用固定的 `s` 才是
   对的。这类分歧只会越来越多，而"哪份是对的"每次都要重新判断。
3. **它让 `cargo build` 产出两个 Dock。** `MiniDock.exe`（GDI+）与
   `minidock-dock.exe`（wgpu）并存，两个都编、都能启动、都读同一份配置。
   调试时先要确认"我跑的是哪个"，这个成本每次都在付。

### 三、删了什么

| 路径 | 行数 | 内容 |
|---|---|---|
| `crates/minidock` | 83 | 老的 bin 入口（`MiniDock.exe`） |
| `crates/minidock-gfx` | 969 | GDI+ 封装（canvas / objects / 字体） |
| `crates/minidock-ui` | 948 | GDI+ 版 Dock 骨架 |

合计 **2000 行**，占当时总代码（16604 行）的 12%。

workspace 从 7 个 crate 减到 4 个：`core` / `platform` / `settings` / `gpu`。

### 四、影响的可见变化

**可执行文件名从 `MiniDock.exe` 变成 `minidock-dock.exe`**。之前两个 exe
并存，现在只剩后者。如果后续需要沿用 `MiniDock.exe` 这个名字，
应该在 `minidock-gpu` 里给 bin 加一个 alias，而不是把老 crate 捡回来。

### 五、被删代码里有什么值得留意的

删之前扫了一遍，只有一处**思路**值得记下来（代码本身不留）：

`gfx/objects.rs` 里对 GDI+ 的 `GraphicsPath` 做圆角矩形的拼法，
和现在 `dock.rs` 里 `rounded_gradient()` 自建三角扇网格是两套完全不同的
做法（前者调库、后者自己出顶点）。现在这套已经跑通且经过像素验证，
没有理由回头。

其余都是薄封装，没有独有逻辑。

### 六、没有做的事

- **没有留 Cargo feature 开关**。留一个 `--features legacy-gdi` 看似稳妥，
  实际会让"删干净"这件事永远差一步：CI 要不要编它？测试要不要跑它？
  两份实现共存的成本会以另一种形式回来。
- **没有先把它们移到 `archive/` 分支**。git 历史里都有，需要时
  `git checkout <sha> -- crates/minidock-ui` 就能拿回来。


## ADR-036 · 设置窗口接进 Dock 进程

### 一、为什么是进程内

设置界面（`minidock-settings`）早就写好了 9 页，但它一直是"有个 UI，没人用"：
原来的 `overlay.rs` 只是个 spike（配 `bin/spike.rs` 的独立 exe），既读不了真正的
配置，也改不动 Dock。要接到一起只有两条路：

- **独立进程**：改完要走 IPC，或者让用户重启 Dock。对一个"拖滑块看栏体变化"的
  面板来说，这两种都不可用。
- **进程内第二窗口**：`settings_ui` 直接改 Dock 用的那份 `Config`，下一帧
  `update()` 就按新值重算几何。上游 C 版本来也是进程内对话框（`DialogBox`），
  口径一致。

选第二条。代价是 Dock 进程多一个分层窗口和一份 UI 状态；收益是零 IPC、零复制、
零"保存后重启"。

### 二、Win32 那几处不能抄菜单（ADR-032）的地方

菜单与设置窗口同为"第二个分层窗口"，但有三处必须相反：

| 事项 | 菜单 | 设置窗口 |
|---|---|---|
| 模态 | 模态（`SetCapture` + 自己的消息循环） | **非模态**，要能看着栏体调参数 |
| 激活 | `WS_EX_NOACTIVATE` + `MA_NOACTIVATE` | 要焦点（颜色选择器、未来的文本输入） |
| 层级 | 用完即隐藏，不需要任务栏 | `WS_EX_APPWINDOW` 进任务栏 + `WS_EX_TOPMOST` |

置顶那条是被环境教出来的：别的窗口一盖上来，用户既点不到它、我们又没有托盘图标
能把它唤回来。`WS_EX_TOPMOST` 让它跟 Dock 保持同一层级。

### 三、`GWLP_USERDATA` 与地址稳定（踩过的坑）

窗口过程要把消息转给 Rust 侧的状态，常规做法是 `SetWindowLongPtrW(hwnd,
GWLP_USERDATA, ptr)`。第一版写成：

```rust
let mut sw = SettingsWindow::new(..)?;   // 栈上的值
sw.attach();                              // 把这时的地址写进 USERDATA
*slot = Some(sw);                         // 移动！之前写的地址失效
```

于是窗口过程拿到的是**已释放的栈帧地址**，下一次消息就是野指针访问 —— 表现为
"点一下设置窗口进程就崩"，且没有任何 Rust panic 输出（`safe` 代码的边界没被触发）。

修法是让 `new` 直接返回 `Pin<Box<Self>>`：装箱之后无论如何搬动 Box，堆上的地址
不变，`attach()` 在构造函数内的最终地址上调用一次即可。同时给了 `Drop`，把
`USERDATA` 清零。

> **通用规则**：把 Rust 值的地址交给操作系统之前，先证明这个地址不会再变。

### 四、写盘：防抖 + 重新读一遍

- **400 ms 防抖**。拖一次滑块会产生几十次 `changed`，而 ini 是全量 UTF-16 回写，
  逐次落盘等于每帧一次文件重写。停手即存，拖着不存。
- **写之前重新 `Ini::load`**。`write_ini` 只管 `[meta]` 之后的配置段，`[Items]`
  不归它管；直接覆盖会把条目全部丢掉。
- **`(changed, remount)` 分开返回**。停靠边 / 预留 / 隐藏模式变了要重新注册
  AppBar 并重算窗口位置，那是主循环的责任 —— 塞进 `tick_settings` 会把它拖成
  十几个参数的函数。AppBar 的注册与窗口位置本来就不属于设置窗口。

### 五、删掉了 spike

`overlay.rs`（约 600 行）与 `bin/spike.rs` 是设置窗口的原型实现，正式版上线后
它们只会各自漂移、与正式实现不一致。与 ADR-035 同一思路删除，workspace 少一条维护面。

### 六、还剩什么

- 一部分命令（应用组、托盘等）在 `run_settings_cmd` 的最后一支给出明确提示，
  而不是静默什么都不做 —— 用户应当知道那是"还没接"，不是"点了没反应"。
- `PORTING.md` 里对应的那条待办已勾掉。

---

## ADR-037 · 把阻塞调用挪出 UI 线程，但只在该用多核的地方用多核

**背景**：UI 线程同时负责消息泵和出帧，任何一次同步系统调用都直接变成掉帧。
审计下来有五条路径压在它身上：

| 路径 | 阻塞点 | 频率 |
|---|---|---|
| 取图标 | `SHGetFileInfoW` + `IImageList::GetIcon` + `GetDIBits` | 启动 |
| 运行状态 | `EnumWindows` + 每个窗口 `OpenProcess` | 1.2 s |
| 遮挡检测 | `EnumWindows` + 每个窗口 8 次查询 | 100 ms（智能隐藏） |
| 写 ini | 读文件 + UTF-16 全量回写 | 拖拽每一步 / 改设置 |
| 启动应用 | `ShellExecuteW` | 点击 |

**决定**：`minidock-gpu::worker` 里四个**专用线程**，各自一条 `mpsc` 通道：

- `IconLoader` —— shell/COM 取图标，**串行**（`IImageList` 是共享的 shell 对象，
  并发访问没有保证），像素处理（缩放判定 + 圆角遮罩）在批内用 rayon 并行；
- `Scanner` —— 运行状态与遮挡合并到一条线程，两者都要 `EnumWindows`，
  分开跑会抢同一个系统锁；
- `DiskWriter` —— 串行写 ini，天然保序；
- `Opener` —— `ShellExecuteW` 与"激活已有窗口"。

**为什么是专用线程而不是线程池**：四类工作性质不同（串行 / 周期 / 保序 /
一次性）。塞进一个池子只会让"保存配置"排在"取 20 个图标"后面。

**结果怎么回到 UI 线程**：每条线程 `PostMessage(WM_APP_WAKE)` 敲一下，
`MsgWaitForMultipleObjectsEx` 立刻返回；`pump` 看到这条消息**只打断等待、
不强制出帧**，否则每次扫描结果都会多渲染一帧，"空闲 0 帧"就没了。
没有走"每 250 ms 轮询一次"—— 那会让空闲时线程不再真睡。

**多核用在哪**：只用在图标像素处理。这是全项目唯一 CPU 密集的活
（每张 256×256 要走 6 万多个像素）。其余四条都是 IO 等待，
并行只会增加线程数、不缩短墙钟时间 —— 所以没有给它们上 rayon，
`crossbeam-channel` / `arc-swap` 两个"计划中的"依赖也一并删掉了。

**代价**：
- 结果比原来晚一帧到几帧。运行指示点最多晚 1.2 s（本来就是 1.2 s 一轮），
  图标从"启动时全部就位"变成"窗口先出来、图标陆续补上"。
- 拖拽重排的写盘从"每次必写"变成"停手 400 ms 后写"。进程被强杀会丢最后一次
  顺序调整 —— 与配置写入同一套取舍。
- 退出时不 join 工作线程：shell 调用可能卡在慢速路径（网络盘、杀软），
  等它会把整个进程拖住。线程只碰自己的数据，detach 是安全的。

**顺带修的缺陷**：`platform::shortcut::ensure_com` 原来用进程级 `static Once`
初始化 COM —— 只有第一个调用的线程被初始化，其余线程上的 `CoCreateInstance`
会以 `CO_E_NOTINITIALIZED` 失败。单线程时看不出来，一引入工作线程就变成
"图标偶尔取不到"：没有报错、没有崩溃，只是结果缺失。现在改成 `thread_local!`。

**重新考虑**：若将来真的要 60 fps 连续动画，瓶颈会回到每帧的同步回读
（`device.poll(wait_indefinitely)`）。届时的解法是双缓冲 + 延迟一帧呈现，
代价是交互延迟 +16 ms —— 对一个"只在悬停时动"的 Dock 不划算，所以现在不做。


## ADR-038 · 吸收上游 v1.6.0：23 个新键与小组件条带

上游从 v1.4.0（6875 行）走到了 v1.6.0（9681 行）。本轮处理其中**唯一会动
已对拍通过的几何**的那部分，以及配置层必须跟进的键表变化。

### 一、配置层：76 → 99 键

上游新增 23 个键，全部追加在 `g_tab[]` 末尾。逐键加进
`core::config::NUMERIC_KEYS` 表（加一个上游键的代价仍然是一行），
默认值与值域照抄上游 `DefaultCfg()`。

**这不只是"跟上上游"。** `Config::upgrade_legacy` 的做法是"读 → 删掉
`[Settings]` 整段 → 按新格式重写"。凡是 `Key::ALL` 里没有的键，删段时
就一起没了 —— 上游 v1.6.0 新增的 23 个键正好撞在这个缺口上。症状是
"用户从 1.4 升到 1.6，配置被清空一半"。

新增 `upgrade_legacy_keeps_upstream_v160_keys` 守着这条。

新键按语义分了段（前缀规则已经不够用，新键跨了四个语义段）：

| 段 | 键 |
|---|---|
| `widgets` | `WSide` `WOrder` `WideMask` `NotifyW` `CtrlCenter` `CcEnergy` `CcHotspot` `ResMon` `ToolW` `ModeW` |
| `widgets.launchpad` | `Launchpad` `LpCols` `LpRows` |
| `advanced` | `PinMenu` `TrayHide` `UpdChk` `UpdLast` |
| `behavior` | `MultiMode` |
| `behavior.group` | `FanBgL` `FanBgD` `FanFgL` `FanFgD` `FanDrag` |

### 二、布局层：单个时钟卡片 → 小组件条带

上游 v1.4.0 的模型是"栏体右边挂一个时钟卡片"：

```c
g_ce = cfg.clock ? g_clockW + M.g + 6 : 0;
```

v1.6.0 把它泛化成"栏体外侧的一排卡片"：

```c
enum { W_CLOCK, W_NOTIFY, W_CC, W_RES, W_TOOL, W_MODE, W_N };
int ord[6] = { cfg.wOrder ? W_NOTIFY : W_CLOCK, cfg.wOrder ? W_CLOCK : W_NOTIFY,
                W_CC, W_RES, W_TOOL, W_MODE };
g_ce = 0;
for (int k = 0; k < g_wn; k++) g_ce += (float)g_wWid[g_wk[k]] + M.g + 6;
```

Rust 侧对应新增 `core::layout::WidgetKind` 与 `WidgetStrip`：

- `WidgetKind::index()` **必须与上游枚举同序** —— `WideMask` 是位掩码，
  按序号取位（上游 `cfg.wideMask & (1 << W_RES)`），换序会让老配置的
  双倍宽设置错位。
- `WidgetStrip::build()` 复刻上游的 `ord[]` 顺序、`WOrder` 交换、
  宽度公式（`barH × 0.92 / 0.72 / 1.16 / 2.2 / 1.6`）与 `g_ce` 汇总。
- `WidgetStrip::left_of()` 复刻上游的 `g_wU[]` 定位：右侧时从栏体右缘
  往外排，左侧（`WSide=1`）时从栏体左缘往左排。

`Metrics.clock_extra` 随之改成 `widget_extra`（= 上游 `g_ce`）。

### 三、关键约束：v1.4.0 的几何一个像素都不能动

v1.4.0 的几何已经逐项对拍通过（`lw` / `lh` / `slot_l` 全部相等），
换模型不能把它弄坏。三条保证：

1. **只开时钟且 `WSide=0` 时，`widget_extra` 恒等于
   `clock_w + gap + 6`** —— 与 v1.4.0 的 `g_ce` 逐位相等。
   测试 `widget_strip_matches_v140_when_only_clock` 守着。
2. **实测 A/B 对比**：同一份对拍 ini，重构前后各跑一次 Dock，
   `GEOM` 输出**逐字节相同**（时钟开与关两种配置各测一次）。
3. **时钟卡片的绘制位置改走条带定位**
   （`WidgetStrip::left_of`），不再硬编码"栏体右缘 + gap + 6" ——
   否则 `WSide=1` 只是配置项，不起作用。

### 四、未实现的小组件不进条带

上游 v1.6.0 默认把控制中心 / 资源监视器 / 计时器 / 专注模式**全部打开**
（`CtrlCenter=1` `ResMon=1` `ToolW=1` `ModeW=1`）。但这五个我们一个都
没移植。

若照上游把它们算进条带，栏体上会留出四段**画不出东西的空白** ——
比不做还糟。所以 `WidgetKind::is_implemented()` 目前只对时钟返回真，
其余五个即使配置为开也不占位。等真正移植时把 `is_implemented` 打开，
条带、定位、宽度公式全都现成。

> 这是"读得进、存得住、但不占位"的策略：配置不丢，功能没做之前
> 不假装做了。

### 五、本轮没做的

- **五个新小组件本身**（通知 / 控制中心 / 资源监视器 / 计时器 / 专注模式）：
  都要新窗口或新面板，属于 M3。
- **起动台**（`Launchpad`）：独立全屏窗口 + 分页 + 拖放，依赖最多。
- **`[Tool]` / `[M]` 两个新配置段**：我们的 `Ini` 原样保留不认识的段，
  读 v1.6.0 配置不会丢数据，只是读不出来 —— 在实现对应功能之前，
  "保留但不解析"就是正确策略。


## ADR-039 · 资源监视器小组件：条带的第一个新成员

ADR-038 把布局泛化成了"小组件条带"，但那时条带里只有时钟 —— 模型有了，
成员还没进来。本轮把**资源监视器**做掉，验证条带模型确实能承载新卡片。

对应上游 v1.6.0 的 `DrawRes()` / `ShowResMon()`（minidock.c:6521、6580）。

### 一、数据采集（`platform::sysinfo`）

上游的三个数据源，全部只读：

| 数据 | API | 说明 |
|---|---|---|
| CPU | `GetSystemTimes` | idle / kernel / user 三个累计值，取两次采样差值 |
| 内存 | `GlobalMemoryStatusEx` | `dwMemoryLoad` 给百分比，`ullTotalPhys` / `ullAvailPhys` 给字节数 |
| 磁盘 | `GetDiskFreeSpaceExW(L"C:\\")` | 剩余 / 总量 |

**CPU 占用率必须留状态**：`GetSystemTimes` 给的是累计值，单次调用算不出占用率。
上游用三个 `static` 变量硬存；这里做成显式的 `CpuSampler`（带 `prev`），
避免全局可变状态。第一次调用返回 0（没有基线），与上游一致。

### 二、采样频率：按秒，不按帧

`GetSystemTimes` 与 `GlobalMemoryStatusEx` 都不是便宜调用，而 CPU 占用率
本身也只有秒级意义。上游是"每次重绘才重新采样"，但它空闲时几乎不出帧；
我们这边空闲时同样不出帧，所以改成**定时采样 + 数值变了才出帧**
（CPU 常年是 0，每秒重画一次纯属浪费）。

### 三、卡片的文字要能装下（踩到的坑）

上游画文字用 `GdipDrawString(g, t, -1, f, &rc, ...)` 传一个矩形，
**GDI+ 会自动把字缩小到装得下**。egui 不会 —— 文字直接溢出卡片。

两处修正：

1. **字号按栏高走**，不按图标尺寸走。卡片宽度是 `barH × 1.16`，
   而栏高会随条目数量被压缩（128 个条目时 `barH` 从 76 掉到 37），
   固定字号在窄卡片里会被裁成 "PU%"。
2. **量了宽度再缩字号**：`ctx.fonts_mut(|f| f.layout_no_wrap(...))`
   量出真实字宽，超出卡片就按比例缩。
   （`row_height` 只给行高，量不出字宽 —— 试过，不行。）

### 四、详情面板：自己画，不靠布局

面板是进程内第二个分层窗口（与设置窗口同一套做法，ADR-036）。

面板内容用 `painter` **显式定位**绘制，不用 egui 的布局：
上游也是显式定位（`DrawTextBox` 传矩形）。

> 用布局做左右对齐时右边的数值会**整个消失** ——
> `with_layout(right_to_left)` 在 `horizontal` 里拿不到剩余宽度。
> 实测过一次：三条进度条的标签都在，数值一个没画出来。

另外面板必须传带 `screen_rect` 的输入（`Input::take`），
否则 egui 的绘制区只有一小块：背景被裁、右边的文字落在裁剪区外。
第一版传的是 `RawInput::default()`，症状是"面板半透明 + 数值消失"。

### 五、点击命中要单独一条路径

小组件卡片**不在条目槽位里**（它在栏体外侧），所以：

- `Dock::widget_at(u, v)` 单独判卡片（对应上游 `WidgetAt()`）；
- `WM_LBUTTONUP` 里**先判卡片**，命中就记卡片点击，不当条目点击；
- 卡片点击走 `widget_click` 标志，由主循环消费（与设置窗口同款做法）。

### 六、诊断：让程序报屏幕坐标

自动化脚本要点击卡片，就得知道它在哪。栏体坐标是**窗口内**的，
脚本自己推算过一次 —— 忘了加窗口偏移，点在了空气里。

现在 `MINIDOCK_GEOM=1` 会多打一行：

```text
GEOM widget=Resource screen=(1415,1486) size=(90,78)
```

直接是屏幕坐标，脚本照着点。

### 七、这一轮顺手修掉的两个"自己造成的坑"

1. **往运行目录拷了上游 harness 的压力测试配置**（128 个重复条目），
   截图里那一屏重复图标就是它。已删除。
   → 教训：几何对拍要用 `git stash` 前后对比，**不要**把测试配置拷进
   程序实际会读的目录。
2. **写盘把用户配置里的 `ResMon` 写成了 0**（那时该键还没实现，
   落到默认值）。已改回 1。

> 这两条都不是产品缺陷，是我做验证时污染了现场。
> 但"写盘会覆盖用户的真实配置"这件事本身值得警惕：
> 设置窗口的每一次改动都会落盘，而落盘写的是**整个配置**。


## ADR-040 · 计时器 / 待办小组件

上游 v1.6.0 的第二个小组件（`ToolU` / `ToolDraw` / `ToolUp`，
minidock.c:6322-6510）。它比资源监视器多一层：**要交互**。

### 一、纯逻辑进 `core`，时间基准由调用方给

`core::tool` 里没有 `Instant::now()` —— 所有与时间有关的函数都收一个
`now_ms` 参数。好处是所有行为都能用固定数字写单测，不必 sleep 等真实时间：

```rust
state.start(0);
assert_eq!(state.display_ms(61_000), 240_000);   // 5 分钟减 61 秒
```

这条约束与 `core::layout` 的纯函数风格一致，也让"倒计时归零"这种
边界情况能被稳定复现。

### 二、修掉上游一个会累积数据的 bug

上游存盘格式是 `{done}|{text}`，但读的时候：

```c
if (v[0] == L'1') { d = 1; memmove(v, v + 2, ...); }   // 只有 '1' 才跳前缀
```

`done=0` 时前缀**没被剥掉**。于是"存 → 读 → 存"每轮都多叠一层：

```
0|买菜  →  0|0|买菜  →  0|0|0|买菜  →  …
```

而上游每次打勾都会存一次盘，所以这个 bug 在正常使用中会稳定恶化。

Rust 侧**只剥一层** `d|` 前缀，不循环剥。理由：

- 循环剥会误伤合法文本 —— 用户写 `1|foo`，存盘后是 `0|1|foo`，
  循环下去变成 `foo`，把内容吃掉了；
- 只剥一层已经**根治累积**：被上游写坏的文件残留一层 `0|`，
  但从此不再恶化。

两条都有回归测试（`todo_parse_strips_zero_prefix`、
`todo_parse_does_not_eat_user_text`）。

### 三、比上游多做的两件事

1. **时长可调**。上游把倒计时总时长写死 5 分钟，界面上改不了 ——
   等于这个小组件只有一种时长可用。加了「− 1 分钟 / + 1 分钟」两个按钮，
   范围 1..99 分钟。
2. **面板高度按待办条数算**。上游固定 `Sc(470)`，那条高是按 16 条待办
   估的；只有两三条时下面是一大片空白。现在 `330 + n × 25`，
   待办增减时重排窗口（复用 `FrameRenderer::resize`，不重建管线）。

### 四、面板要交互，所以命中区自己算

与资源监视器（ADR-039）不同，这个面板有按钮和列表。做法照上游
`ToolMove` / `ToolUp`：绘制时把每块可点区域的矩形存进 `Hits`，
鼠标事件里逐个数。

不用 egui 的控件，理由在 ADR-039 已经记过：`with_layout(right_to_left)`
在 `horizontal` 里拿不到剩余宽度，右边的东西会消失。面板类 UI
一律自己算矩形。

### 五、三个渲染坑（都实测过）

1. **`Color32::from_rgba_unmultiplied` 的参数是 `(r, g, b, a)`**。
   按 `(a, r, g, b)` 写会得到一个纯青色实心块 —— 四个按钮全变亮青色。
2. **`✓` / `✕` 不在打包字体里**，直接画字符会得到豆腐块。
   改成线段绘制（`check_mark` / `cross_mark`）。
3. **分层窗口不响应 `WM_NCHITTEST`**，手型光标要自己在 `WM_SETCURSOR` 里设。

### 六、输入：`WM_CHAR` 的边界

添加待办走 `WM_KEYDOWN` + `WM_CHAR`：

- 退格、回车、Esc 三个特殊键在 `WM_KEYDOWN` 里处理；
- 其余字符靠 `WM_CHAR`（已经是上屏后的字符）。

**中文输入法还没接**：IME 组合过程中的候选窗口不归我们管，
`WM_CHAR` 只在选词上屏后才来。所以现在能输入英文与粘贴的中文，
拼音直接打不出来。这是 `PORTING.md` 里「IME 兜底方案验证」那条待办，
不是这一轮能解决的。

### 七、落盘

`[Tool]` 与配置/条目**分开防抖**（120 ms vs 400 ms）：

- 勾一条待办是"一次一下"的动作，等 400 ms 会觉得没生效；
- 但秒表跑着时每一帧剩余量都不同，不能真的每次改动都写盘。

`WriteJob` 因此多了一个 `tool` 字段，带着提交那一刻的时间戳 ——
不能在写盘线程里重新取时间，否则秒表数字会有可见跳变。


## ADR-041 · 日历 / 农历：纯算法 + 一个弹层

上游 v1.6.0 的日历是 392×556 的弹层，点时钟卡片弹出：年/月翻页、周一起始的
6×7 网格、每格下方一行副标签（节日 / 节气 / 农历日）、底部四行详情（公历、
农历、干支历、下一个节气）。功能本身不复杂，但它是这个项目里**第一个可以
穷举验证**的功能，值得单独记一笔。

### 一、把算法和窗口彻底分开

`core::calendar` 里没有一个 Win32 调用。于是"农历算得对不对"不必启动程序
看截图，而是：

- **逐日往返**：1901-01-01 到 2099-12-31 共 72 684 天，每天做
  「公历 → 农历 → 公历」，必须回到原处；
- **逐日连续**：后一天的农历日要么是 +1，要么是下月初一，且上月必须是
  29 或 30 天；
- **锚点**：2020–2026 七年的春节、三个闰月（2017 闰六月、2020 闰四月、
  2023 闰二月）对公开可查的日期。

前两条是自己跟自己较劲（能抓住"月长算错"这类局部错误），第三条才是跟外部
世界对齐。三条一起才能说"这块是对的"。

### 二、表是生成出来的，不是抄的

201 个十六进制数手抄必然错位，而且错了**不会报错** —— 只会让某几年的农历
静默错一两天。所以 `core/calendar/calendar_tables.rs` 由
`scratch/gen_calendar_tables.py` 直接从上游 `minidock.c` 提取，脚本里对每张
表都断言了项数（201 / 24 / 30 / 12 / 10 / 12 / 12 / 7）。文件头的注释写明
"不要手改"。

### 三、`(0x8000..=0x8)` 是空区间（踩坑）

`LYearDays` 要数 12 个普通月里哪些是大月：

```c
for (unsigned i = 0x8000; i > 0x8; i >>= 1) s += (g_lunarInfo[y-1900] & i) ? 1 : 0;
```

照抄成 Rust 写成 `for bit in (0x8000_u32..=0x8) { … }` —— 起始大于终止，
**这个范围是空的**，循环一次都不跑。结果每年都少算 0~12 天，年长度恒为
348 + 闰月，整个农历往后错位两年多（2020 春节算成了 2022-02-20）。

正确写法是按位移数遍历：

```rust
for shift in 0..12 {
    if info & (0x8000 >> shift) != 0 { sum += 1; }
}
```

C 的 `for` 循环改成 Rust 的 range 时，**边界方向要先在纸上走一遍**。

### 四、节气用平气，不换 VSOP87

上游的 `TermDate` 是平气公式（回归年当常数），与本世纪真实节气最多差一天
（实测春分在 3 月 19~22 日之间漂）。照抄而不是升级，理由是：我们的验收
基准是**与上游对得上**，不是与紫金山天文台对得上；真要精确到分钟得引
VSOP87，为一个 Dock 的日历背那套表不划算。

### 五、弹层的 dismiss 方式改了

上游是"鼠标移出矩形就关"（`GuardStart(RectInside, …)`）。那对鼠标经过太
敏感 —— 想看一眼下个月，鼠标一动弹层就没了。这里改成 `SetCapture` +
"点到自己外面才关"：

- `SetCapture` 让外面那一下点击先落到我们这里，判完再关；
- 代价是弹层开着时 Dock 收不到鼠标消息（图标悬停会冻住）。这与菜单一致，
  可以接受；
- 同一个卡片点第二下也是关，因为那一下同样被算作"点在外面"。

**Esc 关不掉**：窗口是 `WS_EX_NOACTIVATE`，永远拿不到键盘焦点，
`WM_KEYDOWN` 根本不会到。第一版写了这个分支，是死代码，删了。

### 六、摆位照抄 `PopRun`，注意那 8 像素边距

窗口四周有 8 像素透明边距（画圆角与阴影用）。上游定位时让这层边距**压住
卡片边缘 4 像素**，看起来才是"贴着"而不是"隔着一条缝"。所以断言要比
"内容区"（窗口坐标 + `PM`）而不是窗口坐标 —— 第一版比窗口坐标，测试挂在
`56 >= 60` 上。

### 七、还剩什么

- 只能看 1901–2099（上游同样限制，农历表只覆盖 1900–2100）。
- 没有农历节日之外的节日（清明/中秋之外的公历节日只有 6 个，照抄上游）。
- 右键时钟卡片在上游是"小组件菜单"（换边、隐藏），这里还没做。


## ADR-042 · 控制中心：真音量与"拉系统面板"

上游 v1.6.0 的控制中心是 360 宽的弹层：3 列磁贴 + 一条主音量滑块 + 电量文案
（minidock.c:6237-6310、6716-6735）。功能不多，但每一格背后都是一次系统调用，
所以把它拆成三层：

- `core::control`：磁贴顺序、行数、哪些磁贴有开/关语义 —— 纯数据，可单测；
- `platform::audio` / `platform::sysact`：真调用；
- `gpu::cc_win`：只画和判命中。

### 一、音量走 Core Audio，不模拟按键

上游用的是 `IAudioEndpointVolume`（`CcVolInit` / `CcVolGet` / `CcVolumeSet`），
我们也一样。**不用 `VK_VOLUME_UP/DOWN`**：那种做法会改"应用程序音量"分组、
会在系统 OSD 上闪一下，而且拿不到真实数值 —— 滑块就没法反映当前音量。

三个注意点：

- **COM 必须在调用线程初始化**。`platform::audio::Audio::new` 里顺手
  `CoInitializeEx`，忽略 `RPC_E_CHANGED_MODE`（宿主可能已经初始化过）。
- **接口指针不能跨线程**，所以 `Audio` 不是 `Send`/`Sync`，由编译器守住。
- **没有音频设备时要降级**：`Audio::new()` 返回 `None`，滑块画成禁用态
  （灰轨 + 圆钮贴左），不崩、不假装能调。

`windows` crate 里 `IMMDevice::Activate` 需要 `Win32_System_Com_StructuredStorage`
与 `Win32_System_Variant` 两个 feature 同时开着（`PROPVARIANT` 参数），
漏一个就"方法不存在"。

### 二、Wi‑Fi / 蓝牙 / 热点：不硬啃私有接口

这三格没有稳定的公开开关接口（WinRT 的 `Radio` 要 UWP 上下文，`netsh` 太慢
而且改不了蓝牙）。上游的做法是**模拟 Win+A 拉出系统快速设置**
（`keybd_event`，minidock.c:6290-6295），我们照做：把选择权交回给系统，
用户点到的仍然是那个熟悉的开关。

代价是"点了会弹系统面板"，不是就地切换。这是**有意的取舍**：就地切换要
赌未文档化接口，赌输了是"点了没反应"，更糟。

### 三、深色模式：注册表 + 广播

`HKCU\...\Themes\Personalize` 的两个 DWORD（`AppsUseLightTheme` /
`SystemUsesLightTheme`），改完广播 `WM_SETTINGCHANGE("ImmersiveColorSet")` ——
系统自己那套开关走的就是这条路。注意语义是**反的**：`= 0` 才是深色。

我们的配色不缓存（`theme::is_dark()` 每次读注册表），所以改完只要让主循环
再出一帧。第一版还写了个 `reload_theme()`，写完发现没东西可 reload。

### 四、修掉上游一个布局 bug

上游把滑块的 y 坐标写成"2 行磁贴之后"（`2 * (th + Sc(10))`），但磁贴数是
6~8 个 —— 节能与热点默认都开着，就是 8 个、3 行，滑块直接压在第三行上。
这里按实际行数算（`control::rows`）。

### 五、已知限制

- 亮度没做。上游有 DDC/CI 与 `\\.\LCD` 两条路（`CcBright` / `CcLcdBright`），
  但 v1.6.0 的绘制里没用到，属于"有代码没入口"，跳过。
- 有线网卡检测没做（要 `GetAdaptersInfo`，得再加一个 `Win32_NetworkManagement_IpHelper`
  feature），所以永远显示「无线网络」。对台式机用户是个可见的偏差。


## ADR-043 · 起动台：把 700 行 C 拆成"模型 + 窗口"

上游的起动台是 700 行 C：扫描开始菜单、分页、搜索、拖拽重排、拖到图标上
合并成文件夹、文件夹弹层、右键菜单、拖文件进来。这里面**唯一难验证的是
"拖到图标上会不会正确合并"** —— 而它恰好是最容易写错的部分。

所以按老规矩分层：

- `core::launchpad`：应用 / 文件夹 / 格子顺序 / 隐藏列表 + 拖放规则 +
  `launchpad.dat` 读写。零 Win32，15 条单测覆盖合并、解散、重排、隐藏、
  搜索、分页；
- `platform::launchpad`：扫开始菜单（两个 `Programs` 目录，深度 4，
  过滤卸载器/readme）；
- `gpu::launchpad_win`：全屏窗口，只管画与收输入。

### 一、`drop_at` 为什么要一个 `on_icon` 参数

上游 `LpDrop` 有两种结果：落在**图标本体**上 → 合并成文件夹；落在同一格的
**空白处** → 重排。区分依据是"离格心多近"（`LpCellAt` 里的 `center`）。

像素不该进模型层，所以 `drop_at(src, target, on_icon)` 把判断留在 UI。
第一版没这个参数，结果"把 A 拖到 B 的位置想换序"永远变成"合并成文件夹"，
测试里当场就露馅了。

### 二、文件夹的生命周期

三条规则（上游 `LpFolderCleanup`）：成员为 0 → 整格消失；成员为 1 → 散开成
单独一格；否则保持。散开这一步很容易漏 —— 漏了会出现"点开一个只有一个
图标的文件夹"，看起来像 bug。

### 三、图标：复用 Dock 那条取图标线程的**实现**，不复用那条线程

`worker::IconLoader` 的构造只需要一个 `Waker`（往某个 HWND 发 `WM_APP_WAKE`），
所以起动台自己起一条，只给"这一页看得见的"排任务：一百个应用一次性取图标
会让打开卡住几秒。到货后 `PostMessage` 催重绘，UI 线程只做 `load_texture`。

图标没到之前画**字母占位**（首字母 + 按名字哈希的色相），不做转圈动画 ——
一个 Dock 不值得为图标 loading 再引一套动画。

### 四、和上游不一样的三处

1. **添加方式**：上游是右上角「＋」弹菜单（添加文件 / 文件夹 / 网址）。
   这里改成**把文件拖进窗口**（`WM_DROPFILES`），更符合"起动台"的直觉，
   也省掉三个系统对话框与一个自绘输入框。点击「＋」只会打印一行提示。
2. **右键菜单**：上游有重命名 / 移出 / 添加到 Dock 等一整套。这里只保留
   "文件夹里右键 = 移出文件夹"，其余不做 —— 自绘菜单要再来一个窗口类，
   投入与收益不成比例。**重命名、添加到 Dock 目前没有入口**，这是明确的缺口。
3. **搜索框自己收键盘**：上游塞了个 `EDIT` 子控件。这里直接用 `WM_CHAR`，
   于是窗口必须能拿焦点（`WS_EX_NOACTIVATE` 不行，见 ADR-041 的反面）。
   **中文输入法仍然没有**：`WM_CHAR` 收得到已上屏的字符，拼音打不出来。

### 五、两个互相掩盖的错误（这一轮最费时间的地方）

第一次跑起动台，进程在渲染第一帧时 panic：

```
Dropped TexturesDelta with 1 unapplied deltas.
```

`epaint` 的 `TexturesDelta::drop` 在调试构建里断言"没人认领的纹理增量"。
追下去发现真正的错误是 **`UpdateLayeredWindow` 失败** —— 窗口少写了
`WS_EX_LAYERED`，而 `FrameRenderer::render` 在呈现失败时用 `?` 提前返回，
**跳过了 `textures_delta.clear()`**，于是那个 delta 在函数退出时被断言抓住，
把真正的错误盖掉了。

两处都改了：

- `launchpad_win` 的窗口样式补 `WS_EX_LAYERED`（呈现走 `UpdateLayeredWindow`，
  没有这个样式必然失败）；
- `FrameRenderer::render` 把"读回 + 呈现"包进闭包，先无条件清理 delta 再
  `?` 传播错误。**任何窗口呈现失败时都不该看到"纹理增量没人处理"这种
  指向错误方向的报错。**

### 六、入口

右键菜单新增「起动台」。上游是把一个 `mdock:launchpad` 的伪条目塞进 Dock
（`SysAdd(SYS_LAUNCH)`），我们没有那套系统条目机制，所以走菜单。


## ADR-044 · 中文输入法、起动台的菜单与重命名

两件遗留的事一起做：**输入法**（卡着所有"能输入中文"的功能）与**起动台的
右键菜单 / 重命名**（ADR-043 里明确留的缺口）。

### 一、为什么 `WM_CHAR` 不够

`WM_CHAR` 只在**上屏那一刻**给一个字符。拼音输入的过程中，按键被输入法
自己吃掉，窗口什么都收不到 —— 实测在起动台里敲 "we"，系统弹出一个孤零零
的候选窗，搜索框毫无反应。用户看到的是"程序不响应"。

正确做法是接 `WM_IME_*` 四条消息（`platform` 不用动，全在 `gpu::ime`）：

| 消息 | 做什么 |
|---|---|
| `WM_IME_STARTCOMPOSITION` | 清空组合串；把候选窗挪到光标处 |
| `WM_IME_COMPOSITION` | `GCS_COMPSTR` = 还在打的拼音；`GCS_RESULTSTR` = 已上屏的字 |
| `WM_IME_ENDCOMPOSITION` | 清空组合串 |
| `WM_IME_SETCONTEXT` | **必须交回 `DefWindowProc`**，否则系统不画候选窗；失焦时顺手清组合串 |

`ImmGetCompositionStringW` 的协议是"先问字节数、再取内容"，而且返回的是
**字节数**（UTF-16 所以是字符数 × 2），长度为 0 或负数都表示没有。

### 二、不自绘候选列表

候选框仍然是系统那一套：用户熟悉、词库一致、翻页键不用重学。我们只负责
"文字进来"和"位置正确"。自绘候选要接 `ITfTextInputProcessor`（TSF），
那是另一个量级的工程，换不来任何用户可感知的好处。

### 三、组合中的拼音要看得见

搜索框与待办输入框都把 `已上屏 + 组合中` 拼起来画，组合部分**加下划线**。
这不是装饰：用户需要看到"字还在输入法里"，否则会以为没打进去而重复输入。

### 四、起动台的菜单：画在同一个窗口里

上游起动台的右键菜单是另开一个弹出窗口。这里改成**在起动台窗口内部画**
（和文件夹弹层同一套）：

- 命中区、输入法、重绘全部复用已有的那套，不需要第二个窗口类与消息循环；
- 菜单项：应用 4 项（打开 / 重命名 / 添加到 Dock / 移除），文件夹 3 项
  （打开 / 重命名 / 解散）；
- 重命名框也在窗口内，键盘与输入法都走同一条路径。

「添加到 Dock」起动台自己做不到 —— 它不拥有 Dock 的配置。做法是抛一个
`LaunchpadCmd::AddToDock` 给主循环执行（`take_cmd`）。这条边界是刻意的：
窗口只管交互，配置改动集中在 `bin/dock.rs`。

### 五、找到并修掉的一个真 bug

加了菜单之后右键毫无反应。加一行 trace 打出来：

```
[lp] RBUTTONUP (182,224) hit=None hits=1
```

`hits=1` —— 整个网格只有右上角「＋」进了命中区。**格子本体从来没被 push
进 `hits`**（当时只 push 了 `icon_rects`，而 `hit_at` 只查 `hits`）。也就是
说在此之前，**点图标根本不会启动应用**，只是没人试过 —— 上一轮验收只点了
"空白处关闭"。

教训：命中区有两个列表（格子 / 图标），漏掉一个的症状是"点了没反应"而不是
报错。现在两处都 push，并在 `draw` 的注释里写明各自用途。

### 六、测试脚本点到了别人的窗口上

验证菜单时脚本盲点坐标，结果打在了 **Windows 设置**上（那个窗口是上一轮
控制中心测试点「设置」磁贴时打开的）。为此做了两件事：

- 所有点击前先 `WindowFromPoint` 核对窗口类名，不对就报错退出；
- 起动台改成 `WS_EX_TOPMOST`：全屏覆盖层本来就该压住别的窗口，
  否则从 Dock 打开时经常被前台窗口盖住，看起来像没反应。

（顺带确认：过程中没有改动任何系统设置项。）

## ADR-045 · 光标为什么一直是"忙"

### 一、现象

鼠标一进任何自绘 UI（Dock 栏体、右键菜单、起动台、资源面板、日历……）
光标就变成沙漏/转圈，而且因为没人把它设回去，**只要还停在窗口上就一直转**。
一两秒之内没人会觉得是 bug，停上十秒就会以为程序死了。

### 二、两条独立成因，都实测过

先把"到底是哪一步把光标设成忙"问清楚：注册一个最小窗口，把鼠标移进去，
再读 `GetCursorInfo`。同一个窗口类，只改 `WNDCLASSW::hCursor`：

| 类光标 | 进入窗口后 |
|---|---|
| `NULL` | `IDC_WAIT` |
| `IDC_ARROW` | `IDC_ARROW` |

**Windows 对"没有类光标、又不回 `WM_SETCURSOR`"的窗口不是退回箭头，而是
退回忙光标。** 我们所有的窗口类都是 `..Default::default()`，也就是 `NULL`；
Dock 主窗口、右键菜单、起动台、资源面板连 `WM_SETCURSOR` 都没写。

第二条更阴：`SetCapture` 期间系统**根本不发 `WM_SETCURSOR`**。同样的窗口
（类光标 `IDC_ARROW`）加一句 `SetCapture`：

| 是否持有捕获 | 收到 WM_SETCURSOR | 光标 |
|---|---|---|
| 否 | 1 次 | `IDC_ARROW` |
| 是 | **0 次** | `IDC_WAIT` |

于是"弹出即 `SetCapture`"的东西——模态右键菜单、日历弹层、控制中心、
按住图标拖动——光标**从头到尾都是忙的**。这也解释了为什么这个 bug 感觉
"哪儿都有"：我们几乎每个自绘窗口都会抓住鼠标。

### 三、修法

新增 `gpu::cursor`，一处维护映射：

- `class_cursor()` —— 注册窗口类时填 `hCursor`（管"刚进入窗口"那一瞬）；
- `apply(icon)` / `set_hand_if(bool)` —— 窗口过程里设（管之后每一次）；
- `client_pos(hwnd)` —— `WM_SETCURSOR` 的 `lparam` 不是鼠标坐标（低字是
  命中测试码），要自己取一次；也不要用上一次 `WM_MOUSEMOVE` 存的位置，
  系统是先发 `SETCURSOR` 再发 `MOUSEMOVE`，用存的会慢一格。

然后三层都补：

1. 七个窗口类全部给 `hCursor`；
2. 五个窗口过程补 `WM_SETCURSOR`（Dock、菜单、起动台、资源面板原先没有）；
3. **持有捕获的窗口在 `WM_MOUSEMOVE` 里也设一次**（Dock、菜单、日历、
   控制中心）——这一条是第二条成因的唯一解，只靠 `WM_SETCURSOR` 修不好。

顺手把设置窗口的一个小毛病改了：它在 `WM_SETCURSOR` 里一律设 `Default`
（箭头），于是文本框上永远见不到 I 型光标。现在记住上一帧 egui 想要的形状。

### 四、排查时踩的坑（值得记）

前两次"最小复现"都得出"类光标没用"的错误结论，原因是**窗口在 A 线程创建、
在 B 线程泵消息**——窗口从不响应用户输入，系统当然给忙光标。改成
"建窗 + 消息循环在同一个线程"之后，才看到干净的对照表。

另一处：`wintypes.LPARAM` 在 64 位 Python 上仍是 32 位，导致窗口过程每次
抛 `OverflowError`，窗口又变成"不响应"。**测量工具本身也要先被验证。**

### 五、验证

`scratch/check_cursor.py` 把上面七处窗口逐个打开、把鼠标移进去、读
`GetCursorInfo`：

```
Dock: HAND      右键菜单: HAND     起动台: HAND
资源面板: ARROW  计时器面板: ARROW  日历: HAND
控制中心: HAND
```

另有一条"横扫栏体"的检查确认手型/箭头是**跟着命中区变**的（图标上手型、
缝隙里箭头），不是恒定的一个值。

## ADR-046 · 吸收 v1.6.5 的 19 个键 + 启动宽限期

### 一、为什么先做键

上游 1.6.5 的 `g_tab[]` 从 99 增到 118。我们的 `upgrade_legacy` 是
"读 → 删掉 `[Settings]` 整段 → 按 `Key::ALL` 重写"，表里没有的键在删段时
一起消失。ADR-038 为 1.6.0 的 23 个键修过一次，这是**同一个洞的第二次复发**：
用户从 1.6.5 升级，状态栏与三处毛玻璃的配置会静默归零。

所以这一轮先补表，不碰任何渲染：功能可以后做，配置丢了找不回来。

### 二、`HPad` 的位置：两处不一样

上游把 `HPad` 插在 `Margin` 之后（`g_tab` 第 4 位），其余 18 个追加在末尾。
我们有两套顺序，处理方式不同：

- **`Key::ALL`**（决定写回 ini 的行序）—— 照上游插在第 4 位。后面所有键
  的 ini 行序整体后移一位；测试里的下标抽查已同步。
- **`NUMERIC_KEYS`**（决定 `Key::Numeric(i)` 的下标）—— **追加在末尾**。
  它是私有顺序，与上游无关；插在中间会让 64 个具名常量全部错位，
  而错位是编译期发现不了的（`Key::Numeric(4)` 依然合法，只是指向了别的键）。
  `numeric_constants_match_table_rows` 那条测试是唯一的护栏。

一句话：**行序跟上游，下标只跟自己**。

### 三、两个"默认是关"的键

`MenuFrost` 与 `SepScale` 在上游 `DefaultCfg()` 里**没有赋值**，而 `Cfg cfg`
是静态零初始化 —— 所以默认是 0（关）。发布说明的措辞（"可在设置里关闭"）
暗示默认开，代码不是。我们照代码：默认 0，并在默认值测试里写明原因。

### 四、启动宽限期（1.5 s）

上游 1.6.5 在 `FsWatch()` 里加了 `GetTickCount() - g_launchTick < 1500`
直接返回。我们原来没有：`step_fullscreen` 每秒判定一次，而新启动的应用
常常**先以最大化/全屏姿态出现**（浏览器、游戏启动器），于是点一下图标
Dock 就闪没了、等窗口缩回正常尺寸再弹回来。

实现放在 `platform::occlusion`（与全屏判定同处），做成纯函数：

```rust
pub fn in_launch_grace(last_launch_ms: Option<u64>, now_ms: u64) -> bool
```

`now < last`（时钟异常）时按"在宽限期内"处理 —— 宁可晚 1.5 秒再藏，
也不要闪。两个启动入口（左键点击、右键菜单"打开"）都打时间戳；
1.5 s 是上游的值，改它没有依据。

### 五、验证

- 19 个键：`upgrade_legacy_keeps_upstream_v165_keys` 从旧格式读一遍，
  逐个断言值没变；`key_list_matches_upstream_order` 钉死 118 个键的行序。
- 宽限期：`launch_grace_covers_the_edges` 覆盖 0 / 1499 / 1500 / 时钟倒退。
- 门禁：fmt、clippy、289 条测试。

## ADR-047 · 合并胶囊 / HPad / 托盘小组件

三件都是 v1.6.5 的增量，一起做是因为它们都落在条带模型里。

### 一、HPad：一次**有意的**几何变化

上游把横向内边距从 `pad + 2` 改成独立键 `HPad`（默认 `Sc(12)`）。
1.6.0 的 `M.padX` 恒为 10（pad 8 + 2），1.6.5 默认 12 —— **栏体两侧各宽
2 px，里面的图标整体右移 2 px**。

这不是我们擅自改的，而是跟上游走。代价要写清楚：

> 我们的"与 v1.4.0 逐像素一致"基线**到此为止**。对拍基准要从 1.4.0/1.6.0
> 换成 1.6.5，否则每一条几何断言都会差 2~4 px。

`Metrics.pad_x` 的注释与 `bar_height_matches_c_formula` 里那条
`assert_ne!(pad_x, pad + 2)` 就是留给后来人的路标。

### 二、合并胶囊（`WMerge`，默认开）

只改**绘制**，不改布局：上游 `RenderFrame` 画的是"首尾成员的外接框"，
成员之间的 `gap + 6` 仍然留在胶囊内部。所以：

- `WidgetStrip::left_of` 一个字节没动；
- 每张卡片画底盘的动作改走 `paint_card_bg`，合并时直接 return；
- 胶囊本身在 `paint` 里**先于所有卡片**画一次，覆盖 `[first.left, last.right]`。

如果在布局层做合并（把间隔压成 0），位置会和上游对不上，而且
`widget_at` 的命中区也要跟着改 —— 没有必要。

### 三、托盘小组件（`W_TRAY`）

- 条带第 7 个成员，宽度 `barH × 0.72`，位置永远在最外侧（`WOrder` 只换
  前两位）；
- 图形照 `DrawTray()`：两段线组成的 chevron；
- 点击 → `sysact::open_tray()`。

**没有抄上游的跨进程点击**。`TrayOpen()` 前半段是 `SendMessage` 到 Explorer
的 `ToolbarWindow32`，取最后一个按钮的矩形再 `mouse_event`。它要求两个进程
同位数、工具栏结构与系统版本一致，坐标算错就是"点到了别处"，而收益只是
省一次 Enter。我们只保留后半段：`Win+B` 把焦点移到通知区域，120 ms 后
`Enter` 打开溢出面板。

### 四、测试要跟着改的地方

`TrayW` 默认 1，条带默认多一个成员。所有"数成员个数"的测试要么显式
`TRAY_W = 0`（保持聚焦），要么把期望值 +1。新加的
`tray_widget_is_last_and_fixed_width` 钉住宽度与位置。

### 五、验证

- 截图两种模式各一张：合并时是一整块胶囊（时钟 / CPU / 计时器 / 托盘
  连在一起），关掉 `WMerge` 是四块独立卡片；
- 点击托盘卡片，`MINIDOCK_GEOM=1` 打出 `[widget] 打开系统托盘`，进程存活。

---

## ADR-048 · 环境烟雾：把 Taskbar Smoke 的流体模拟搬进 Dock

**背景**：栏体是一块带圆角的深色胶囊，边缘是硬的（只有 1.2 px 描边）。
想让它在桌面上"浮"起来，就得有光晕；而任何静态阴影都盖不住"栏体宽度随
条目数变化"这件事 —— 阴影形状得每帧重算。参考实现是 Windhawk 的
*Taskbar Smoke*（`taskbar-smoke-fork.cpp`，Rubicus）：给 Windows 11 任务栏
胶囊画一圈真的在流动的烟。

**决定**：把它的三块核心搬进 `minidock-core::smoke`，渲染接到 egui：

| 原版 | 这里 |
|---|---|
| `Rauschen()` / `Fbm()` | `Noise`：三层可平铺的域扭曲 FBM |
| `FluidSchritt()` | `Fluid::step()`：Stam 稳定流体 |
| `FluidMalen()` | `Fluid::raster()`：网格 → 预乘 RGBA |

**为什么值得抄，而不是画个渐变**：烟不是"贴图平移"。密度场被速度场输运、
速度场由浮力与噪声湍流驱动、涡量约束让小漩涡不肯散开、光标把烟推开而不是
擦掉。这几条叠加才有"活的"观感；预渲染动画一循环就假。

**与原版的三处偏离**（都是有理由的，不是偷懒）：

1. **每帧只画一条栏体，不是每个胶囊一条。** Dock 本来就只有一个胶囊。
2. **不做「经典纹理烟」风格**（原版 `style=texture`）。那一档靠 D2D 的
   *带纹理画刷描边* 叠二十圈倒角矩形；egui 没有带纹理的 stroke，硬做只能
   预渲染几十张圆角环。保留默认的流体风格（原版也是默认这一档）。
3. **发射区改成"贴轮廓的窄带"**（原版是整个胶囊内部）。原版的任务栏胶囊
   只有 40 px 高，整块内部当发射区没问题；Dock 栏体高 100 px（72 图标 +
   内边距），照抄会让烟在内部就被浮力顶到上沿、被边界困住，只看得见沿顶边
   的一条细线。改成只从轮廓附近 3 格（15 px）内发射之后，外面那圈才是完整
   的烟 —— 反正内部被栏体挡住了。掩码只有一份（`Fluid::edge`），源与光栅化
   共用，免得两处各调各的。

**烟画在栏体下面，不是上面。** 原版把烟画在任务栏**之上**，因为它的胶囊
（图标组）与任务栏是两块不同的元素。这里栏体就是胶囊，画在上面会把图标
染成紫色。代价是只看得见溢出的那圈光晕 —— 这恰好是想要的。

**性能**：噪声表 40 ms（一次性，关着就不建）；模拟 0.6 ms/帧、光栅化
1.3 ms/帧（156×24 网格、390×60 位图，release）。按 30 fps 出帧，约 6% 单核；
**不启用时零成本**（`step_smoke` 直接返回 false，等待超时仍是 1 s）。
限速 30 fps 不只是省 CPU：`FrameRenderer` 每帧要把整块画面从 GPU 同步读回，
60 fps 等于把这条回读翻倍。

**代价 / 已知限制**：

- 只在**底部停靠**生效（见 CONFIG.md 第三节第 3 条）。
- 栏体很窄（只有一两个条目）时，烟雾表现为"一圈不均匀的光环"而不是
  "一片飘动的烟" —— 原版的观感依赖长条胶囊，短胶囊上纹理的大尺度结构
  藏不住。条目多了自然就对了。
- 顶层窗口仍是 `WS_EX_TOPMOST` 的分层窗口，烟的模糊靠网格双线性放大
  （2 px/格 → 5 px/格），不做高斯。

**何时重新考虑**：如果以后接了桌面捕获做磨砂（M3），可以顺带拿到桌面颜色，
那时候"烟随桌面明暗自动选 tone"才有意义；现在 `SmokeTone` 只能手选。

---

## ADR-049 · 设置界面改成「左导航 + 右侧卡片」，一页一个文件

**背景**：原来的设置界面是一列纵向排下来的 9 页，全部写在 `pages.rs` 一个
文件里（653 行）。问题不在长，而在**改一页要读全部**：加一个设置项得先在
`BuildRows` 里找到正确的位置，再确认它的 `Key`、取值域、夹紧规则和显示
文本是否一致；而"某一页长什么样"这件事，在文件里没有任何边界。

**决定**：

1. **布局换成左侧导航 + 右侧多卡片**。这是 Windows 11 设置、macOS 系统设置
   的通行形态，用户不用滚一屏去找一页。
2. **一页一个文件**（`pages/general.rs` … `pages/add.rs`），共享控件收进
   `pages/widgets.rs`。加一页 = 加一个文件 + `Page` 枚举加一项。
3. **视觉基准是 HTML 雏形**（`docs/settings-prototype.html`），页面、卡片、
   行与 Rust 端一一对应。改设计先改雏形，再改 Rust。

**为什么不用 `SidePanel`**：设置窗口外面套着 `settings_win::shell` 画的标题
栏，而 `SidePanel` 按**根 Ui** 的可用区域布局 —— 在子 Ui 里开面板会跟标题
抢地方。改成手工切两块 `Rect`（`ui.new_child(max_rect)`），想放哪放哪。

**踩到的坑**：切矩形时起点必须用 `available_rect_before_wrap()`，
`max_rect()` 不随已用内容收缩 —— 用它会让两列从窗口顶端算起，直接盖住标题。
这条现在有测试守着（`有标题栏时两列不会盖住标题`）。

**颜色为什么硬编码**：卡片与侧栏的三块表面色取自雏形的
`--sidebar` / `--card` / `--line`。用 `visuals.panel_fill` 的话，浅色主题下
它和窗口底色几乎同值，卡片会"消失"（第一版渲染出来就是这样）。只覆盖
表面色，控件与文字仍走 egui 的 `visuals`，所以换主题时文字对比度不会失控。

**验收方式**：没有显示器也要能看。`FrameRenderer` 拆出 `render_rgba`
（画完不呈现），配一个 `examples/settings_shot.rs` 把任意一页渲染成 BMP，
走的是与真实窗口**完全相同**的渲染路径。另外三条 kittest 断言守住布局契约
（导航在左、整行可点、说明在标题下方）。

**代价**：

- 卡片宽度由 `card()` 强制撑满 —— 否则一行 `label` 的卡片会缩成一小块，
  和同列其它卡片对不齐。
- 派生开关（磨砂 = `bg_mode == Frost`、"扫描运行中应用"同时改两个键）需要
  `toggle_derived`，不能直接用 `toggle`。这类"设置项与配置键不是一对一"的
  映射，是重构里最容易错位的地方，所以每个都配了测试。

**何时重新考虑**：如果以后要支持用户自定义分组或搜索设置项，`Page::ALL`
这个静态表就不够了，得引入一份带索引的元数据（那时雏形也要跟着加搜索框）。

---

## ADR-049 · 点击图标没反应：捕获、前台锁与"再点一次收起"

**背景**：用户报"点图标没反应"。查代码发现三层独立的问题叠在一起，
每一层单独看都像"实现好了"。

### 一、`ReleaseCapture` 把点击自己吃掉了（从 M1 就在的 bug）

`WM_LBUTTONUP` 原来的顺序是：

```rust
if GetCapture() == hwnd { ReleaseCapture(); }   // ①
...
release_at(u, v);                                // ②
```

`ReleaseCapture()` 会**同步**给自己的窗口发一条 `WM_CAPTURECHANGED`
（这是 Win32 的既定行为，不是异步的），而那条分支调 `cancel_drag()`
把 `press` 清成 `None`。等 ② 跑到时，"按在哪个图标上"已经没人记得，
`release_at` 只返回 `None` —— **每一次点击都被吞掉**。

表现因此非常"干净"：没有弹跳、没有日志、没有任何副作用，看起来像
点击根本没到达窗口。

修法两条一起做：

1. **先结算再交还捕获**：`release_at` 必须在 `ReleaseCapture` 之前；
2. **`cancel_drag` 不再清 `click`**：真正丢失捕获时 `click` 本来就是
   `None`（它只由 `release_at` 写、主循环立刻取走），清它反而会让
   "自己交还捕获"这条正常路径误伤。

`click_survives_release_capture` / `lost_capture_drops_the_click`
两条测试把这两个方向都钉住了。

### 二、前台锁：`SetForegroundWindow` 会被静默拒绝

Dock 窗口是 `WS_EX_NOACTIVATE`，点它不会让 Dock 拿到前台权，于是
`SetForegroundWindow` 经常什么也不做（任务栏按钮闪一下）。加了三级阶梯
（`force_foreground`）：直接置前 → `AttachThreadInput` 挂到前台线程 →
ALT 键连击。每级之后都**复核** `GetForegroundWindow`。

复核是必要的，但也带来第二个坑：`SetForegroundWindow` 的实际切换不是
同一时刻完成的，立刻复核会误判成失败，于是白跑后面两级。现在的作法是
在 `in_front` 里认"同进程"与"未被遮挡"，而不是靠一次即时比对。

### 三、"再点一次收起"依赖的前台判定在真实桌面上不可靠

原判据只有 `GetForegroundWindow() == hwnd`。这条在下面几种情况下必然
为假，于是永远走"显示"分支 —— 而窗口本来就在眼前，用户看到的正是
"点了没反应"：

- 前台是桌面（`Progman`，无标题）；
- 前台是提权窗口，标题被 UIPI 挡住（`GetWindowTextW` 返回空）；
- 应用有多个窗口，真正在前台的不是枚举到的那个（浏览器、Electron）。

现在 `in_front` 三条取或：句柄相等 → 同进程 → **自己没被遮挡**
（`WindowFromPoint(中心点)` 的根窗口是自己）。

第三条同时把语义补完整了：窗口可见且没被遮挡时，"带到前台"是空操作，
唯一有意义的动作就是收起；被别的窗口盖住时才走"显示"。

**代价**：并排摆放、都没被遮挡的两个窗口，点后面那个的图标会把它收起
而不是提到前台。这个取舍是有意的 —— 它可见，用户不缺"看到"它。

### 四、顺带补上的"获取应用程序"（`ShowRunning`）

`ShowRunning` 一直是**未实现**的键（PORTING.md M2 那条）。现在补上：

- `core::running::temp_keys`：纯逻辑，只追加不重排（`EnumWindows` 的
  Z 序每轮都变，照它排会让图标互相穿插）；
- 临时条目挂在 `dock.items` 的**最后一段**，`item_at(i)` 统一做下标映射，
  点击/菜单/退出都走它；
- 临时条目不给"从 Dock 移除""加分割线"（它们不在配置里，按下标改
  `raw` 会改错条目）；
- 上限 16 个：栏体会按条目数等比缩小，几十个会压成一条细线。

**同时把扫描改成按需**（`ScanInput::want_running`）：指示点与"正在运行"
都关掉时不再每 1.2 秒枚举一遍全部顶层窗口。

---

## ADR-050 · 面板「点外面收起」：不能用鼠标捕获，只能轮询

**需求**：点小组件面板（日历 / 控制中心 / 资源监视器 / 计时器）以外的任意
位置就收起面板，并且做成可关的设置项。

### 一、先把走过的弯路写下来：`SetCapture` 在这里是死的

第一版实现是"面板一显示就 `SetCapture`，收回来的点击落在客户区外就关"。
代码看着没问题，日志里 `grab` 也确实打出来了，**但真机上点桌面毫无反应**。

根因不是代码，是 Win32 的一条老规矩：**只有前台窗口能抓鼠标**。Dock 是
`WS_EX_NOACTIVATE`，点它不会拿到前台；面板是它的子窗口，同样从不在前台。
于是捕获"设置成功、收不到消息"：

| 面板是否前台 | 点面板外面时面板收到 `WM_LBUTTONDOWN` |
|---|---|
| 是 | 收到（把面板 `SetForegroundWindow` 之后能复现关闭） |
| 否 | **收不到**（默认状态；点别的应用、桌面都收不到） |

实测方式很重要：`GetCapture()` 是**按线程**返回的，跨进程查永远是 0，会
让人误判成"根本没抓到"。要看别的线程抓了谁，得用
`GetGUIThreadInfo(tid).hwndCapture`。

顺带确认了另一件事：`WS_EX_NOACTIVATE` 的窗口收不到 `WM_ACTIVATE` /
`WM_KILLFOCUS`，所以"失焦即关"这条路本来也不成立 —— 这三条一起把这个
功能逼到了轮询。

### 二、改成轮询

主循环里加一步 [`step_panel_dismiss`]：面板可见时每
`popup::POLL_MS`（24 ms）读一次 `GetAsyncKeyState` 的左右键，发现"按下"
且落点不在任何面板里就收起。

三个细节都是有原因的：

1. **同时看最低位与当前状态**。最低位是"自上次调用以来按下过"，能抓住
   两次轮询之间的快速点击；只看它会被别的线程调过同一 API 弄丢边沿。
2. **落在 Dock 上放行**。点卡片是"切换"、点图标是"启动"，这两种语义都
   由点击逻辑自己管。轮询要是抢先收掉面板，用户会觉得"点了一下什么都没
   发生"。
3. **点条目时由 `step_click` 主动收起**。放行 Dock 带来的副作用是
   "点应用图标时面板还开着"，所以在点条目的路径上补一次 `hide_panels()`。

**轮询不改变事件流向**：那一下点击照常落到它该去的地方。点别的应用会
顺带收起面板 —— 这比捕获式"点一下只关面板"更符合直觉，也不需要全局钩子
（`WH_MOUSE_LL` 要额外承担超时被系统摘掉的风险）。

### 三、代价

- 面板开着时每秒醒 42 次。每次只读两个按键状态与不超过四个窗口矩形，
  量级是微秒；面板全关时一步都不走。
- 键盘无法收起 `WS_EX_NOACTIVATE` 的日历 / 控制中心（它们拿不到焦点），
  只能再点一次卡片。资源监视器与计时器是普通窗口，补了 Esc。

### 四、顺带修掉的两个"打不开也关不掉"

- **资源监视器**：原来只有 `show()`，没有 `hide()` 的调用点，点开之后
  只能结束进程。现在卡片点第二下收起。
- **计时器 / 待办**：同上，而且 Esc 只在输入待办时可用；现在不在输入态
  也能用 Esc 收起。

---

## ADR-051 · 小组件文字按逻辑点排，不再"固定物理像素"

**现象**：150% 缩放的时间/日期字号明显偏小，比悬停气泡还小一圈；
资源卡片的 `CPU xx%` 反而偏大。两者本该差不多。

### 原因：同一份代码里混了两种单位

`dock.rs` 的绘制坐标是**逻辑点**（`map()` 里乘了 `k = 1/ppp`），
但两处字号各错一半：

| 位置 | 原写法 | 实际渲染出的物理字号 |
|---|---|---|
| 时钟 / 日期 | `14.0 * p.k` | **固定 14 px**，不随 DPI 变 |
| 资源卡片 | `metrics.bar_h * 0.27` | `bar_h` 本身已经是物理像素，当成逻辑点用 → **DPI 乘了两遍** |

于是 150% 缩放下：时钟 = 14 px（= 9.3 逻辑点，比 12 逻辑点的气泡小），
资源卡片 = 27 逻辑点（= 40 px），一大一小全歪在同一个 Dock 上。

### 改法

所有小组件文字都从**卡片的逻辑尺寸**推：

- 时钟：`line_h = (c.height() - 2·PAD_Y) / 行数`，
  时间 `0.70 · line_h`、日期 `0.44 · line_h`；
- 资源：`0.27 · c.height()`；
- 两者都保留"量真实字宽、装不下就缩"的兜底。

`c.height()` 是逻辑点（物理 ÷ `ppp`），所以算出来的字号在 100% / 150% /
200% 下是**同一个物理尺寸**，栏高被条目数压缩时又同比缩小。

**为什么时间比日期大一号（0.70 对 0.44）**：汉字在同样字号下比数字"重"，
两者用同一档字号时日期反而更抢眼。留白也分开：横向 6、纵向 4 ——
日期那行几乎顶满卡片，贴着圆角很难看。

### 副作用

- 旧的像素级"对拍"基线只覆盖栏体几何，文字本来就没进过基线；这次改动
  不涉及任何几何量。
- 只影响小组件卡片上的文字。悬停气泡（12 逻辑点）、空状态提示（13 逻辑点）
  本来就是逻辑点，随 DPI 正常缩放，没动。

---

## ADR-052 · 内置条目的图标用矢量画，起动台补键盘操作

### 一、`mdock:` 条目此前是"半条命"

上游 `LoadItemIcon` 里有一句分支：`mdock:start` 走 `StartIcon(cs)`、
`mdock:launchpad` 走 `LaunchIcon(cs)` —— 这两个条目**没有文件**，图标是
现画的。移植时漏了这条，于是：

- 图标：走 shell 取不到 → 落到"首字占位"，Dock 上立着一块写着「起」的方块；
- 点击：`mdock:launchpad` 被当作普通文件丢给 `ShellExecuteW`，必然失败，
  表现为"点起动台毫无反应"。

### 二、为什么不用 emoji、也不用字体字符

三条路都试过，只有矢量最稳：

| 做法 | 问题 |
|---|---|
| emoji（🚀 / ⊞） | 字体里不一定有；缺字是豆腐块，而且彩色字形跟其余图标完全不是一套语言 |
| 字体字符（如 ❖ ⠿） | 依赖字体覆盖，等宽/比例字体下重心会飘；放大到 100 px 就露馅 |
| **矢量（圆角矩形 + 几何）** | 与尺寸无关、无字体依赖、颜色可控，缩放动画里不会糊 |

形状按上游的比例（四格窗 0.27/0.04，九宫格 0.2/0.075），但**不用渐变**：
项目整体是纯平风格，渐变在这种尺寸上只显得脏。

### 三、起动台补键盘

起动台是个全屏网格，只用鼠标点很别扭。补上：

- 方向键移动选中格，**左右到头翻页**（上下翻页需要额外约定"翻到下一页的
  第几列"，实际用起来更容易迷路，所以只夹在本页内）；
- Home / End 到本页首尾，PageUp / PageDown 翻页，Enter 打开；
- 选中格画一圈 accent 描边；打搜索词时选中会被清掉（过滤结果变了，
  旧下标不再指同一个应用）。

菜单 / 重命名 / 文件夹弹层开着时方向键不归网格 —— 它们各自有自己的键盘语义。

**踩到的坑**：第一版把方向键写在 `match` 里、`_ => return None` 收尾，
结果 Esc 也被这条兜底吞掉（"Esc 关不掉起动台"）。改成 if 链，未命中的键
继续往下走默认分支。

---

## ADR-053 · 起动台分类 / 标签 / A–Z：照搬语义，不照搬关键词

### 数据模型

`launchpad.dat` 从"格子 + 文件夹 + 隐藏"扩了三样，都放在文件末尾：

```text
G\t<组名>            一组标签（1..=8 组）
C\t<应用名>\t<组号>   手工指定的分类
```

老文件没有这两行 → 读出来就是默认三组（办公 / 系统 / 工具），分类全走
启发式。**没有单独的开关文件**：标签是起动台的一部分，跟格子顺序一样属于
"用户自己摆出来的东西"。

分类号从 1 开始，`0` 在界面里表示"全部"。分类与搜索词是"与"的关系：
只按分类看还是只搜，都是同一个过滤视图（只列应用、不列文件夹）。
文件夹没有分类，混进结果里只会让人困惑 —— 原版 `LpFiltered` 也是这个语义。

### 两个当场踩到的坑

**一、默认三组把自己删了。** "文件里有 G 行就以文件为准"写成：

```rust
if lp.tags == DEFAULT_TAGS { lp.tags.clear(); }   // 每次 push 前都比一遍
```

读完前三组时 `tags` 恰好又等于默认值，第四行把三个组一起清空 —— 四组只剩
最后一组。改成一次性的 `tags_seen` 标志位。回归测试 `default_tags_survive_a_roundtrip`
用"原样存回去再读"钉住；早先那条 `tags_and_overrides_roundtrip` 因为改了第三组
名字（开发 / 游戏），恰好绕开了这个状态，所以没抓到。

**二、分类器把 190 个应用全判成"系统"。** 原版 `LpCat` 的关键词里有
`\Windows\`，但原版的 `path` 是 **.lnk 解析出来的目标 exe**，我们存的是
**开始菜单里的 .lnk 路径**：

```text
C:\ProgramData\Microsoft\Windows\Start Menu\Programs\…
```

每一行都含 `\Windows\`。于是"系统"一组 190 个、"工具"一组 0 个。
改成只认 `\Windows\System32\` 与 `\Windows\SysWOW64\`，再加上 `shell:` 前缀
（UWP / 控制面板那一类）。分类结果变成 办公 9 / 系统 0 / 工具 181，
符合预期（这台机器上开始菜单里没有"设置""回收站"之类的系统项）。

**教训**：照搬启发式关键词之前，先确认两边喂给它的**是什么字符串**。

### 交互

- 分类栏贴在搜索框下方：分类是"搜索的另一种过滤方式"，放一起符合直觉。
- 右键「移动到标签组…」是**二级菜单**，不是把 8 个组平铺进主菜单 ——
  主菜单已经有 5 项，再塞 9 项会长得没法看。为了二级菜单，`run_menu_item`
  改成自己负责"下一步显示什么"，点击处理不再无条件关菜单。
- 当前组标「（当前）」而不是 `✓`：字体里没有那个勾，画出来是豆腐块
  （同一个坑 ADR-045 之后又踩了一次）。
- A–Z 排的是**顶层格子**，不是应用表：用户之后手动拖一次就覆盖回去。

### 没做的

- 标签**改名 / 删除**：需要复用重命名输入框给"标签"用（现在只认格子），
  留到下一轮。目前"＋"加出来的组名是占位的「标签N」。
- 过滤视图里的拖拽排序：格子索引与应用索引不是一回事，先不开。


---

## ADR-054 · 起动台的改名框复用搜索框；过滤视图里拖动怎么算落点

### 一、改名框不该是浮层

第一版把"重命名"做成一块 260×30 的浮层，固定画在 `y=96`、水平居中。
问题是标签栏就在 `y=92..120` —— 两个字直接叠在一起，谁也看不清。

改成**搜索框就地接管**：

| 状态 | 搜索框的画法 |
|---|---|
| 平时 | 占位提示 / 已输入的搜索词 |
| 改名 | `名字|` + accent 描边，右侧框外给「回车确认 · Esc 取消」 |

好处不只是不重叠：位置固定（用户视线不用挪）、命中区不变（点别处仍然是
"提交并退出"）、少一段绘制代码。改名时**同时**给目标描一圈（格子上是
格子框、标签上是那个胶囊），否则用户只看到顶上多了个字，不知道在改谁。

名字上限 40 字，14 点排下来能到 400 px 以上，超过 380 的框宽。这里**缩字号
而不是截断**：名字是用户正在核对的东西，少一个字就可能改错。

### 二、过滤视图里拖动的落点

过滤视图的格子里是**应用**，不是顶层格，所以不能直接复用 `drop_at`。
新增 `move_app_before(src_ai, dst_ai)`：把源应用在顶层的位置挪到目标应用
**前面**。

三个决定：

1. **不合并文件夹**。过滤结果里相邻的两个应用，在完整视图里可能隔着十几格；
   就地合成一个文件夹，用户回到完整视图会找不到自己的东西。原版在过滤视图
   里也只重排。
2. **只处理在顶层格子里的应用**。已经在文件夹里的应用挪不动（它没有自己的
   顶层位置），返回 `false`，界面保持不变。
3. **顺序必须跟着顶层走**。原来 `visible_in` 按应用表顺序返回 —— 那是扫开始
   菜单扫出来的顺序，跟界面上看到的不是一回事。更要命的是：拖动改的是顶层
   次序，画出来的还是应用表次序，于是**拖了完全没有反应**。改成按
   `app_slot()`（顶层下标，文件夹内的按夹内序）排序后才对得上。

### 三、这轮踩到的两个"看起来是 bug，其实是测试姿势"

- **改名框"画不出来"**：`self.hits` 是**上一帧**的结果，右键之后隔 0.25 s
  就点菜单项，那一帧还没渲染完，命中区里没有菜单项，点击于是落到菜单下面
  的格子上（`打开` → 启动应用并关窗）。隔 0.6 s 再点就一切正常。
  这不是产品的 bug，但说明**自动化点击必须等一帧**。
- **`✓` 又变成豆腐块**：上一轮改了菜单项文案，但改的是另一处（选择分支），
  `menu_items` 里那行 `format!("✓ {t}")` 留着没动。现在统一成
  「名字（当前）」。

### 四、已知没做的

- `launchpad.dat` 仍然**只在关窗时**写盘。改名 / 删组 / 拖拽之后进程被强杀
  会丢（正常情况下退出即保存）。要改成"改动后 1–2 秒防抖保存"，得先给
  `save()` 加一个时间戳节流，否则拖拽过程中会写几十次。

---

## ADR-055 · 控制中心磁贴顺序：存 id 不存位置

### 一、存什么

`[widgets] Order=5,1,7,3,2,4,6,0` —— 逗号分隔的**磁贴 id**：

| id | 磁贴 | id | 磁贴 |
|---|---|---|---|
| 0 | 无线 / 有线（同一格） | 5 | 设置 |
| 1 | 蓝牙 | 6 | 节能模式 |
| 2 | 深色模式 | 7 | 移动热点 |
| 3 | 定位 | 8 | 有线网卡 |
| 4 | 静音 | | |

数值沿用上游 `[Cc] Order` 的写法，从 C 版迁过来的配置不用翻译。段名换了
（上游 `[Cc]`，我们 `[widgets]`）：Rust 版的 v2 配置本来就按语义分段，
`CcEnergy` / `CcHotspot` 已经在 `[widgets]` 里，顺序跟它们放一起。

**为什么是 id 不是位置**：位置会被"开关一个磁贴"挤乱。用户把设置拖到第一格、
后来关掉节能模式，如果用位置存，重开节能会插到中间去；用 id 存就很自然 ——
顺序是"这些磁贴的相对次序"，跟谁显示谁不显示无关。

### 二、三条回退规则

1. **关掉的磁贴保留位置**。`Order` 里始终写着它，只是排列时跳过；重新打开
   就回到原处。否则关一次再开一次，磁贴就跑到末尾，用户会以为配置丢了。
2. **有线 / 无线共用一个槽位**。`Order` 里记的是"网络那一格"的位置，具体
   显示哪个由网卡决定。不这样的话，插一次网线那一格就会跳到别处。
3. **`Order` 里没提到的追加在末尾**。新版本加磁贴，或用户手改只写了几个 id，
   都靠这条兜住 —— 少一个磁贴比位置不对更难查。

### 三、拖动语义：即时换位，不做拖影

上游 `CcMove` 是"指针每进入一个新格子就交换一次"，所见即所得。这里照抄：

- 3 列 × 3 行的格子只有 107×100，拖影会把底下的格子遮住一半，反而看不清
  会落到哪；
- 即时换位本身就是最好的预览。

阈值 6 逻辑点：低于它算点击（否则每次点磁贴都会因为手抖判成拖动，变成
"点了没反应"）。**换过位就不再触发点击** —— 否则每次重排都会顺带把系统
面板拉起来，正好盖住刚排好的磁贴。

### 四、写盘

拖动结束只把新顺序交给主循环，`cfg.cc_order = …` + `touch_cfg()`，由既有的
防抖写盘线程（400 ms）落到 ini。不在拖动过程中写：一次拖拽会连着换好几次位，
每次都是 UTF-16 全量回写。

**没拖过不写这一行**。空的 `Order=` 会让排查配置的人以为用户调过顺序
（`[Graphics] Adapter` 是反例 —— 那里空串有"自动"的语义，必须写）。

### 五、没做的

- 设置界面里没有"恢复默认顺序"按钮。用户把顺序弄乱之后只能手改 ini
  （`Order` 删掉即可）。要加的话属于设置页的活，跟本轮分开。
- 有线网卡检测仍未接（`tiles_ordered` 的 `wired` 参数恒为 `false`，
  跟改动前一样），属台账第 9 条。

---

## ADR-056 · 分割线拖动缩放：状态机放在 Dock 里，不在窗口过程里

### 一、为什么是 `Dock` 而不是 `wndproc`

`wndproc` 只拿得到 `Dock` 的裸指针（`GWLP_USERDATA`），拿不到 `App`。
而缩放要改 `cfg.icon_size` / `cfg.zoom_size`、要标脏让主循环重建窗口、
要通知写盘。把这些塞进窗口过程就得再开一条回调通道，反而更绕。

所以状态机放 `gpu::dock::Dock`：`press_at` / `drag_to` / `release_at` /
`cancel_drag` 四个已有入口各加一个分支，窗口过程只多了一件事 —— 光标形状。

### 二、判定时机

`press_at` 里**先于**普通拖拽判定：

```text
命中项是分割线 && SepScale != 0  →  进入缩放，press = None，drag = None
否则                             →  原来的拖动重排
```

顺序不能反。否则这一下会被当成"拖动分割线重排"，而分割线本来就不该被
拖走。`release_at` 里缩放态直接返回 `None`：缩放不是点击，否则每次调完
尺寸都会顺带启动一个应用（或弹出系统面板）。

### 三、防闪烁是这轮最要紧的一条

上游注释写得很直白：`rebuild only when the size really changes`。
`drag_to` 里 16 ms 节流之后还比较了一次 `icon != cfg.icon_size`，
尺寸没变就什么都不做 —— 鼠标停着不动时不会反复触发 `update()` 和窗口重建。

主循环那侧本来就有 `step_relayout()`：只在 `physical_size_i32()` 真的变了
时才 `frame.resize()` + `reposition()`。两处合起来，实测一次 40 px 拖动
只有 3 次重建（1495×237 → 1658×265 → 1743×279 → 1825×292），
没有逐帧抖动。

### 四、其它决定

| 决定 | 理由 |
|---|---|
| 缩放比例以**按下那一刻**的 `icon0/zoom0` 为基准 | 用每帧当前值会累积取整误差，来回拖回原位回不到原尺寸 |
| `zoom` 同比缩放后另夹 `32..220` | 上游如此；起点图标很大、放大很小时，按比例的原始值可能低于 32 |
| 截断 `(dy * 0.6) as i32` 而不是四舍五入 | 与上游一致，慢速拖动不会在同一格上反复 ±1 |
| 捕获被抢走时清缩放态、但保留已改尺寸 | `WM_CAPTURECHANGED` 意味着抬起来不了；改动是用户真拖出来的，不该回滚 |

### 五、单位偏差（已知，不修）

`dy` 是客户区**物理像素**（上游也是），`icon_size` 是配置里的逻辑值。
高 DPI 下拖同样的物理距离，尺寸变化会比 96 dpi 时稍慢；而且栏体是贴底
的，长高时窗口上沿上移，同一个屏幕位置对应的 client-y 会变大，对 `dy`
有阻尼。这两条上游都有，保持一致比"修正手感"更重要 —— 这条功能的验收
基线就是 C 版行为。

### 六、测试方法上的坑（又一条）

用合成消息点分割线时，第一次没生效：Dock 窗口在悬停放大时会**改变自身
宽度**，我按"屏幕 x − 窗口 left"算出的客户区坐标，等消息送达时已经偏了
几十像素，按下的其实是隔壁图标。改成先 `ScreenToClient` 把当前屏幕点
换算成客户区坐标再发消息，`icon_size` 就从 72 变成了预期的 96。

另外光标形状本身可以当探针：扫一遍 x，`GetCursorInfo().hCursor ==
LoadCursorW(NULL, IDC_SIZENS)` 的位置就是分割线。比截图猜坐标准。

---

## ADR-057 · 毛玻璃在 CPU 上做，不接 glass-backdrop 的 SPIR-V 管线

**背景**：窗口背景要实现亚克力模糊，仓库 [`glass-backdrop`] 提供了
「GDI 抓屏 → GPU 模糊链 → `fs_glass` 全屏 pass → 预乘 alpha 交换链」的完整
Vulkan 路径。

**决定**：只用它的三个**数据**模块 —— `capture`（GDI 抓屏）、`blur`
（与 WGSL 同一份高斯权重）、`glass_sdf`（圆角 SDF）；模糊在 CPU 上做，
结果当一张普通 egui 纹理画在 UI 最底层。

**理由**：

- 本项目**没有交换链**（ADR-008）：渲染是 `egui → 离屏 sRGB 纹理 →
  CPU 回读 → 预乘 BGRA DIB → UpdateLayeredWindow`。接它的 pass 等于再开
  一条 Vulkan 呈现路径，和现有帧管线并行，收益与复杂度不成比例。
- `fs_glass` 的折射 / 色散 / 2.5D 高光边与项目的纯平基调冲突（用户明确
  排斥渐变、折射、内发光）。
- 权重仍取自上游，`normalized_gaussian_weights` 改一边另一边会红。

**代价**：

- 模糊是 CPU 开销（见 ADR-058 的实测表），且捕获分辨率必须降到 25%
  才能接受 —— 亚克力只看卷积结果，25% 与 100% 肉眼无差。
- 抓屏与模糊都在我们自己的线程上排队，不能像 GPU 那样和渲染重叠。

**何时重新考虑**：如果将来引入 DirectComposition / 交换链呈现，或者
`glass-backdrop` 提供与 `UpdateLayeredWindow` 兼容的 CPU 参考实现。

---

## ADR-058 · 实时背景用独立抓取线程 + `WDA_EXCLUDEFROMCAPTURE`

**背景**：0.9.9 的毛玻璃是「开窗前抓一次」，桌面播视频 / 拖窗口时面板背后
糊的是一张静止截图。要实时，就必须在面板**已经上屏**的情况下继续抓屏 ——
而那时 GDI 抓到的是「桌面 + 面板自己」，直接当背景会得到无限嵌套的镜像。

**决定**：

1. 面板显示前 `SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)`，
   隐藏时恢复 `WDA_NONE`。
2. 面板可见期间起 `minidock-frost` 线程：抓屏 → 与上一帧逐字节比较 →
   变了才模糊 → `ColorImage` 经通道回传 → `WM_APP_WAKE` 叫醒 UI 线程。
3. 节拍 100 ms；连续无变化时间隔翻倍退避到 800 ms，一有变化立刻复位。

**验证记录**（`scratch/wda_test2.c`，分层窗口用 `UpdateLayeredWindow` 铺纯红）：

| 状态 | 抓屏采样点 |
|---|---|
| 绿色窗口在底下，红面板在上 | R255 G0 B0（抓到面板） |
| 面板设 `WDA_EXCLUDEFROMCAPTURE` | R0 G200 B0（抓到底下的绿窗口） |
| 恢复 `WDA_NONE` | R255 G0 B0 |

结论：排除后读到的是**背后**的真实内容，不是黑色，也不是自己。

**代价（重要）**：面板可见且开着毛玻璃时，它在**所有**屏幕捕获里消失 ——
截屏、录屏、共享屏幕、任务栏缩略图都看不到它。不排除自己就只能抓到自己，
实时模糊绕不开。关掉对应的 `LpFrost` / `UiFrost` / `MenuFrost` 即恢复原样
（Live 只在毛玻璃开着时才启动，WDA 也才设）。

**为什么在 `ShowWindow` 之前设**：之后才设的话，中间那一帧抓到的是
「桌面 + 自己」，实时层里会闪一下自己的镜像。窗口尚未上屏时设置，
从第一帧起就是排除状态，没有这个窗口期。

**为什么不做「抓之前临时排除、抓完恢复」**：`SetWindowDisplayAffinity`
到 DWM 生效是异步的，要靠 `DwmFlush` 等一次合成（+33 ms/次），而且这
30% 的时间里用户截屏就会缺一块。比常驻排除更糟。

**为什么抓屏线程自己做模糊**：一次 640×400/r=12 的模糊要 20 ms，放在
UI 线程上就是每 100 ms 卡一下。线程里"抓 + 模糊"串起来做，`recv_timeout`
的等待量按本轮实际耗时扣除，实际节拍才是稳定的 100 ms 而不是「100 ms + 耗时」。

**何时重新考虑**：如果切到 DirectComposition 呈现，可以改用系统亚克力
（`DWMWA_SYSTEMBACKDROP_TYPE`）—— 那是 DWM 自己算的模糊，既不需要抓屏，
也不需要排除自己。前提是分层窗口的 `UpdateLayeredWindow` 呈现方式被替换掉。

---

## ADR-059 · `blur_rgba` 改成平面布局 + 转置写

**背景**：实时背景把模糊从"开窗一次"变成"每秒十次"，40 ms/次（640×400,
r=12）在实时路径上不可接受。原实现是交错 RGBA，内层按步长 4 取同一通道，
外加每个 tap 一次 `clamp`，实测每次乘加要 ~3.7 个周期。

**决定**：拆成三个 `f32` 平面 → 横向卷积（结果**直接按转置写**）→ 再横向
→ 交错回 RGBA。第二次卷积把"列"当行读，两趟内层都是连续切片点积；跨行写
一次卷积才写一个值，不影响。

**结果**：640×400/r=12 从 40 ms 降到 20 ms；384×240 从 14 ms 降到 7 ms。
语义完全不变 —— 新增 `separable_blur_matches_naive_2d` 用朴素二维卷积
（核外积）逐像素对拍，误差 ≤1。

**代价**：多两块 `f32` 平面缓冲（640×400 各 1 MB）。够小，不值得为省内存
把实现改回交错。

**顺带**：圆角遮罩只遍历四个角方块（半径以外覆盖率恒 255），
`corner_only_alpha_matches_full_scan` 保证与整幅扫描逐字节相同。

---

## ADR-060 · 显示/命中判定收敛到一条"停靠带"热区

**背景**：自动隐藏与智能隐藏的"光标是否靠近"判定，原先按状态各写一套
（`cursor_near` 里分成 `hide_target` 两支）。展开态只认图标条
`bar_l..bar_l+bar_w`，收起态认"贴屏幕边 + 落在窗口宽度内"。

**症状**：光标停在时钟 / 控制中心 / 资源卡片上时，展开态判"不靠近" →
450 ms 后收起；收起后光标仍贴着屏幕底边且在窗口宽度内 → 立刻判"靠近" →
弹出。日志实测每 450 ms 翻转一次：

```text
[hide] target=false near=true  cur=0.188
[hide] target=true  near=false cur=0.250
```

**决定**：

1. `Dock::hot_phys()` 给出**一条**矩形：图标条 ∪ 小组件条带，外加两者
   之间 6 px 的视觉缝与朝停靠边的投影留白。窗口物理坐标。
2. `layout::near_dock()` 是纯函数，两态共用同一套几何；差别只有容差 ——
   展开时多 32 px 回滞。施密特触发器，边界上不会自激。
3. 判定需要的输入（光标、热区、工作区、伸展量）全部显式传参，Win32 只剩
   `GetCursorPos` / `GetWindowRect` 两句。这样"光标在时钟上算不算靠近"
   可以写成断言，而不是靠人挪鼠标看它抖不抖。

**代价**：热区比"能看见的栏体"略大（含投影留白与 32 px 回滞），光标在
Dock 外一指宽的地方仍会让它保持展开。这是刻意的：宁可多留一会儿，也别
在用户手底下抽搐。

**同时修掉的两处同源问题**：

- 遮挡判定（智能隐藏）原先只算图标条，盖住时钟的窗口不算"挡住"。
- `busy` 原先只有"拖动中"，面板/设置/起动台开着时 Dock 会自己收起，
  面板凭空悬在半空。

**何时重新考虑**：如果将来支持"多段停靠带"（例如栏体与小组件分居两侧时
留一条真实空隙），热区要跟着变成多矩形并集 —— 那时 `near_dock` 的签名
要改成接受矩形列表。

---

## ADR-061 · 收起之后不再出帧：唤醒 ≠ 要出帧

**背景**：Dock 支持"自动隐藏 / 智能隐藏"，收起后画面上什么都不剩。但主循环
在收起状态下仍在出帧，能耗降不下去。

**根因**：`pump()` 把"取到过消息"等同于"界面变了"：

```rust
if saw { Pumped::Frames } else { Pumped::Idle }
```

而 `PostMessage(WM_APP_WAKE)` 是后台线程的统一唤醒手段。智能隐藏的扫描
线程按 100 ms 的遮挡检测周期回传结果，每次都 `waker.wake()` —— 于是收起
状态下每秒 10 条唤醒，等于每秒 10 帧白画。

`worker.rs` 的注释写得很清楚："主循环看到它只**打断等待**，不强制出帧"。
实现与注释不一致，这条 ADR 把两者对齐。

**决定**：

1. `Pumped` 增加 `Wake`：一批消息里只要出现过非 `WM_APP_WAKE` 的消息才
   算 `Frames`，否则 `Wake` —— 后者不置 `need_frame`。
2. 出帧条件加一条 `!dock.fully_hidden()`：`hide_cur > 0.999` 时一帧都不画。
   这条是**兜底**：即使将来又有哪条路径置了脏标记，也不会画一张屏幕外
   的图。
3. `step_smoke` 同样按 `fully_hidden()` 停摆 —— 粒子位置是纯时间函数，
   隐藏期间算出来也没人看得见。

4. `wait()` 里的烟雾节拍同样要按 `fully_hidden()` 关掉。

**为什么第 4 条比第 2 条更致命**：`step_smoke` 在收起时直接 `return false`，
**没有推进 `next_smoke`**。而 `wait()` 用 `next_smoke` 算还要睡多久 ——
`next_smoke` 停在过去，`left` 恒为 0，`timeout = max(0,1) = 1 ms`。于是
收起状态下主循环以 ~1000 Hz 空转，每一步都跑一遍（只是不出帧）。这条在
日志里完全看不出来，只有 CPU 占用降不下去。

**代价**：收起状态下，主循环仍按 100 ms 醒一次（要轮询光标才能"探头"弹出），
但每次醒来只做几次只读判定，不产生 GPU / `UpdateLayeredWindow` 工作。
实测收起 8 秒 0 帧、CPU 0.26% 单核。

**验收方式**：`MINIDOCK_FPS=1` 每秒打一行帧率。收起后应当**一行都没有**。

**注意**：别把 `pump` 的 `Wake` 当成"什么都不用处理"。它仍然要
`DispatchMessage` —— 面板类窗口（起动台 / 设置 / 菜单）的 `WM_APP_WAKE`
就是在自己的窗口过程里把 `next_repaint` 置为现在，主循环随后靠
`due()` 发现并出帧。

---

## ADR-062 · 烟雾网格的身份只认尺寸，位置每帧更新

**背景**：`Fluid::ensure(w, h, margin, thickness)` 把"栏体在网格里的位置"
编码成四边 margin，网格 = 栏体 + margin。于是网格尺寸会跟着栏体走。

**症状**：悬停放大时栏体每帧都在长 → `ensure` 每帧重建 → 速度场与染料
清零 → 烟闪烁。日志实证 `grid` 在放大过程中从 300x49 跳到 341x49。

**决定**：

1. `ensure(w, h, seed, bar, thickness)`：`bar` 是格坐标矩形，**不参与身份
   判定**；只有 `(w, h, seed)` 变了才重建。
2. 掩码 `build_edge` 每次调用重算（O(格数) 的距离场，约 0.1 ms）。
3. 网格取整窗尺寸，并按 `GRID_BUCKET` 分档：悬停时窗口本身也会变宽
   （1496 → 1720 px），不分档仍会跨档重建。

**为什么不干脆把网格钉成一个常数**：竖停靠与横停靠的长宽比差十倍，固定
尺寸要么浪费内存要么不够用。分档是"够稳定"与"不浪费"之间的折中。

**代价**：整窗网格比原来的"栏体 + margin"大 3～12 倍，但 `raster` 对未点亮
的格子提前返回，实测帧率不变（21–26 fps）。

**遗留**：用户报的"四角变成直角"在本轮没有独立复现。掩码一直是胶囊形
（半径 = 栏体高 / 2），若本轮之后仍有直角，说明来源不是掩码 —— 下一轮
要用 `MINIDOCK_SMOKE_TRACE` 的 `maxA` 配合逐像素采样定位（可能是烟团粒子
或位图被窗口边界裁切）。
