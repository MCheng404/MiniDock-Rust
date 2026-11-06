# 架构

## 一、一个 UI 线程 + 四个工作线程

```
┌─ UI 线程 ──────────────────────────────────────────────────┐
│ Win32 消息循环（热键 / AppBar / 捕获 / 全屏检测 / 自动隐藏轮询）│
│   ↓ 消息                                                   │
│ Dock 状态（一个 Box<Dock>，wndproc 与主循环共用同一份）      │
│   ↓                                                       │
│ egui::Context::run_ui → tessellate → egui-wgpu → 离屏纹理   │
│   ↓                                                       │
│ 回读 → 预乘 BGRA 写入 DIB → UpdateLayeredWindow            │
└────────────────────────────────────────────────────────────┘
   ↑ try_recv（从不等）           ↑ PostMessage(WM_APP_WAKE)
┌──┴──────────────┬──────────────┬──────────────┬────────────┐
│ IconLoader      │ Scanner      │ DiskWriter   │ Opener     │
│ shell 取图标    │ 运行状态     │ 写 ini       │ 启动应用   │
│ + rayon 并行    │ + 遮挡检测   │              │            │
└─────────────────┴──────────────┴──────────────┴────────────┘
```

**所有会阻塞的系统调用都在下面那四个线程里**（ADR-037）。UI 线程只做
`try_recv`，永远不等；工作线程有结果时 `PostMessage` 敲一下，主循环的
`MsgWaitForMultipleObjectsEx` 立刻返回 —— 空闲时仍然真睡，不轮询。

没有 `wgpu::Surface` —— Windows 上没有带 alpha 的交换链（ADR-008），
所以"呈现"是每帧一次同步回读 + `UpdateLayeredWindow`。这条回读是 UI 线程
上唯一剩下的阻塞点，也是将来要提帧率时的第一个目标。

线程里**绝对不做**的事：不碰 `egui::Context`（纹理注册与图集上传必须由
UI 线程做）、不创建窗口、不改窗口状态。`wgpu` compute 是例外：
`Device` / `Queue` 是 `Send + Sync`，图标圆角遮罩在工作线程提交并回读。

rayon 只用来并行派发图标批；像素处理本身已移到 GPU compute，
毛玻璃卷积也不再占 CPU 核。其余线程都是 IO 等待。

| 工作 | 线程 | 代价 |
|---|---|---|
| 图标取用 | `IconLoader` | 启动时窗口先出来，图标陆续补上；遮罩走 GPU compute |
| 窗口枚举 | `Scanner` | 1.2 s / 100 ms 一轮，都在后台 |
| 写 ini | `DiskWriter` | 停手 400 ms 后才写 |
| 毛玻璃抓屏 | `frost::Live` | 100 ms 起步，桌面静止退避到 800 ms |
| 毛玻璃模糊 | **UI 线程**（`shaders::frost` compute） | 每帧提交约 1.8 ms，零回读 |
| 烟雾光栅 | **UI 线程**（`shaders::smoke` compute） | 每帧上传流场，输出直接注册为 egui 纹理 |

## 二、crate 依赖

```
minidock-core        （无 Win32 依赖，纯逻辑）
   ↑        ↑
   │        └── minidock-settings （egui，无 wgpu）── kittest 测试
   │                    ↑
   └── minidock-platform│
              ↑         │
         minidock-gpu ──┘  （wgpu + Win32 窗口/输入/命中 + 工作线程）
```

依赖方向是单向的：`gpu → settings → core`、`gpu → platform`。
`settings` 不认识 `wgpu`，所以它的测试不需要 GPU/显示器；
`core` 不认识 Windows，所以它的测试能跑在任何宿主上。

## 三、渲染管线

一帧的完整路径在 **`frame.rs::FrameRenderer::draw_frame`** 里，`render` 与
`render_rgba` 共用同一份实现 —— 这条链路原先两边各写一遍，逐行相同，任何
API 变动都要改两处。

1. 调用方组装 `egui::RawInput`（含 `screen_rect`、`time`、`viewports`），
   传给 `render(raw, hwnd, pos, |ui| ...)`。
2. `ctx.run_ui(raw, build)`：闭包里只负责画。
3. `ctx.tessellate(shapes, ppp)` → `Vec<ClippedPrimitive>`。
4. `textures_delta.set` 逐项 `renderer.update_texture`（字形图集、图标纹理）。
5. `renderer.update_buffers` 写顶点/索引缓冲。
6. `begin_render_pass`，`LoadOp::Clear(Color::TRANSPARENT)` —— **透明清屏是窗口能透出桌面的关键**。
7. `renderer.render(&mut pass, ...)`。
8. `copy_texture_to_buffer` → `queue.submit` → `textures_delta.free` + `clear()`
   （不 clear 会在 Drop 时断言 panic）。
9. 之后分两路：
   - `render`：`with_mapped_pixels` 把带 256 字节行对齐的回读缓冲**直接**
     写进 DIB（同时去对齐 + RGBA→BGRA），再 `UpdateLayeredWindow`。
     没有整帧中间缓冲，也没有每帧 1–6 MB 的堆分配。
   - `render_rgba`：`read_pixels` 返回紧密 RGBA，不呈现 —— 离屏截图
     （`examples/settings_shot.rs`）与像素级回归走这条路。

返回 `FrameOutcome { cursor, repaint_delay }` —— 调用方只需要这两个值。

毛玻璃背景不走上屏纹理上传：`frost::Live` 线程只抓屏并把字节用 `Arc`
交给 UI 线程；`FrameRenderer` 在 `draw_frame` 前调用 `blur_gpu` 的 WGSL
compute（两趟可分离高斯 + 圆角 coverage），输出 `Rgba8Unorm` 纹理直接
注册给 egui，**零回读**。CPU `blur_rgba` / `compose` 保留为回退路径与
像素级对照基准。

### GPU shader 框架：`gpu/shaders/`

所有逐像素着色器集中在这里。`ShaderRuntime` 缓存 compute pipeline、管理
输出 `Rgba8Unorm` 纹理、统一注册为 egui native texture；WGSL 分文件存放，
每条 GPU 路径都有 CPU 参考实现和 `#[ignore]` 真机逐通道对照（差 ≤1）。

| 模块 | GPU 做什么 | CPU 参考 |
|---|---|---|
| `frost.wgsl` | 两趟可分离高斯 + 圆角 + 预乘 | `frost::blur_rgba` |
| `smoke.wgsl` | 双线性放大 + 三层噪声 + 调色 + pre/post | `Fluid::raster` / `post` |
| `icon.wgsl` | 图标圆角 alpha 遮罩 | `core::icon::apply_round_mask` |
| `puff.wgsl` | 64×64 软边烟团贴图 | `SmokeFx::puff_rgba_cpu` |

CPU 版本只作回退与测试基准。**保留在 CPU 的不是着色器**：
`Fluid::step`（流体物理模拟）、`Noise` 表（共享数据源，启动上传一次）、
`fit_icon` / bbox（标量几何）、`platform::icon::bgra_to_rgba`（GDI 格式
重排，platform 层按架构不依赖 wgpu）、`present::swap_rb`（GPU→CPU 呈现
桥接，`UpdateLayeredWindow` 要求 CPU DIB，M3 换 DirectComposition 才能消除）。

### 帧节拍与常驻内存

活跃态由 `App::wait()` 按 `perf::pacing_ms` 唤醒：默认不限（1 ms 泵），
`[performance] FrameCap` 可设上限，`VSync=1` 时按
`DwmGetCompositionTimingInfo` 的实际刷新率对齐（本机 16 ms）。空闲态
仍睡满 `FS_CHECK_MS`，保持「空闲 0 帧」。设置窗口用 `repaint_in()` 把
egui 的重绘期限并入等待，不再靠 1 秒兜底。

MSAA 默认按面积自适应：> 2 M 物理像素用 2x（2560×1600 省约 31 MiB
常驻显存），小窗口仍 4x；`Msaa` 可强制 1/2/4（下次启动生效）。
CJK 字体（`msyh.ttc`，8.04 MiB）由 `OnceLock<Arc<FontData>>` 全进程
共享，不再每个 `FrameRenderer` 各读一份。

### 槽位布局：`core::layout`

"每个图标画在哪、多大、鼠标命中谁"全部是纯函数，住在 `core::layout`：

| 函数 | 职责 |
|---|---|
| `calc_metrics` | 由配置 + 工作区算出 `Metrics`（图标边长、栏高、时钟占位……） |
| `base_centers` | 未放大时的槽位中心（放大权重必须以它为基准） |
| `magnify_targets` | 本帧每个槽位的目标边长（余弦衰减） |
| `lerp_to` | 向目标值指数逼近一帧，返回"还需不需要下一帧" |
| `place_slots` | 放大后的实际槽位 `bar_l/bar_w/slot_l/slot_r/act_c` |
| `bars` | 纵向基线 `bar_top/bar_bottom/icon_bottom` |
| `hit_test` | 命中下标（从左到右第一个，见下） |

GPU 版与 GDI+ 版都只是"取结果 → 画"。这套逻辑曾经两边各写一遍，
已经漂移过（GDI+ 版拿动画中的 `cur` 去算基准宽度，导致基准槽位每帧移动）。

### 设置界面：左导航 + 右侧卡片

`minidock-settings` 是**纯 UI**：没有 HWND、没有 device、没有线程，所以能被
kittest 无头驱动。结构按"一页一个文件"拆开：

```
crates/minidock-settings/src/
  pages/
    mod.rs         Page 枚举、SettingsState、Command、两列骨架
    widgets.rs     共享控件：card / row / toggle / slider / combo / 侧栏
    options.rs     下拉选项表（纯数据）
    general.rs     常规      appearance.rs  外观     position.rs   位置
    icons.rs       图标      interaction.rs 交互     extras.rs     附件
    effect.rs      效果      performance.rs 性能     library.rs    图标库
    add.rs         添加
```

加一页设置 = 加一个文件 + `Page` 枚举加一项；加一个设置项 = `core::config`
加一个键 + 对应页面加一行。**控件长什么样、怎么排版只在 `widgets.rs` 里改**。

布局不是 `SidePanel` 而是手工切两块 `Rect`（`ui.new_child(max_rect)`）：
设置窗口外面还套着标题栏，`SidePanel` 按根 Ui 布局会跟标题抢地方。起点用
`available_rect_before_wrap()` —— `max_rect()` 不随已用内容收缩，用它会把
两列画到标题上。

视觉基准是 [`docs/settings-prototype.html`](docs/settings-prototype.html)：
页面、卡片、行与 Rust 端一一对应，改设计先改雏形。**不靠人肉截图验收** ——

```bash
cargo run -p minidock-gpu --example settings_shot -- 出图.bmp 外观 [dark]
```

走的是与真实窗口完全相同的 `FrameRenderer` + `settings_win::shell`，只把最后
一步的 `UpdateLayeredWindow` 换成写 BMP。

## 四、帧调度：空闲不提交

```rust
loop {
    if ctx.has_requested_repaint() { render()?; sleep(8ms); continue; }
    GetMessageW(...)   // 阻塞，直到有消息
    TranslateMessage / DispatchMessage
}
```

egui 只在有动画、悬停、输入时请求重绘；其余时间线程阻塞在 `GetMessageW`，
CPU 占用为 0。这条性质是产品承诺（"空闲无定时器"），**不能被任何"为了流畅"
的常驻 vsync 循环破坏**。M1 引入的动画必须走 `ctx.request_repaint()`，
不允许自建 60fps 定时器。

## 五、输入与命中

- 鼠标/滚轮/键盘由 `input.rs` 翻译成 `egui::Event`。这份翻译是"不用 eframe"
  的代价，M0 只覆盖鼠标、滚轮、Esc/Enter/Tab/方向键。
- **命中测试在 `WM_NCHITTEST` 里同步完成**：把光标坐标换成逻辑像素，
  判断是否落在面板矩形内，是则 `HTCLIENT`，否则 `HTTRANSPARENT`。
  系统每次点击都会问，不需要轮询、不存在"鸡生蛋"问题。
- M1 要补：按圆角/图标实际形状判定（现在是矩形近似）、`WM_MOUSELEAVE`、
  IME（`WM_IME_*` → `egui::Event::Ime`），以及输入框的原生 `EDIT` 兜底。

## 六、线程模型

| 线程 | 职责 | 禁止事项 |
|---|---|---|
| UI | 消息泵、egui、GPU 提交、命中 | 不做磁盘 IO、不枚举窗口 |
| rayon 池 | 图标像素处理（缩放判定 + 圆角遮罩） | 不碰窗口/device |
| `IconLoader` | shell/COM 取图标，**串行** | 不碰 UI 状态 |
| `Scanner` | 运行状态 + 遮挡检测 | 不产生 `wgpu` 对象 |
| `DiskWriter` | 写 ini（400 ms 防抖、串行保序） | 不碰窗口 |
| `Opener` | `ShellExecuteW` / 激活已有窗口 | 不碰窗口 |
| `minidock-frost` | GDI 抓屏 + CPU 可分离高斯（面板可见时） | 不碰窗口/device |

跨线程数据用 `std::sync::mpsc`（任务与结果各一条通道）+ `PostMessage` 唤醒。
**没有 `ArcSwap` / `crossbeam-channel`**：本项目跨线程传的是值拷贝，不存在
读多写少的共享快照，队列深度也只有个位数（ADR-037）。
不用 tokio：这里没有大量并发连接，只有几条长驻线程 + 一个线程池，
async 运行时带来的调度开销与栈占用都不划算。

## 七、不变式（违反即 bug）

1. **字体先于第一帧**。egui 0.37 缺字形直接 `panic`（`MissingGlyphPolicy::Panic`），
   不是画豆腐块。`fonts::install_cjk` 必须在 `run_ui` 之前调用。
2. **`GWLP_USERDATA` 里存的是 `Box<Dock>` 内的地址**。`Dock` 必须一直待在堆上：
   一旦被移出 `Box`（比如改成 `App` 的裸字段），窗口过程手里的指针立刻失效，
   表现为"点一下进程就崩"、且没有任何 Rust panic 输出（ADR-036 记过同一个坑）。
   同理，`SettingsWindow::new` 直接返回 `Pin<Box<Self>>`。
3. **设置界面不产生副作用**。它只能改 `Config` 并返回 `Command`；
   文件、注册表、对话框由主程序执行。这条保证了 kittest 的可断言性。
4. **一个 Device**。禁止为第二个窗口再建 device —— 纹理与图集无法共享，
   内存会翻倍。菜单 / 设置窗口各自有自己的 `FrameRenderer`，但共用同一个
   `Gpu`（device + queue）。
5. **配置写入只有一个出口**。所有值必须经 `Config::apply_i32`（内含夹紧），
   不允许直接改字段。原版崩溃的一半来自"某处忘了夹紧"。

## 八、与 C 版的模块映射

| 原版（minidock.c） | Rust | 状态 |
|---|---|---|
| `LoadConfig` / `SaveSettingsOnly` / `g_tab[]` | `core::config` | ✅ 已迁移 + 夹紧 + 单测 |
| `ParseItemLine` / `SaveNow` / `LoadItems` | `core::item` / `core::store` | ✅ 已迁移 + 单测 |
| `CalcMetrics` / `MapR` / `ToLogical` / `HitIndex` | `core::layout` | ✅ 已迁移 + 单测 |
| `InitPaths` | `platform::paths` | ✅ |
| `GetMon` / `GetDeviceCaps` | `platform::display` / `win` | ✅ |
| `SystemDark` / `GetAutostart` | `platform::theme` | ✅ |
| `BuildRows` / `PaintSettings` / `SetProc` | `settings::pages`（10 页，一页一文件） | ✅ 布局重做（ADR-049） |
| `RenderFrame` / `DrawBar` / `DrawBubbleAt` | `gpu::dock`（待写） | ⏳ M1 |
| `LoadItemIcon` / `HiconToGp` / `FitBitmap` | `platform::icons`（待写） | ⏳ M2 |
| `UpdateFrost` / `BlurPass` | `gpu::frost`（DXGI + compute） | ⏳ M3 |
| `DT_*`（OLE 拖放） | `platform::drop` | ⏳ M2 |
| `ShowMenu` / `ShowGroup` / `ShowFan` / `ShowPreview` | `gpu::popups` | ⏳ M3 |
| `SetMinHook` / `AnimStep` | `platform::minimize` | ⏳ M4 |

函数级台账与验收标准见 [`PORTING.md`](PORTING.md)。
