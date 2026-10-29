# 迁移台账

上游 `minidock.c`（4799 行）是**行为基准**。这份文档记录：哪些已经迁完、
哪些在哪个里程碑、验收标准是什么、以及**已知会踩的坑**。

## 一、原则

1. **像素与行为对齐优先**。没有"顺手优化"，除非能给出量化理由。
2. **纯逻辑进 `core`**，能在无显示器环境测试；窗口与 GPU 才进 `gpu`。
3. **一个配置键只有一个夹紧入口**（`Config::apply_i32`），不允许旁路。
4. **补丁只加不猜**：原版没有的语义（例如"关闭磨砂 = 退回渐变"）必须写明来由。

## 二、里程碑

### M0 · 尖刺（已完成，2026-10-27）

目标：证明技术前提成立，不写业务。

| 验收项 | 结果 |
|---|---|
| wgpu 能建 DX12 设备并出帧 | ✅ `Dx12 / AMD Radeon 610M`，`Bgra8UnormSrgb` |
| egui 中文界面正常渲染 | ✅ 9 个页签、复选框、下拉框全部正确 |
| 设置界面可无头测试 | ✅ `cargo test -p minidock-settings` 5/5 |
| 窗口透明（逐像素 alpha） | ✅ **离屏渲染 + `UpdateLayeredWindow`**，圆角外透出桌面 |
| 逐像素命中 | ✅ 分层窗口按 alpha 判定，无需 `WM_NCHITTEST` |
| 每帧记账（耗时/RSS） | ✅ 性能页实时显示 |
| 体积 / 常驻内存 | ✅ release 7.2 MB；设置窗 **111 MB**（调优前 271 MB，见 BUILD.md）|
| 空闲 CPU | ✅ 单实例静止 30 秒：**0.000 秒**（阻塞在 INFINITE）|
| 单实例 | ✅ 第二次启动唤起已有窗口后退出（ADR-015）|
| 重绘调度 | ✅ 按 egui 的 `repaint_delay` 排帧，0 值节流到 60 fps（ADR-014）|
| 上游 v1.4.0 键表 | ✅ 76 键建模 + 顺序一致 + 布局公式更新（ADR-017）|

**两个必须记住的发现：**

1. **wgpu 的 DX12 表面没有 alpha 模式。** 默认实例会挑 Vulkan（本机有 AMD
   Vulkan 驱动），Vulkan 同样不提供 `CompositeAlphaMode`；强制 DX12 后
   `alpha_modes` 仍只有 `Opaque`。结论：**不能用 wgpu 的 `Surface` 直接做
   透明窗口**。已改为离屏渲染 + `UpdateLayeredWindow`（ADR-008），
   实测：不透明面板时整窗逐像素等于 `(24,25,30)`，圆角外 alpha=0 处透出桌面。
   长期方案（DirectComposition + 自建交换链）留到 M3 评估，用于省掉回读。
2. **内存的账单是后端给的，不是驱动给的。** 同一块 AMD 核显：只建 DX12 设备
   164 MB、Vulkan 103 MB、原生 D3D11 34 MB；完整 setting 窗 271 MB。
   19 MB 的缓冲在 wgpu 下还会放大成 +64 MB（见 `BUILD.md` 的基线表）。
   这推翻了"GPU 路线必然吃 150 MB"的判断。曾据此提议换 `egui_glow`
   （ADR-011），**已否决**：保留 `egui-wgpu` 单栈。显卡选择的落点是
   `[Graphics] Adapter`（ADR-012）。

3. **DPI 必须自己声明。** 进程不声明 per-monitor v2 时，Windows 会把窗口位图
   整体拉伸，`GetClientRect` 与物理像素对不上，表面尺寸与客户区错位
   （第一版表面 1320×1080、客户区 880×720，画面被裁切）。
   逻辑尺寸 → 物理尺寸的换算必须在建窗时完成。

### M1 · Dock 骨架（进行中）

范围：状态、布局、绘制、悬停放大、自动隐藏。
不含图标加载与拖拽。

- [x] `gpu::dock`：复用 `core::layout` 的度量与槽位数学（`dock.rs` + `minidock-dock`）
- [x] 命中与放大映射改成 C 版语义（ADR-018）
- [ ] 与 C 版并排截图比几何（栏体已对齐，图标仍是占位，差分要等 M2 真图标）
- [ ] 放大曲线（余弦）、滑动回弹、隐藏位移
- [ ] 时钟卡片（横向/纵向两种排布）
- [ ] 悬停标签气泡（`measure_text` 自适应宽度）
- [x] 透明呈现：离屏 + 分层窗口（M0 已打通，M1 只需按 Dock 尺寸复用）
- [x] 输入：`WM_MOUSELEAVE` + `PointerGone` + 光标形状映射
- [ ] 拖拽捕获、IME 兜底方案验证

验收：与 C 版同配置并排截图，栏体几何误差 ≤1 px；空闲 5 分钟 CPU 占用 0%。

### M2 · 图标与条目

- [ ] `platform::icons`：`SHGetFileInfo` / `IShellItemImageFactory` → RGBA
- [ ] `.lnk` 解析、`shell:` 协议、网址 favicon 缓存
- [ ] 组图标拼接（原版 `BuildGroupIcon`，2×2 四宫格）
- [ ] 条目增删改、拖拽重排、合并成组
- [ ] 图标纹理缓存（按 96/128/192/256 分档，与原版 `g_cacheSize` 一致）

验收：128 个条目、全部设置项遍历（含越界值）无崩溃；图标内存峰值 ≤ 原版 1.2×。

### M3 · 弹窗与磨砂

- [ ] 右键菜单、组展开、弧形堆叠（fan）、图标选择对话框
- [ ] DWM 窗口预览（`DwmRegisterThumbnail`，必须留在 UI 线程）
- [ ] 磨砂：DXGI Desktop Duplication + compute 两趟模糊
- [ ] 磨砂不可用（RDP/虚拟机）时退化为 CPU 模糊

验收：菜单/组/预览的位置与原版逐像素一致；磨砂开启时帧时间 ≤ 2 ms。

### M4 · 系统集成与收尾

- [ ] 托盘、全局热键、AppBar 预留、单实例唤起
- [ ] 最小化飞入动画（`SetWinEventHook`）
- [ ] 自动启动注册表、导出/导入 `.mdock`
- [ ] 删除遗留 `minidock-gfx` / `minidock-ui`

## 三、函数级台账

| 子系统 | 原版函数 | Rust 目标 | 状态 |
|---|---|---|---|
| 配置 | `DefaultCfg` `LoadConfig` `SaveSettingsOnly` `g_tab[]` | `core::config` | ✅ |
| 条目 | `ParseItemLine` `SaveNow` `LoadItems` `SplitFields` | `core::item` `core::store` | ✅ |
| 布局 | `CalcMetrics` `MapR` `ToLogical` `HitIndex` `Clampf` | `core::layout` | ✅ |
| 缩放 | `Sc` / `MulDiv` | `core::units` | ✅ |
| 路径 | `InitPaths` `EnsureIni` | `platform::paths` | ✅ |
| 显示器 | `GetMon` `MonEnum` | `platform::display` | ✅ |
| 主题 | `SystemDark` `UpdateTheme` | `platform::theme` + `core::config::dock_is_dark` | ✅ |
| 自启动 | `GetAutostart` `SetAutostart` | `platform::theme` | ✅ |
| 单实例 | `CreateMutexW` + `FindWindowW` | `platform::single_instance` | ✅ |
| 设置界面 | `BuildRows` `PaintSettings` `SetProc` `RowHit` `SetSlider` `DoAct` | `settings::pages` | ✅ 9 页壳 |
| Dock 渲染 | `RenderFrame` `DrawBar` `DrawBubbleAt` `DrawReflection` | `gpu::dock` | ⏳ M1 |
| 画布 | `CanvasNew` `CanvasPresent` `MakeSurface` `FreeSurface` | `gpu::surface`（ADR-008 后改写） | ⏳ M1 |
| 图标 | `LoadItemIcon` `HiconToGp` `FitBitmap` `AlphaBBox` `LetterTile` | `platform::icons` | ⏳ M2 |
| 网址 | `IsUrlStr` `HostOf` `Fnv` `IconFileFor` `FaviconBitmap` | `platform::url` | ⏳ M2 |
| 快捷方式 | `ReadLnk` `LnkIconBitmap` `ResolveTarget` `BuildItem` | `platform::shell` | ⏳ M2 |
| 拖放 | `DT_*` `AcceptData` `DropPoint` `GetGlobalText` | `platform::drop` | ⏳ M2 |
| 分组 | `KidAdd` `MergeInto` `ExtractKid` `DissolveGroup` `BuildGroupIcon` | `core::group` + `gpu::popups` | ⏳ M2 |
| 菜单 | `ShowMenu` `MenuProc` `MenuDraw` `HandleCmd` | `gpu::menu` | ⏳ M3 |
| 预览 | `ShowPreview` `PvProc` `PvEnum` `ClosePreview` | `gpu::preview` | ⏳ M3 |
| 磨砂 | `UpdateFrost` `BlurPass` | `gpu::frost` | ⏳ M3 |
| 托盘 | `AddTray` `WM_APP_TRAY` | `platform::tray` | ⏳ M4 |
| 热键 | `RegisterHotKey` `ToggleHide` | `platform::hotkey` | ⏳ M4 |
| AppBar | `SetupAppBar` `WM_APP_AB` | `platform::appbar` | ⏳ M4 |
| 最小化 | `SetMinHook` `MinProc` `AnimStep` | `platform::minimize` | ⏳ M4 |
| 动画 | `Kick` `BounceOff` `Remap` | `gpu::anim` | ⏳ M1 |
| 设置窗口外壳 | `ShowSettings` `DarkTitle` | `gpu::settings_window` | ⏳ M1 |

## 四、风险清单

| # | 风险 | 影响 | 应对 |
|---|---|---|---|
| 1 | wgpu 无 alpha 表面 | Dock 无法透明 | ADR-008：离屏 + 分层窗口（先做），或 DComp |
| 2 | 中文 IME | 设置页输入框不可用 | 先叠原生 `EDIT`（原版做法），再评估 `WM_IME_*` 桥接 |
| 3 | DWM 缩略图是窗口级 API | 无法离线程 | 保留在 UI 线程，接受 |
| 4 | AppBar 需要先有 HWND | 与 GPU 表面初始化顺序耦合 | 建窗 → 注册 AppBar → 再建 surface |
| 5 | 拖放走 OLE `IDropTarget` | 需要 COM 套间与消息泵配合 | 复用 `OleInitialize` + `RegisterDragDrop`，M2 单独验证 |
| 6 | egui 缺字形直接 panic | 无中文字体的机器直接崩 | 启动即装系统字体，失败则消息框提示而非静默 |
| 7 | `opt-level="z"` 可能拖慢动画 | 掉帧 | 用 `Renderer` 实测；必要时对热点函数 `#[inline]` |
| 8 | 组图标拼接的像素对齐 | 与原版有肉眼差 | 按原版四宫格公式重写，截图差分 |

## 五、像素对齐方法

1. 用 `harness/MiniDock.ini` 的同一份配置分别启动 C 版与 Rust 版。
2. 固定 DPI（100% / 150% 各一次）、固定显示器、关闭磨砂（消除背景差异）。
3. 用 `scratch/capture.c` 抓窗口矩形（**必须与目标进程同样声明 DPI 感知**）。
4. 差分：允许 ≤1 px 的圆角/文字抗锯齿差异，几何（栏宽、槽位、图标边长）必须完全一致。

> 截图工具本身踩过 DPI 的坑：不声明感知时 `BitBlt` 的源坐标被虚拟化，
> 截出来的区域整体偏移，看起来像"渲染错位"。诊断任何几何问题前，
> 先确认截图链路是 DPI 一致的。

## 六、明确不做

- **跨平台**：Dock 依赖 shell/DWM/AppBar/托盘，移植到 macOS 等于重写产品。
- **多进程架构**：单进程足够，IPC 只会增加崩溃面。
- **保留 C 版兼容层**：M4 直接删除 `minidock-gfx` / `minidock-ui`。
