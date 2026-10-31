# 迁移台账

上游 `minidock.c` 是**行为基准**。这份文档记录：哪些已经迁完、
哪些在哪个里程碑、验收标准是什么、以及**已知会踩的坑**。

> **基线说明（2026-10-31）**：本台账的里程碑与键表按 **v1.4.0**
> （`minidock.c` 6875 行）写。上游现在已经到 **v1.6.0（9681 行）**，
> 中间还有 v1.4.5 / v1.5.0 / v1.5.5。增量清单见
> [第七节](#七上游-v160-增量2026-10-31-比对) —— **先读那一节再动手加功能**，
> 它列了 23 个新配置键和一处会动到布局的结构变化。
>
> 两份源码放在 `D:\work\MiniDock160\{v1.4.0,v1.6.0}\minidock.c`。

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
- [x] **度量与 C 版逐项相等**（见下方"几何对拍结果"）
- [x] 放大曲线（余弦）、滑动回弹
- [x] 时钟卡片：多行均分 + 时间/日期分组分割线（`core::clock`，ADR-022）
- [x] 栏体圆角渐变（自建网格，ADR-020）
- [x] 透明呈现：离屏 + 分层窗口（M0 已打通，M1 只需按 Dock 尺寸复用）
- [x] 输入：`WM_MOUSELEAVE` + `PointerGone` + 光标形状映射
- [x] 悬停放大 + `WM_MOUSELEAVE`（ADR-023）
- [x] 点击启动（`platform::launch`，与原版 `LaunchPtr` 分支对应）
- [x] 单实例（两个 Dock 会让 `FindWindow` 抓到错的那个）
- [x] 自动隐藏：轮询光标 + 450 ms 宽限 + 按需出帧（ADR-025）
- [x] `SmartHide`（遮挡检测 + 点名诊断，ADR-027）
- [x] `Reserve`（AppBar 预留工作区，RAII 注销，ADR-026）
- [x] 四边停靠（`layout::window_origin` + 竖排尺寸转置，ADR-030）
- [x] 四种停靠边（左/右会转置窗口尺寸，实测通过）
- [ ] 悬停标签气泡逐像素比对（字体度量不同，需单独定容差）
- [x] 拖拽重排 + 跟手残影 + 位移动画（ADR-024）
- [ ] 拖出栏体删除、拖到图标上合并成组
- [x] 中键 / Shift 强制新实例、Ctrl+Alt+D、启动弹跳、全屏隐藏（ADR-031）
- [x] 右键菜单（独立窗口 + 模态捕获，ADR-032）
- [x] 条目增删即时生效（窗口尺寸热更新，ADR-033）
- [x] 设置窗口接入 Dock 进程（进程内第二窗口，改设置即时生效并自动落盘，ADR-036）
- [ ] IME 兜底方案验证
- [x] 运行状态扫描 + 点击激活已运行实例（ADR-028）
- [ ] `FlyStart` 最小化/恢复的飞入动画（纯观感）
- [ ] Store 应用（`AppExecutionAlias`）的 exe 解析

验收：与 C 版同配置并排截图，栏体几何误差 ≤1 px；空闲 5 分钟 CPU 占用 0%。

#### 几何对拍结果（2026-10-30）

用上游 v1.4.0 源码本地编译出 C 版，**同一个 ini** 跑两侧，逐项比对：

| 度量 | C 版 | Rust 版 |
|---|---|---|
| `S` / `Z` / `g` | 52 / 88 / 8 | 52 / 88 / 8 |
| `sepW` / `pad` / `padX` | 6 / 10 / 12 | 6 / 10 / 12 |
| `barH` / `overhang` / `margin` | 76 / 96.6 / 12 | 76 / 96.6 / 12 |
| `clockW` / `clockExtra` | 249 / 263 | 249 / 263 |
| `lw` / `lh` | **1181 / 186** | **1181 / 186** |

窗口在屏幕上的矩形也完全相同：`(689,1414)-(1870,1600)`。

对拍时踩到两个坑，记下来免得重来：

1. **C 版不是 DPI 感知的**（官方靠清单文件声明 PerMonitorV2，本地编译若没带上
   清单，`GetDeviceCaps` 会返回 96，整个窗口被系统放大 1.5 倍）。对比前要在
   C 版 `WinMain` 里补 `SetProcessDPIAware()`，否则得到 96 dpi 的度量。
2. **C 版会把"正在运行的前端应用"作为临时条目插到分割线之后**
   （`cfg.showRunning`，默认 1）。ini 里 7 项，它运行时是 10 项，`lw` 自然
   对不上。测试配置里必须写 `ShowRunning=0`。
   这条也是"为什么 C 版屏幕上多出几个图标"的答案 —— Rust 版**尚未实现**
   该功能（M2 条目管理的一部分）。

C 版退出时会把配置以 UTF-16 全量回写，所以每轮对拍都要用原始 ini 重新启动，
不能复用上一轮被改写的文件。

### M2 · 图标与条目

- [ ] `platform::icons`：`SHGetFileInfo` / `IShellItemImageFactory` → RGBA
- [ ] `.lnk` 解析、`shell:` 协议、网址 favicon 缓存
- [ ] 组图标拼接（原版 `BuildGroupIcon`，2×2 四宫格）
- [ ] 条目增删改、拖拽重排、合并成组
- [ ] 图标纹理缓存分档（原版按 `g_cacheSize` 预缩放到 96/128/192/256；
      Rust 版直接从 256 缩到目标尺寸，少一次重采样，清晰度不低于原版）
- [ ] **显示未固定的运行中应用**（`ShowRunning`）：原版把当前运行的前端
      应用作为临时条目插到分割线之后，是本轮"多出几个图标"的原因

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
- [x] 删除遗留 `minidock-gfx` / `minidock-ui` / `minidock`（ADR-035）

## 三、函数级台账

状态按**代码实际存在与否**标，不按计划标（2026-10-31 核对）。

| 子系统 | 原版函数 | Rust 落点 | 状态 |
|---|---|---|---|
| 配置 | `DefaultCfg` `LoadConfig` `SaveSettingsOnly` `g_tab[]` | `core::config` | ✅ 99 键（v1.6.0 全量，ADR-038） |
| 条目 | `ParseItemLine` `SaveNow` `LoadItems` `SplitFields` | `core::item` `core::store` | ✅ |
| 布局 | `CalcMetrics` `MapR` `ToLogical` `HitIndex` `Clampf` | `core::layout` | ✅ 含小组件条带模型（ADR-038） |
| 缩放 | `Sc` / `MulDiv` | `core::units` | ✅ |
| 时钟文案 | `ClockLines` | `core::clock` | ✅ |
| 运行状态 | `ScanProc` | `core::running` + `platform::app_window` | ✅ |
| 路径 | `InitPaths` `EnsureIni` | `platform::paths` | ✅ |
| 显示器 | `GetMon` `MonEnum` | `platform::display` | ✅ |
| 主题 | `SystemDark` `UpdateTheme` | `platform::theme` + `core::config::dock_is_dark` | ✅ |
| 自启动 | `GetAutostart` `SetAutostart` | `platform::theme` | ✅ |
| 单实例 | `CreateMutexW` + `FindWindowW` | `platform::single_instance` | ✅ |
| Dock 渲染 | `RenderFrame` `DrawBar` `DrawBubbleAt` | `gpu::dock` | ✅ |
| 倒影 | `DrawReflection` | `gpu::dock` | ❌ 未做（v1.6.0 还改成了 12 条细带，§7.5） |
| 画布/呈现 | `CanvasNew` `CanvasPresent` `MakeSurface` | `gpu::frame` + `gpu::offscreen` + `gpu::present` | ✅ 离屏 + `UpdateLayeredWindow`（ADR-008） |
| 图标 | `LoadItemIcon` `HiconToGp` `FitBitmap` `AlphaBBox` `LetterTile` | `platform::icon` + `core::icon` + `gpu::worker` | ✅ 含圆角/缩放，工作线程取 |
| 网址 favicon | `IsUrlStr` `HostOf` `Fnv` `IconFileFor` `FaviconBitmap` | —— | ❌ 未做（网址条目走首字占位） |
| 快捷方式 | `ReadLnk` `ResolveTarget` `BuildItem` | `platform::shortcut` | ✅ |
| 拖放 | `DT_*` `AcceptData` `DropPoint` `GetGlobalText` | —— | ❌ 未做（拖进来加条目还没有） |
| 分组 | `KidAdd` `MergeInto` `ExtractKid` `DissolveGroup` `BuildGroupIcon` | 只有序列化（`core::store`） | ❌ 交互未做，`launch` 明确返回"不支持" |
| 菜单 | `ShowMenu` `MenuProc` `MenuDraw` `HandleCmd` | `gpu::menu` | ✅ 独立窗口 + 模态捕获（ADR-032） |
| 预览 | `ShowPreview` `PvProc` `PvEnum` `ClosePreview` | —— | ❌ 未做（只有配置键） |
| 磨砂 | `UpdateFrost` `BlurPass` | —— | ❌ 未做，`Frost` 退化为渐变 |
| 托盘 | `AddTray` `WM_APP_TRAY` | —— | ❌ 未做 |
| 热键 | `RegisterHotKey` `ToggleHide` | `bin/dock.rs` | ✅ `Ctrl+Alt+D` |
| AppBar | `SetupAppBar` `WM_APP_AB` | `platform::appbar` | ✅ |
| 最小化动画 | `SetMinHook` `MinProc` `AnimStep` | —— | ❌ 未做 |
| 动画 | `Kick` `BounceOff` `Remap` | `gpu::dock`（弹跳/隐藏/放大插值） | ✅ |
| 设置窗口外壳 | `ShowSettings` `DarkTitle` | `gpu::settings_win` | ✅ |
| 设置内容 | `BuildRows` `PaintSettings` `SetProc` `RowHit` `DoAct` | `settings::pages` | ⚠️ 9 页 / 约 90 项；上游 **10 页**，且项目不全 |
| 后台任务 | —— | `gpu::worker` | ✅ 本轮新增（ADR-037） |
| 资源监视器 | `ResCpuPercent` `ResSample` `DrawRes` `ShowResMon` | `platform::sysinfo` + `gpu::resmon_win` | ✅ 卡片 + 详情面板（ADR-039） |

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
- ~~保留 C 版兼容层~~：已于 0.2.1 删除，见 ADR-035。

## 七、上游 v1.6.0 增量（2026-10-31 比对）

**基线变了。** 本台账此前按 v1.4.0 写（6875 行），上游现在到 **v1.6.0
（9681 行）**，中间还隔了 v1.4.5 / v1.5.0 / v1.5.5 三个版本。

下面每个数字都是拿两份源码实际 diff 出来的，不是抄发行说明。复核方式：

```bash
cd /d/work/MiniDock160
python - <<'PY'
import re
def keys(p):
    s = open(p, encoding='utf-8', errors='replace').read()
    return re.findall(r'L"([A-Za-z0-9_]+)"',
                      re.search(r'g_tab\[\] = \{(.*?)\};', s, re.S).group(1))
a, b = keys('v1.4.0/minidock.c'), keys('v1.6.0/minidock.c')
print(len(a), '->', len(b), '新增:', [k for k in b if k not in a])
PY
```

### 7.1 配置：76 → 99 键（+23，**一个都没删**）

> **已处理（ADR-038）**：23 个键已全部进 `core::config` 的 `NUMERIC_KEYS`，
> 键表 99 键与上游 `g_tab[]` 逐项一致。

| 键 | 默认 | 语义 |
|---|---|---|
| `Launchpad` | 0 | 起动台按钮。上游在 1.4 首次运行时自动置 1 |
| `WSide` | 0 | 小组件条带在**右**（0）/ **左**（1） |
| `WOrder` | 0 | 0 = 时钟在前、通知在后；1 = 反过来 |
| `NotifyW` | 0 | 通知小组件（任务栏闪烁 / 未读 → 图标红点） |
| `FanBgL` `FanBgD` | -1 | 弧形展开的**标签背景**色（浅 / 深），-1 = 跟随主题 |
| `FanFgL` `FanFgD` | -1 | 同上，**文字**色 |
| `PinMenu` | 0 | 资源管理器右键菜单加「固定到 MiniDock」 |
| `MultiMode` | 0 | 0 = 多窗口应用合并成一个图标；1 = 每个文档窗口一个 |
| `UpdChk` | 1 | 每天自动查一次更新（只访问 GitHub Releases） |
| `UpdLast` | 0 | 上次检查的时间戳（天） |
| `LpCols` `LpRows` | 7 / 4 | 起动台每页列数 / 行数 |
| `FanDrag` | 1 | 允许从弧形展开里把文件拖出来 |
| `TrayHide` | 0 | 隐藏通知区域图标 |
| `CtrlCenter` | 1 | 控制中心小组件 |
| `CcEnergy` | 1 | 控制中心里额外显示「节能」磁贴 |
| `CcHotspot` | 1 | 控制中心里额外显示「移动热点」磁贴 |
| `ResMon` | 1 | 资源监视器小组件 |
| `ToolW` | 1 | 计时器 / 待办小组件 |
| `WideMask` | 0 | 位掩码，第 k 位 = 第 k 个小组件双倍宽 |
| `ModeW` | 1 | 专注模式小组件 |

**键的取值域没变**：新增的全是 0/1 或 -1..16777215，`core::config` 的
`NUMERIC_KEYS` 表加 23 行即可，不需要新的枚举变体。

### 7.2 配置段：+2，外加一个目录

| 段 | 键 | 说明 |
|---|---|---|
| `[Tool]` | `N` / `T0..Tn` / `Total` / `Remain` | 待办条数 + 每条一行；计时器总秒数 + 剩余秒数 |
| `[M]` | `Name` | 专注模式快照文件里存模式显示名 |

**目录级变化**：`<数据目录>\profiles\<名称>.ini` —— 每个专注模式是一份完整
配置快照。切换前把当前配置备份成 `MiniDock.ini.bak`。

> 我们现在的 `Ini` 会**原样保留**不认识的段（`unknown_sections_survive_roundtrip`
> 有测试），所以读 v1.6.0 配置不会丢数据，只是读不出来。在实现对应功能之前，
> "保留但不解析"就是正确策略。

### 7.3 结构变化：单个时钟卡片 → 小组件条带（**会动布局**）

> **已处理（ADR-038）**：`core::layout::WidgetStrip` 已复刻上游的
> `ord[]` 顺序、`WOrder` 交换、宽度公式与 `g_ce` 汇总。硬约束：
> 只开时钟且 `WSide=0` 时几何与 v1.4.0 逐像素一致（实测 A/B 对比
> `GEOM` 输出逐字节相同）。五个新小组件本身尚未移植，
> 未实现的不进条带（否则栏体留白）。

这是唯一一处会碰到我们已经"逐项对拍通过"的几何的地方。

v1.4.0：

```c
g_ce = cfg.clock ? (float)g_clockW + M.g + 6 : 0;
```

v1.6.0：

```c
g_ce = 0;
for (int k = 0; k < g_wn; k++) g_ce += (float)g_wWid[g_wk[k]] + M.g + 6;
```

最多 6 个小组件，宽度按 `M.barH` 的比例算：

| 小组件 | 单倍宽 | 双倍宽 |
|---|---|---|
| 时钟 | `g_clockW`（算法与 v1.4.0 相同） | 不支持 |
| 通知 | `barH * 0.92` | — |
| 控制中心 | `barH * 0.72` | `barH * 1.6` |
| 资源监视器 | `barH * 1.16` | `barH * 2.2` |
| 计时器 | `barH * 0.72` | `barH * 1.6` |
| 专注模式 | `barH * 0.72` | — |

并且 `WSide=1` 时栏体整体右移 `g_ce`：

```c
bl = (LW - baseW - g_ce) / 2 + (cfg.wSide ? g_ce : 0);
```

我们现在的 `Metrics::clock_w` / `clock_extra` 只建模了 v1.4.0 的单卡片情形。
**不换成"小组件条带"模型，后面 6 个小组件一个都放不下。**

### 7.4 新功能：120 个新函数

`v1.4.0` 318 个函数 → `v1.6.0` 433 个。按子系统归类：

| 功能 | 上游函数 | 量级 | 依赖 |
|---|---|---|---|
| **起动台**（全屏应用网格、分页、搜索、合并成文件夹） | `Lp*` 共 22 个 + `LpCols/LpRows` | 大 | 独立全屏窗口 + 拖放 + 图标 |
| **控制中心**（音量滑块、磁贴、Wi‑Fi/蓝牙/热点状态） | `Cc*` 13 个 + `DrawCC` + `ShowCtrlCenter` | 大 | Core Audio（`IAudioEndpointVolume`）、Win+A 转发、`powercfg` |
| **日历 / 农历**（农历、节气、干支、节日） | `Cal*` 7 个 + `SolarToLunar` + `L*` 5 个 + `GanzhiText` + `TermDate` | 中（纯算法） | 无 Win32，可全进 `core` |
| **通知小组件** | `Notify*` 3 个 + `Nf*` 3 个 + `ShowNotifyList` + `DrawNotify` | 中 | 任务栏闪烁 / 未读检测 |
| **资源监视器** | `Res*` 4 个 + `ShowResMon` | 小 | `GetSystemTimes` / `GlobalMemoryStatusEx` / `GetDiskFreeSpaceEx` |
| **计时器 / 待办** | `Tool*` 6 个 + `ShowTool` | 小 | `[Tool]` 段 |
| **专注模式（多布局）** | `Profile*` 5 个 + `DrawMode` + `ShowModeMenu` | 中 | `profiles\*.ini` + 整份配置切换 |
| **动画重做**（genie 变形、飞入飞出、磨砂快照） | `Anim*` 3 个 + `Genie*` 4 个 + `Fly*` 2 个 + `FrostSnap*` 3 个 | 大 | 逐帧位图变形 |
| **多窗口应用模式** | `ToggleWindow*` 3 个 + `IsDocApp` + `BuildWinIcon` + `SyncWin` | 中 | 文档窗口枚举 + 缩略图 |
| **系统集成** | `PinPath` `SetPinMenu` `HideTaskbarButton` `TaskbarHidden` `TrayBalloon` | 小 | 注册表 / 托盘 |
| **更新检查** | `StartUpdateCheck` `UpdThread` `VerNum` | 小 | 联网 + JSON 解析 |
| **弹窗基础设施** | `PopPaint` `PopProc` `PopRun` + `ShowWidgetMenu` + `WidgetAt` | 中 | 通用弹出面板（多个小组件共用） |

### 7.5 既有功能的 4 处行为变化

| # | 变化 | 出处 |
|---|---|---|
| 1 | **最大化闪屏**：还原"最小化前是最大化"的窗口时直接 `SW_MAXIMIZE`，不再"先回原位再最大化" | `v1.6.0:5440`（`WPF_RESTORETOMAXIMIZED`） |
| 2 | **倒影**：从两段硬接缝改成 12 条细带 + 平方衰减，连续淡出 | `v1.6.0:2803`（`const int N = 12`） |
| 3 | **设置项搬家**：「显示图标倒影」从「小组件」页搬到「个性化」页下的「图标倒影」小节（长度/间距/浓度仍在独立的「倒影设置」子页） | `v1.6.0:7902`（`RHdr(P, L"图标倒影")`） |
| 4 | **应用组标题栏加视图切换按钮**：当前网格就显示「列表」图标，点一下切换，状态仍写回组配置 | 发行说明；`BuildGroupIcon` 相关 |

### 7.6 建议的移植顺序

按"能独立验证 + 不引入新窗口"排序，先做便宜的：

1. **小组件条带**（§7.3）—— 先做布局模型。工作量小，但会让 `core::layout`
   的几何黄金测试全部重算；不先做，后面 6 个小组件一个都放不下。
2. **资源监视器** —— 三个只读系统调用 + 一个小组件面板，是最小的端到端验证。
   > **已做（ADR-039）**：条带卡片 + 点击弹详情面板。
3. **计时器 / 待办** —— `[Tool]` 段 + 一个面板，顺便验证"新配置段"的读写。
4. **日历 / 农历** —— 纯算法，可以全进 `core` 并写单测（节气表、干支换算），
   不依赖任何 Win32。
5. **专注模式** —— 文件快照 + 一个菜单，无新窗口。
6. **控制中心** —— 最大的一块，音量要接 Core Audio。
7. **起动台** —— 独立全屏窗口 + 分页 + 拖放，依赖最多。

**暂时不做**：更新检查（要联网 + JSON）、多窗口模式（要逐文档窗口枚举）、
动画重做（genie 是逐帧位图变形，与我们的 egui 即时模式不搭）。

### 7.7 顺带发现的两处早已存在的偏差

不是 1.6.0 引入的，但既然对着源码核了一遍，一并记下来：

- **设置页数**：上游 10 页（通用 / 个性化 / 位置与显示 / 交互动效 / 小组件 /
  图标自定义 / 性能 / 配置与备份 / 关于 / 右键菜单配置），我们是 9 页且名称不同。
  这是 M0 阶段的简化，要补的内容见 §7.4。
- **`Metrics` 的字段名**：我们叫 `clock_w` / `clock_extra`，上游在 v1.6.0 里
  已经泛化成 `g_wWid[]` / `g_ce`。改名建议与 §7.3 的重构一起做。
