# 变更记录

格式：里程碑为单位，每条写"做了什么 / 为什么 / 已知问题"。
版本号从 1.0.0 起以 `Cargo.toml` 为唯一来源；此前按里程碑递进，
未与 crate 版本绑定（0.1.0 一直没动过）。

---

## 2.0.0 — 更名为 NookSill（2026-11-06）

项目、仓库、产物全部改名：**MiniDock → NookSill**，仓库改为 **NookSill-Rust**。

- crate 与二进制：`nooksill-core` / `nooksill-platform` / `nooksill-settings` /
  `nooksill-gpu`，主程序 `nooksill-dock.exe`；资源改成 `assets/nooksill.{ico,rc}`。
- 运行时标识：窗口类 `NookSillWindow`、单实例互斥体 `Local\NookSill_Singleton`、
  配置目录与 `NookSill.ini`、便携标记 `NookSill.portable`、内置条目 scheme
  `nooksill:`、更新器常量 `MCheng404/NookSill-Rust`。
- 环境变量前缀 `MINIDOCK_*` → `NOOKSILL_*`（`NOOKSILL_DEBUG_OPEN` 等调试开关）。
- 旧名字只保留在 `PORTING.md` / `DECISIONS.md` 指代**上游 C 项目**的地方
  （`minidock.c`、上游发行包 `MiniDock-1.x.zip`）—— 那是历史事实，不改。

**破坏性**：这是一次身份变更，不复用旧路径 —— 配置目录、ini 名、互斥体、
URI scheme 全换，旧版本配置不会被继承，两个版本也不会抢同一个单实例。
GitHub 上旧的 Release 与标签一并清空，从 2.0.0 重新开始。

## 1.1.0 — 应用内更新（2026-11-06）

- **Debug 命名规则**：`X.Y.Z-<第几次构建>-<提交号>`（如 `1.1.0-66-fe678af`），
  三段分别是稳定版前缀、本次构建序号、构建所对应的提交；Release 仍是 `X.Y.Z`。
- **应用内更新**：设置 →「关于」新增更新卡片，检查 GitHub Release → 选镜像 →
  下载 → 校验 SHA-256 → 提示重启替换。网络全在后台线程，UI 不出帧等它。
- **镜像**：默认走 GitHub 反代（gh-proxy / ghfast / ghproxy.net / gh.llkk.cc /
  mirror.houlang.cloud），下载前对候选做一次真实 HTTP 测速（DNS+TLS+首字节），
  够快就先上并停止测速；可切换成「全部测完再选」。
- **安全 DNS（默认开）**：用 Cloudflare DoH 复核系统解析结果，判定被污染时
  不再走 GitHub 直连，只用镜像。WinHTTP 的 TLS 校验绑域名，DoH 拿到的 IP
  没法直接拿来建连，所以这里是**判据**而不是传输通道，理由写在 ADR-071。
- **隐藏高级入口**：连点「关于」页版本号 5 次才出现「高级」卡片；设置窗口
  一关就收回，下次进来要重新点。
- Debug 预发布正文改成英文并附构建时间与 SHA-256（Release 正文仍从本文件抽）。

## 1.0.0 — 首个正式版（2026-11-06）

第一个对外发布的稳定版。范围：Dock 本体（图标、悬停放大、拖拽重排、文件夹、
分割线与分组、运行指示、倒影）、小组件条带（时钟与日历、资源监视器、计时器
与待办、控制中心、系统托盘、专注模式）、起动台，以及完整设置界面（11 页）。
几何与键表以 `PORTING.md` 记录的上游 v1.6.5 为基准。

- 渲染：wgpu / DX12 离屏 + `UpdateLayeredWindow`，空闲时不出帧。
- 体积：单文件 exe（静态 CRT），release 约 7.4 MB，不依赖 VC 运行库。
- 分发：`NookSill-1.0.0-win64.exe` + 同名 `.sha256`，**裸文件不打包**。
- 尚未移植：顶部状态栏、窗口动画重做、多窗口应用模式；缺口与理由见
  `PORTING.md` 第九节。

从这一版起版本号以 `Cargo.toml` 为唯一来源：Debug 是 `X.Y.Z-dev.<构建编号>`，
Release 是 `X.Y.Z`；推 `v*` 标签触发正式发布，正文从本文件对应小节抽。

## 0.9.27 — 图标主题色独立开关 / 圆润重绘 / 动态图标基础（2026-11-06）

### 主题色：两个独立开关

- 新增 `UiAccentDockIcons`（Dock 图标）与 `UiAccentCcIcons`（控制中心
  图标），**默认开**，外观页「主题色」卡各一行开关。与已有的
  「同步到 Dock 栏体」彼此独立：关掉栏体同步不影响图标，反之亦然。
- 运行时值 `Config::runtime_icon_accent`（不写 ini）：主循环每帧在两个
  开关任一开启时解析一次 `accent::resolve`，Dock 与控制中心共用，各自
  再按自己的开关决定上色与否（`CcWindow::set_icon_accent`）。关闭时
  分别回落默认蓝 / 中性前景色。单测守住两开关互不牵连。

### 重绘：圆润、连续、统一尺寸

按用户建议查了 Fluent 2 / Lucide / Material 图标规范，落地三条：

- **20px 安全区**：24px 画布内容占 20px（`SAFE = 0.40`），方形类再做
  光学补偿（`BOX = 0.34`）；Wifi / Hotspot 的扇形天生只占下半格，
  额外放大 1.15~1.18 倍才与其它图标看着一样大。
- **线条连续**：`Canvas::polyline` 用一整条 `Shape::line` + 每个顶点
  补**一次**圆头（round join / cap）。旧版逐段画线段、每段两端各补圆，
  拐点会叠出结节、端点接不上；闭合路径首尾同点还会重复补圆，现去重。
- **没有锐角**：填充块只用圆角矩形 / 圆，删掉三角喇叭、梯形齿轮、
  方形九宫格；喇叭改闭合圆角折线、齿轮改「外环 + 8 圆齿 + 轴孔」、
  Launchpad 改圆点、蓝牙/闪电/水滴全部走圆头折线。

### 动态图标基础

`IconState { level, active, phase, muted }` + `IconStyle`（颜色/主题色/
状态）统一所有公开入口，绘制端不持有时间。已接通的状态：

| 状态 | 效果 |
|---|---|
| `level` | Wifi 三段信号、喇叭两道声波、电池电量条、Launchpad 九点逐个点亮 |
| `phase` | 齿轮齿微转、热点涟漪外扩 |
| `muted` / `active` | 喇叭叉 / 磁贴开关态 |

以后加动画只需调用方推进 `phase`，不用改绘制函数签名或绘制代码。

### 验证

`examples/icon_sheet` 扩到 4 行（新增 level / phase 组与
「accent off / on」并排对比），浅色蓝主题、深色绿主题两张逐项核对；
`gpu` / `core` 全部测试通过，门禁全绿。

---

## 0.9.26 — Dock 与控制中心图标统一重构（2026-11-06）

用户："优化重构调整应用程序组件部分中出现的所有图标"（澄清后范围 =
Dock 栏图标 + 控制中心图标）。

### 问题

两处图标原先各画各的、两套语言：Dock 内置条目是写死颜色的填充色块
（蓝底四格窗、九个高饱和原色）、控制中心是写死像素圆角的线稿；首字
占位是脏灰底 + 暗红字；蓝牙画成 ▶、设置画成太阳/雪花。缩放（Dock
悬停放大）与 DPI 一变，写死像素就错位。

### 重构：统一矢量图标中心 `gpu::glyph`

- **归一化画布 `Canvas`**：所有坐标按「以图标中心为原点、边长 `s`」
  归一化（-0.5..0.5），线宽统一 `s*0.085`（下限 1.1px）、圆角按 `s`
  取比例，线段/圆弧都带圆头 —— 线稿不再有 butt 端点的毛刺感。
- **颜色全由调用方给**：Dock 内置条目底色/首字跟 `Config::runtime_accent`
  （关掉同步回落默认蓝），控制中心磁贴跟前后景/开关态。
- 控制中心 `cc_win.rs` 删掉本地 `glyph`/`speaker`/`arc`（约 170 行），
  9 个 `Tile` 图标全部在共享模块重画；蓝牙几何取自 simple-icons
  `bluetooth.svg`（竖线 + 右侧菱形 + 左侧斜线），设置改成真齿轮
  （外环 + 8 个梯形齿 + 轴孔），节能改成电池 + 闭合闪电。
- Dock：Start 跟主题色、Launchpad 九色改低饱和并随主题换底、圆角/间距
  全部按 `s`；首字占位换成中性 tonal 底 + 主题色字母 + 微描边。
- `sliders()` 条带图标改成「轨道 + 圆钮」，与设置界面滑条同一语言。

### 验证

新增 `examples/icon_sheet`：离屏把 20 个图标（含 24px 缩放组、开关态、
深浅两套主题、自定义 accent）排成总览图。两张图逐项核对形状、圆头、
开关态白图标、缩放一致性；`gpu` 99 条测试全过，门禁全绿。

> 插曲：临时 PNG 转换脚本把 BMP 的 BGR 字节直接当 RGB，看图时一度所有
> 蓝变橙、粉变紫；BMP 数值本身一直正确（Start 底色精确 `(46,123,246)`）。
> 用 PIL 直接读 BMP 即可，别手写字节序。

---

## 0.9.25 — 构建机制、版本号与 CI（2026-11-06）

- 版本号统一到 `scripts/version.rs`，两个 `build.rs` `include!` 同一份逻辑：
  Debug 是 `X.Y.Z-dev.<构建编号>`，Release 是 `X.Y.Z`。构建编号取 `NOOKSILL_BUILD`
  （CI 传 run number），本地回落 `git rev-list --count HEAD`。
- 三处分发同源：`nooksill_core::VERSION`（关于页）、exe 的 VERSIONINFO（属性页）、
  `target/<triple>/<profile>/NOOKSILL_VERSION`（`package.sh` 的产物名）。
  `package.sh` 只读那个文件，不重新推导。
- workspace 版本 `0.1.0` → `0.9.25`，与 CHANGELOG 对齐；删掉名不副实的
  `APP_VERSION = "1.2.0"`（从未被引用过）。
- 新增 `scripts/package.sh`：产出 `dist/NookSill-<ver>-win64[-debug]/`
  （exe + LICENSE + README）；`--pdb` 才带调试符号（debug 的 PDB 有 200 MB 量级）。
  `./scripts/build.sh --version` 打印本次构建钉住的版本。
- 新增 `.github/workflows/build.yml` 三段式：`gate`（每次 push / PR）、
  `debug`（任意分支的提交都出包）、`release`（`v*` 标签，先校验标签与
  `Cargo.toml` 一致，再打包发正式 Release，正文从本文件对应小节抽）。
  - debug 有两个出口：Actions 产物（无条件，14 天）和 **rolling Pre-Release**
    （默认分支上发，tag 固定为 `dev`，标题带版本，资产每次覆盖）。
  - 想只留 Actions（测试流程）：仓库变量 `DEBUG_PUBLISH=artifact`，或手动
    触发时把 `publish` 选成 `artifact`。
  - release 显式 `prerelease: false` + `make_latest: true`，与 Pre-Release
    在 Releases 页上区分开。
- exe 资源补 `LegalCopyright` / `OriginalFilename` / `InternalName`；
  FileVersion 与 ProductVersion 同时写数值与字符串，属性页不会再吞掉 dev 后缀。

## 0.9.24 — 设置新增「关于」页（2026-11-06）

- 新页 `about.rs`：版本 / 构建、项目地址、作者主页、许可（GNU GPL v3.0 or
  Later）。项目与作者两行整行可点，右侧分别绘制 Invertocat 与作者头像。
- 图形是矢量不是位图：GitHub 标志用 simple-icons `github.svg` 的路径**离线**
  切成 162 个三角形（egui 只填充凸多边形），数据放 `about_art.rs`，坐标归一化
  到 0..1，按主题黑白反转；头像暂为字母徽标（无位图资源）。
- 修既有 bug：`widgets::button` 用白色排版 galley、绘制时又传 `text_color()`，
  但 `Painter::galley` 只覆盖 `PLACEHOLDER`，于是浅色主题下**所有按钮**都是
  "白字浅灰底"（预设 / 添加 / 重置页都有）。改用 `Color32::PLACEHOLDER`。
- 许可：`Cargo.toml` 的 `LicenseRef-Proprietary-Upstream` 改为
  `GPL-3.0-or-later`，补 `LICENSE` 全文，README 增加 License 段。

## 0.9.23 — 修崩溃：全屏起动台开不出来（2026-11-06）

- **2x MSAA 触发 wgpu 校验错误**。设备没声明
  `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES` 时，WebGPU 只保证
  `Rgba8UnormSrgb` 支持 [1, 4] 采样，而我们按适配器报告的 [1,2,4,8] 选了 2，
  `create_texture` 直接 panic —— 任何面积 > 2 M 像素的窗口都会中招，起动台
  是全屏所以必现（与设置窗口是否打开无关）。现在：适配器支持就请求该特性；
  `perf::resolve_samples` 以设备实际能力为准，不可用时向上取 4（不悄悄砍
  抗锯齿）。ADR-067 补记。
- **烟雾字段缓冲写越界**。`SmokeShader::raster` 的缓冲只在首帧创建，而网格
  按栏体宽度重新分档（256×128 → 384×128）后 `write_buffer` 越界报错。改为
  容量不足即重建。这个 bug 原本被上面的崩溃挡在后面。
- 验证：`NOOKSILL_DEBUG_OPEN=launchpad` 与「设置窗口 + 起动台同开」各跑
  12 秒无 panic（改前必崩）。

## 0.9.22 — 同步上游 1.7.x 的三处行为（2026-11-06）

- 节能磁贴读**真实**电源方案（`PowerGetActiveScheme`），打开面板时回读；
  记住进节能前的方案，退出时还原而不是一律切回「平衡」；切换改用
  `PowerSetActiveScheme`，不再 spawn `powercfg`（ADR-069）。
- 有线网络磁贴改开 `ncpa.cpl`（网卡列表），不再落到 Wi‑Fi 状态页。
- 任务栏在同一侧且自动隐藏时，既不登记 AppBar 预留也按显示器物理边界
  定位，避免 Dock 离屏幕边缘空出一条任务栏高度（上游 1.7.3 同源问题）。
  新增 `appbar::taskbar()` / `should_reserve()` / `taskbar_steals_edge()`。
- 台账新增 [第九节](PORTING.md)：上游 1.7.3–1.7.6 的键表 diff（118 → 153，
  缺 35 个键，已确认升级往返不丢），以及明确不跟进的三块（窗口动画重做、
  任务栏接管、顶部状态栏）。

## 0.9.21 — 呈现换序 SIMD 化（ADR-068 后续，2026-11-06）

- `present::swap_rb_strided` 加 AVX2 路径：一条 `vpshufb` 处理 8 个像素，
  标量实现保留为参考与非 AVX2 回落。宽度不足 8 像素走标量，行尾用
  重叠的整向量写入，不留标量尾巴。
- 实测（release，1000 次，p50）：`upload_rgba_strided` 1320×1080
  0.79 → 0.09 ms（8.7x）、2560×1440 2.04 → 0.44–0.60 ms、391×315
  0.08 → 0.007 ms。呈现路径总成本在 Dock 尺寸降到约 0.2 ms。
- 一致性：新增 `simd_path_matches_scalar_for_every_width`（1–40 px 与
  391 / 1320 px，覆盖行尾、重叠写入与 < 8 px 回落）；整页离屏渲染
  SIMD 版与标量版 **0 字节差异**。

## 0.9.20 — 帧率设置、延迟与内存（ADR-067，2026-11-06）

- 性能页新增「帧率与同步」卡片：`FrameCap`（0–240，默认 0 = 不限）、
  `VSync`（按 DWM 刷新率对齐）、`Msaa`（自动 / 关 / 2x / 4x，下次启动生效）。
  三键进 `[performance]` 段，`CONFIG_VERSION` 升到 3。
- 取消硬编码帧率限制：删除烟雾 `SMOKE_FRAME` 33 ms 节流及 `next_smoke`；
  动画分支 16 ms 上限改为统一节拍；设置窗口 `Duration::ZERO` 的 16 ms → 1 ms。
- 降低绘制延迟：设置窗口 `repaint_in()` 并入主循环等待（消除最长 1000 ms
  的兜底停顿）；面板轮询 24 → 8 ms。
- 内存：`msyh.ttc`（8.04 MiB）改为全进程共享，8 窗口时约省 56 MiB；
  MSAA 大窗口 4x → 2x，2560×1600 约省 31 MiB。
- 验证：`perf` 节拍 / MSAA 纯函数单测、字体共享单测、DWM 真机测试
  （实测 16 ms）通过；门禁 383 条测试全绿。
- 保留：空闲态 `FS_CHECK_MS` 长睡不变（「空闲 0 帧」承诺）。

---

## 0.9.19 — 统一 GPU shader 框架（ADR-066，2026-11-06）

- 新建 `gpu/shaders/`：`ShaderRuntime` 统一 compute pipeline 缓存、
  `Rgba8Unorm` 输出纹理、egui native texture 注册与回读；WGSL 分文件。
  frost 从 `blur_gpu.rs` 迁入 `shaders/frost.rs` + `frost.wgsl`。
- 烟雾光栅化迁 GPU：`smoke.wgsl` 一趟完成 raster + post（双线性、三层
  噪声、调色、预乘、第二色、增益），输出纹理直接注册给 egui，
  **每帧约 1.2 MB 位图上传消失**；CPU 只留 `Fluid::step` 与三个场。
- 图标圆角遮罩（`icon.wgsl`）、烟团贴图（`puff.wgsl`）同样迁 GPU，
  CPU 版本留作回退与像素级参考。
- 图标线程允许提交 wgpu compute；线程不变式收敛为「不碰 `egui::Context`」。
- 质量：6 条 GPU/CPU 逐通道对照 + 1 条端到端出帧真机测试全部通过。
- 门禁：fmt / `clippy -D warnings` / 380 条测试全绿。
- 边界：`Fluid::step`、`Noise` 表、bbox 几何、GDI 格式重排、`swap_rb`
  呈现桥接保留 CPU，判据见 ARCHITECTURE 第三节。

---

## 0.9.18 — IO 与渲染效率（ADR-065，2026-11-06）

- frost 模糊迁到 WGSL compute（`src/blur_gpu.rs`）：两趟可分离高斯 +
  圆角 coverage，输出 `Rgba8Unorm` 直接注册给 egui，**零回读**。
  capture 线程只抓屏，字节用 `Arc` 共享给 UI 线程，不再做 CPU 卷积。
- `blur_rgba` / `compose` 保留为回退路径与像素级对照基准。
- 主呈现路径合并回读与 DIB 写入（`with_mapped_pixels` +
  `upload_rgba_strided`）：每帧少一次 1–6 MB 堆分配与整帧拷贝。
  `draw_frame` 抽出后 `render` / `render_rgba` 仍共用一份绘制逻辑。
- 实测（debug，同机）：
  - 640×400 / r=12：CPU blur **242.25 ms** → GPU 提交 **1.78 ms/帧**，
    10 帧排空 30.66 ms。
  - 1320×1080 呈现：**6.78 → 6.01 ms/帧**（1.13x），逐像素一致。
- 质量不变：两条真机测试（compute 输出 vs CPU 参考、egui 出帧 vs CPU
  出帧）逐通道差 ≤1，均通过。
- 门禁全绿；Dock 整体窗口路径本轮未真机运行。

---

## 0.9.17 — 依赖更新与遗留写法清理（ADR-064，2026-11-06）

- `cargo update`：12 个传递依赖到约束内最新（js-sys / web-sys 0.3.107、
  wasm-bindgen 0.2.130、toml 1.2.0、log 0.4.35 等）。直接依赖均已是
  crates.io 最新版；`glass-backdrop` 上游 HEAD 与固定 rev 一致。
- `rust-version` 1.88 → 1.95：egui 0.37 / egui-wgpu 0.37 要求 1.95，
  原声明低于依赖下限。
- 删死代码：`tool_win::_unused`、`launchpad_win::client_size`（含无用 import）。
- 删多余豁免：`resmon_win::ppp` 的 `#[allow(dead_code)]`（字段在用）。
  7 处 `#[allow(clippy::too_many_arguments)]` 改带 reason 的 `#[expect]`；
  `bin/dock.rs` 一处原本挂在 3 参数的 `frost_spec` 上，连同错位的文档注释
  一并修正到 `show_menu`。
- `probe.rs` 两处必须保留的字段豁免改 `#[expect]` 并写明原因。
- 本轮触及的代码注释改为纯英文、简短；Cargo.toml 注释同步。

验证：`fmt` / `clippy -D warnings` / 379 条测试全绿；两条 `--ignored`
真机测试通过。Dock 整体窗口路径本轮未真机运行。

---

## 0.9.16 — 第二轮依赖库官方 API 审查（ADR-063，2026-11-06）

接着 0.9.11 的 egui 审查，把 `windows 0.62.2` / `wgpu 30` / Win32 GDI 的
用法逐条对回官方文档与本地依赖源码。没有新功能，四处改动都有可核查的依据。

### 修：ADR-029 的前提是错的 —— 删掉手写 COM vtable 调用

`platform::shortcut` 原来绕过 wrapper 直接调 `IShellLinkW::GetPath` 的
vtable，理由是"生成的 wrapper 签名与 COM ABI 不符"。复核 `windows-0.62.2`
的生成源码：

```rust
pub unsafe fn GetPath(&self, pszfile: &mut [u16],
                      pfd: *mut WIN32_FIND_DATAW, fflags: u32) -> Result<()>
pub unsafe fn GetArguments(&self, pszargs: &mut [u16]) -> Result<()>
```

与 Microsoft Learn 的 `GetPath(LPWSTR, int, WIN32_FIND_DATAW*, DWORD)` 完全
一致。ADR-029 里记的 `(&self, flags) -> Result<PWSTR>` 是同一个 Shell 命名
空间下 **`IKnownFolder::GetPath`**（`mod.rs:27649`），另一个接口。现在只调
官方 wrapper，`Interface::vtable` / `PWSTR` 那套手工 ABI 代码整段删除，
升级 `windows` 不再需要核对 vtable 布局。

- 新增 `wide_to_string` 纯函数 + 3 条单测（首个 NUL 截断、空/无 NUL）。
- 保留 `#[ignore]` 的真机解析测试作为 COM 通路验收。

### 修：图标第三层兜底把图标画到了屏幕上

`platform::icon::draw_into_dib` 创建 32 位 DIB 后，直接把 `GetDC(NULL)` 的
**屏幕 DC** 交给 `DrawIconEx`，从未 `SelectObject` 把 DIB 选进 DC。结果是
图标画到屏幕左上角、DIB 恒全 0、`has_visible_pixels()` 永远为假 ——
第三层兜底从未生效。前两层能覆盖多数现代图标，所以一直没暴露。

- 按 Microsoft Learn `CreateDIBSection` 的 Remarks 补上：内存 DC →
  `SelectObject(dib)` → `DrawIconEx` → `GdiFlush` → 恢复旧对象 → `DeleteDC`。
- 所有可能提前 `?` 的计算挪到创建 DIB **之前**，避免漏掉 `DeleteObject`。
- BGRA→RGBA 的两份相同循环抽成 `bgra_to_rgba` 并补单测。
- 新增 `#[ignore]` 真机测试：加载系统图标断言 DIB 里真的有像素。

### 加固：wgpu 回读的 map/unmap 生命周期

`offscreen::read_pixels` 依据 wgpu 30 官方文档整理：

- `PollType::Wait` 的语义是"提交完成**且回调已调用**"，所以 poll 返回后
  用 `try_recv` 取结果即可；原来的 `recv()` 在回调没投递时会让 UI 线程
  永久挂死。
- `Buffer::unmap` 之前不能再 `map_async`。原来 `get_mapped_range()` 失败
  时直接 `?` 返回，缓冲区留在 mapped 状态，之后每帧回读都会失败。
  现在把取像素包进闭包，**所有**路径都走到 `unmap`。
- 行去对齐抽成纯函数 `unpad_rows` + 4 条单测（有/无填充、坏几何、零高度）。

### 加固：core::layout 不再有生产 `expect`

`WidgetStrip::left_of` 的两处 `items[j].expect("len 之内必有值")` 改成
`Option::?`：条带字段是 `pub`，手工构造或将来改填充
规则都可能留下空洞，坏数据应当退化为"定位失败"而不是让 UI 线程 panic。
新增 `sparse_widget_strip_does_not_panic` 覆盖左右两侧与越界下标。

### 文档

`BUILD.md` 的依赖表与产物表还停在 M0（列着已删除的 `crossbeam-channel` /
`arc-swap` / `raw-window-handle`、`nooksill-spike.exe` / `NookSill.exe`），
`README.md` 的启动命令指向已删除的 spike、hooks 配置命令少一个空格，
`ARCHITECTURE.md` 第六节仍写着 `ArcSwap` + GPU compute 模糊。全部对齐实现
（`ARCHITECTURE.md` 顺带按 `.gitattributes` 的 `eol=lf` 规范回 LF）。

### 门禁与验证

- `fmt --check` / `clippy -D warnings` / `cargo test --workspace`：全绿，
  379 条测试通过（core 228 / gpu 93 / platform 37 / settings 21）。
- **真机验证**（桌面会话，显式 `--ignored`，两条都过）：
  - `resolves_a_real_start_menu_shortcut` →
    `Access.lnk -> C:\Program Files\Microsoft Office\root\Office16\MSACCESS.EXE`，
    证明官方 wrapper 的 COM 通路可用，不需要手写 vtable。
  - `dib_fallback_draws_into_the_dib_not_the_screen` → 通过；修复前 DIB
    恒全 0，这条测试必然拿到 `None`。
- egui-wgpu 的 `textures_delta` 顺序核对后**不改**：官方 `winit.rs` 同样是
  先 `set` 上传、在 submit 之后才 `free`，与 `frame.rs` 一致。
- 顺手修 `scripts/build.sh --exe`：它还打印着 ADR-035 已删除的
  `NookSill.exe`，改成 `nooksill-dock.exe`。
- **未验证**：Dock 整体（透明窗口、命中、出帧）本轮没有真机运行；这轮改动
  只覆盖纯逻辑、快捷方式解析与取图标路径。

---

## 0.9.15 — 烟雾几何重构：跟着栏体走、网格不再抖（ADR-062，2026-11-05）

### 改：`Fluid` 的位置改由「栏体矩形」直接给定

`ensure()` 原先用 `(w, h, margin)` 描述场景：网格 = 栏体 + 四边 margin。
掩码、染料源区都从 margin 反推栏体在哪。这套约定有两个硬伤：

- **网格尺寸随栏体变**：悬停放大时栏体每帧在长，`ensure()` 每帧判定
  "尺寸变了"并重建 —— 速度场、染料全部清零，画面上就是烟在闪。
  实测 `[smoke] grid=` 在放大过程中一路从 300x49 跳到 341x49。
- **一个网格只能有一个居中矩形**：想覆盖小组件条带就得改这套约定。

现在 `ensure(w, h, seed, bar, thickness)`：`bar` 是**格坐标**的矩形，
身份判定只认 `(w, h, seed)`，掩码每次重算（一万多格距离场约 0.1 ms）。

### 改：网格取整窗 + 分档，避免每帧重建

`w/h` 从"栏体 + margin"改成整个窗口，再按 `GRID_BUCKET`（128 格 = 640 px）
向上取整 —— 悬停时窗口自己会从 1496 px 变到 1720 px，不分档仍会跨档重建。
实测放大动画全程 `grid=384x128` 恒定。

### 改：光晕覆盖整条停靠带

`SmokeView.bar` 从 `bar_phys()`（图标条）换成 `hot_phys()`（图标条 ∪ 小组件
条带）。光标判定、遮挡判定用的也是这条并集，三处终于同一份几何。

### 代价

位图从"栏体周边"扩到整窗（960×320），但 `raster` 对未点亮的格子会提前
`continue`，实测帧率仍是 21–26 fps，与改动前持平。

---

## 0.9.14 — 收起后不再出帧（ADR-061，2026-11-05）

### 修：Dock 完全收起后仍在持续渲染

主循环把**任何**消息都当成"要出帧"：`pump()` 里只要 `PeekMessage` 取到过
东西就返回 `Frames`。后台线程的统一唤醒 `WM_APP_WAKE` 也在其中 —— 智能
隐藏下扫描线程每 100 ms 发一条（遮挡检测周期），于是 Dock 早已滑出屏幕，
主循环还在每秒画 10 帧。

改法两条，缺一不可：

1. `pump()` 区分"真消息"与"唤醒"：`WM_APP_WAKE` 只打断等待，出不出帧交给
   各步的脏标记 —— 这正是 `worker.rs` 注释里写明的意图，实现漏了。
2. 完全收起（`Dock::fully_hidden()`，`hide_cur > 0.999`）时**一帧都不画**：
   画出来的东西全在窗口外。烟雾粒子同步停摆。
3. `wait()` 的烟雾节拍也要跟着停：`step_smoke` 返回 false 时**不推进**
   `next_smoke`，等待循环算出来 `left=0` → `timeout = 1 ms` → 主循环在收起
   状态下按 ~1000 Hz 空转。这条比出帧更费 CPU，而且看不出来。

实测（`NOOKSILL_FPS=1`，自动隐藏 + 光标移开）：收起后 8 秒内日志里没有
任何 `[fps]` 行 = 0 帧；进程 CPU 0.26% 单核（只剩 100 ms 一次的光标轮询）。

### 顺带

- 新增 `NOOKSILL_FPS=1`：每秒打一行 `[fps] N 帧/秒（hidden=… anim=…）`。
  "空闲 0 帧"这个承诺从此可以验收，不必靠读代码相信。
- 顺手验证了上一轮的遮挡矩形：最大化 Edge 覆盖停靠带时日志给出
  `covered=true by "…Edge Beta"`，点名正确。

---

## 0.9.13 — 显示判定收敛到一条热区 + GUI 子系统（ADR-060，2026-11-05）

### 修：自动隐藏 / 智能隐藏在光标附近反复显示隐藏

根因是"靠近"判定按状态用了**两套矩形**：收起态认"贴屏幕边 + 落在窗口
宽度内"，展开态只认 `bar_l..bar_l+bar_w`（图标条）。小组件条带（时钟 /
控制中心 / 资源 / 计时器 / 托盘）在图标条右侧，于是 ——

- 光标挪到时钟上 → 展开态判"不靠近" → 450 ms 后收起；
- 收起后光标仍在窗口宽度内且贴着屏幕边 → 收起态判"靠近" → 立刻弹出。

实测日志 `target=false near=true` / `target=true near=false` 每 450 ms
交替一次，`hide_cur` 连动画都走不完（0.250 ↔ 0.188）。

改法：

- `Dock::hot_phys()`：整条停靠带（图标条 ∪ 小组件条带，含 6 px 视觉缝与
  投影留白）算成一个矩形，**窗口物理坐标**。
- `layout::near_dock()`：纯函数，两态共用同一套几何，只差容差 —— 展开时
  多 32 px 回滞（施密特触发器），光标在边界上慢慢挪不再抽搐。
- `cursor_near` 只剩取坐标；判定本身进单元测试（底/上/左三种停靠 + 回滞）。

实测（144 dpi，光标钉在时钟上）：`[near] true cursor=(1200,1520)`，不再翻转。

### 修：面板 / 设置 / 起动台开着时 Dock 自己收起

`busy` 原先只有"拖动中"这一项。现在加上面板开着、设置窗口开着、起动台
开着 —— 光标进了面板就等于离开了 Dock，收起只会让面板凭空悬在半空。

### 修：双击 exe 多一个命令行窗口

产物一直是 console 子系统（PE `Subsystem=3`）。加
`#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`：
release 是 GUI 子系统（实测 `Subsystem=2`），debug 保留控制台看 panic。
从终端启动仍继承父进程的 stdout/stderr，诊断输出不受影响。

### 顺带

- 遮挡矩形改用同一条热区：只盖住时钟的窗口现在也算"挡住"。
- `[hide]` 与新增的 `[near]` 诊断行带上光标与热区坐标 —— "判错了"和
  "坐标根本没到"是两回事，日志要能分开。

---

## 0.9.12 — 毛玻璃改成实时背景（ADR-058，2026-11-05）

上一版开窗抓一次就再也不动，桌面在播视频时面板背后糊着的是一张静止的
截图。这轮把它做成**活的**。

### 实现

- `frost::Live`：面板可见期间起一条 `nooksill-frost` 线程，按 100 ms 节拍
  抓屏 → 与上一帧逐字节比较 → 变了才模糊并回传。UI 线程在每次出帧前
  `poll_frost()` 取最新一帧，`TextureHandle::set` 原地改写纹理（不是每帧
  重建一张）。
- 变化才产出 → 静止桌面**完全不重绘**；连续抓不到变化时间隔翻倍退避到
  800 ms，静态时的抓屏开销从 20% 掉到 2% 量级。发现变化立刻回到 100 ms。
- 面板显示前 `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`，隐藏时
  恢复。抓屏线程因此读到的是窗口**背后**的真实内容，而不是面板自己
  （分层窗口实测同样生效，见 ADR-058 的验证记录）。
- 三处都接上了：起动台、设置窗口、右键菜单。新帧到达经 `WM_APP_WAKE`
  唤醒各自的消息循环重绘。设置窗口换缩放会重建渲染器，那里补了
  "重新同步抓一次 + 重启线程"。

### 性能

| 场景 | 抓屏 | 模糊 | 合计 |
|---|---|---|---|
| 全屏起动台（2560×1600→640×400） | 21 ms | 20 ms | ~35 ms/帧 |
| 设置窗口（391×315） | 8 ms | 12 ms | 17–30 ms/帧 |

`blur_rgba` 重写成**平面 f32 + 转置写**：交错 RGBA 里同一通道隔 4 字节，
卷积内层没法向量化；拆平面后内层是连续切片点积，第二趟直接按转置写，
省掉一次显式转置。640×400/r=12 从 40 ms 降到 20 ms。
圆角遮罩只在四个角算 SDF（半径以外覆盖率恒 255）。

实测（2560×1600 / 10 Hz / 背后是正在播放的视频）：整进程约 30% 单核。

### 取舍

- 面板**可见且开着毛玻璃**时会被排除出屏幕捕获：截屏、录屏、共享屏幕里
  都看不到它。不排除就只能抓到自己，实时模糊绕不开这一条。把对应的
  `LpFrost` / `UiFrost` / `MenuFrost` 关掉即恢复原样。
- 模糊核与上一版**逐像素一致**（新增一条与朴素二维卷积对拍的测试，
  ±1 以内），观感没有变化。

### 测试

`frost` 9 条：新增「可分离 == 朴素二维卷积」「只算四角的 alpha == 整幅扫描」
「非方图不会把横竖搞混」，其余沿用。另加 `NOOKSILL_FROST_TRACE=1`：
每帧打一行尺寸 / 平均色 / 变化像素数 / 耗时 —— 背后放一个闪色窗口即可
确认"确实在实时重抓、且没有抓到面板自己"。

---

## 0.9.13 — 毛玻璃层次：侧栏/卡片半透明化 + 截图通道修复（2026-11-05）

### 毛玻璃割裂修复

窗口底在毛玻璃开启时 alpha=140（模糊桌面透上来），但侧栏与卡片是
不透明纯色 —— 毛玻璃被整块挡死，看起来像"贴上去的实心板"，与透毛
玻璃的内容缝隙严重割裂。

- **侧栏 / 卡片改为半透明叠加层**（Fluent acrylic tonal 层次，壁纸
  透过率：内容缝隙 55% > 侧栏 24% > 卡片 13% —— 底最透、卡片最实、
  可读性优先）：LIGHT/DARK_SIDEBAR alpha 120/130，LIGHT/DARK_CARD
  alpha 170/150；说明气泡等浮层（MENU）保持不透明。
- **毛玻璃关闭时同一套配色成立**：窗口底 alpha=250 近实色，叠加后
  整体 ≈ 不透明，不需要按 frost 切换两套色值。
- **补 1px 微描边**：半透明边界会融进底色，卡片与侧栏各加 Fluent 式
  subtle border（浅 rgba(23,26,35,16/14)、深 rgba(255,255,255,20/18)），
  投影继续负责"浮起来"，描边负责"边界在哪"。
- 合成验证：epaint 走 **sRGB 目标，混合在线性空间** —— 半透明白叠
  亮底比 gamma 手算暗得多（251@0.47 叠 246 → 212 而非 248），像素
  实测与线性合成公式精确一致，管线正确；壁纸透过率层次才是设计目标。

### 截图工具：红蓝通道互换（第一天就在的 bug）

真窗口路径 `upload_rgba` 用 `swap_rb` 把 wgpu 回读（RGBA）转成 DIB
要的 BGRA；`write_bmp` 却把回读缓冲直接当 BGRA 取 —— **所有离屏截图
的红蓝通道从第一天起就是反的**（冷色调配色全部输出成暖色）。之前
的验证都是相对比较（前后 diff、找色块），通道对称所以从未暴露。
已按 RGBA 取值修复；今后截图的绝对色值可信。

---

## 0.9.12 — 控件重构：胶囊开关 / Fluent 滑条 / 自绘按钮（2026-11-04）

设置页三个最"工具味"的元素整体重构，视觉基准 Windows 11 Fluent，
色板与 [`text_field`] 完全同源（field 半透明叠层 / accent = selection）。

### 胶囊开关（替换裸 Checkbox）

- 之前 `toggle` / `toggle_derived` 全是 `egui::Checkbox` 空方框打勾 ——
  勾选框表达"状态"，开关表达"模式"，观感也是全页最旧的。
- 新 `switch_widget`：40×22 全胶囊（圆角 = 半高），**开 = 主题色填充 +
  白色圆钮靠右，关 = field 灰底 + 描边 + 圆钮靠左**；圆钮位置走 egui
  布尔动画（0.12s 滑动），悬停叠 tonal 层，聚焦画主题色外圈。
- 键盘可达：`Sense::CLICK | focusable_noninteractive()`，聚焦后空格 /
  回车切换；`Role::Switch` 自报可访问性。**accessible name 交给
  labelled_by 关联的中文标签** —— widget_info 自报键名会把标签覆盖掉，
  kittest / 读屏按名字查就找不到了（点击开关写入配置测试当场抓住）。

### Fluent 滑条（替换裸 Slider）

- egui 原生 Slider 的轨道和圆钮共用 `widgets.inactive.bg_fill`，配不出
  "灰轨道 + 白圆钮 + 描边"，圆钮描边完全改不了 —— 自绘。
- 5px 粗圆角轨道（命中区 ±9px 比轨道大一圈，窄轨道好点）、实心主题色
  已填充段、白色圆钮带 2px 主题色描边（悬停/拖动放大到 9）；
  拖动时圆钮上方浮出数值气泡，右侧常驻弱色数值（拖动中变主题色）。
- 键盘：←/→ 按（全程 20 步）步进，Home/End 归零打满；点击/拖动都是
  指针 x → 值映射。ASCI 网格核验：填充段 / 白钮 / 描边 / 灰轨道逐项正确。

### 自绘按钮（替换裸 ui.button）

- 30px 高、圆角 10、宽度随文字（±22 padding、最小 72）；
  三态 = field 底叠 0/1/2 层 hover tonal —— 半透明叠层在卡片白上形成
  渐进台阶，不用为深浅主题各配实色；聚焦画主题色细描边；
  空格/回车可触发（触发收敛到一个分支 —— `Command` 不是 Copy）。

### 测试

设置 crate 16 条全过（`控件右边缘对齐` 的角色查询随 Switch 更新）。

---

## 0.9.11 — 代码审查：跟进 egui 0.37 官方 API（2026-11-04）

系统审查了弃用豁免、自绘控件、样式与 tessellation 用法，对照
docs.rs 0.37 文档与本地 epaint/egui 源码验证每项是否真有收益。

### 采纳（都有实测依据）

- **`Id::new` → `Id::unique`（6 处）**：官方弃用说明明确 `Id::new`
  就是 `Self::unique(source)` 的别名，行为完全等价；清掉全部
  `#[expect(deprecated)]` 豁免，避免以后 egui 升级时被静默移除。
- **主题切换改用官方 `set_theme`**：原来 `apply_theme` 用
  `set_visuals(Visuals::dark/light())` 覆盖当前主题的样式，而截图工具
  用 `set_theme` —— preference 与 style 各说各话，是主题类 bug 的温床。
  `set_theme` 只改一个 `ThemePreference` 字段，比重建整套 `Visuals` 轻。
  验证：浅色 / 深色两张离屏 BMP **逐字节完全相同**（视觉零变化）。

### 实验后不采纳（记录结论，避免以后重复试）

| 官方选项 | 实测 | 结论 |
|---|---|---|
| `bezier_tolerance` 0.1→0.02 | 全图差异 **0 像素** | 只作用于 `Quadratic/CubicBezierShape`；界面全是 `RectShape`，圆角走 epaint 自己的栅格化 |
| `prerasterized_discs=false` | 差异 **0 像素** | 界面没有用到预栅格化圆盘 |
| `feathering_size_in_pixels` 1.25 | 直边 R 降到 241/240（默认 245），弧段过渡像素 157→132 | 官方"更大值会发虚"属实，默认 1.0 最优 |
| `round_rects_to_pixels=false` | 圆角/直边与默认一致，仅别处 0.5px 位移 | 无收益，反而可能让拼接矩形露缝 |

`settings_shot` 保留 `feather=` / `nodiscs` / `noroundrect` 三个实验开关
（只作用于离屏出图），以后升级 egui 可一键复核这些默认是否仍最优。

### 审查确认已经跟上的

`Ui::visuals_mut`（不标脏）、`TextEdit::align`（弃用 vertical_align 的替代）、
`Visuals::slider_trailing_fill`、`StrokeKind`、`WidgetInfo::labeled/selected`、
`Id::with`、MSAA 能力查询 `sample_count_supported(4)`。

---

## 0.9.10 — 输入框：文字对齐修复 + 共享控件（2026-11-04）

### 修复

- **搜索框文字偏上**：之前用 `add_sized` 把 `TextEdit` 强塞进 34px 高、
  又给 `Frame::NONE`（零内边距），文字按内容高度顶格排，与居中的放大镜
  和 `Ctrl K` 明显不在一条线上。查 egui 0.37 官方文档（docs.rs）确认
  `vertical_align` 已弃用、应使用 **`align(Align2::LEFT_CENTER)`**：
  它把内部文字在已分配区域内二维对齐。修复后实测输入文字中心 46.5、
  清除图标 45.5、框中线 46。
- **清除按钮点不动**：整框 `interact` 注册在清除按钮之后，命中顺序把点击
  吃掉了。把整框交互提前注册，后注册的按钮才能压在上层。

### 重构

- 新建 `nooksill_settings::text_field` —— 所有 rs 输入框共用一个控件：
  外壳（12px 圆角 / field 底色 / 主题色 2px 焦点光晕）、放大镜图标、
  快捷键提示、有内容时的 × 清除按钮、整框可点聚焦、焦点收起 hint、
  官方 `align(Align2::LEFT_CENTER)` 垂直居中。`FieldOpts` / `FieldIcon`
  参数化，后续任何输入框（重命名、数值输入）直接调用，不再各写一遍
  painter + interact。
- 全工程当前只有顶栏这一处 `TextEdit`，`settings_win::shell` 已改为调用
  共享控件，原来 80 行手写绘制删掉。
- `settings_shot` 加 `q=文字` 预填参数，核对输入态对齐。

### 测试

kittest：清除按钮清空字符串、空内容时无清除按钮；纯函数单测：
焦点且空值时收起 hint。hint 是绘制文字、不是 AccessKit 节点，所以
焦点规则走纯函数而不是节点查询。

---

## 0.9.9 — HTML 雏形同步到当前 Rust 界面（2026-11-04）

只改 `docs/settings-prototype.html`，不动 Rust。

- **顶栏三块与 Rust 一一对应**：品牌区占 `NAV_W` 同宽并居中（`left`
  对齐 `.body` 的 10px 内边距，实测文字中心 140.5 / 侧栏中心 141.5）；
  搜索改成 260×34、12px 圆角的自绘壳（外层槽居中、内层承接磁吸），
  焦点时 2px 主题色 inset 光晕且 placeholder 收起；右上角换成主题色
  × 图标按钮（hover 淡底、带磁吸）。
- **主题色联动**：只改 `--accent` 一处，`--accent-2/3` 用 `color-mix`
  派生，导航选中、开关、滑杆、焦点光晕、关闭图标全部跟随；
  `?accent=RRGGBB` 深链 + 外观页「主题色」卡（来源下拉 / 取色器 /
  同步到 Dock 开关）实时改色。
- **补上雏形缺的设置项**：常规页「界面缩放」下拉、外观页「主题色」卡、
  常规页「关于」卡（展示 `assets/nooksill.ico` 应用图标资产）。
- 控件圆角与 Rust 对齐（10 / 12 / 18 / 22），`--r-pill` 不再用于搜索框；
  修正过时的头部注释（宽度已写死，没有 `fitWindow()`）与 caption。
- 深链新增 `?focus`；磁吸表加入 `.close-btn`。
- 截图核对：常规 / 自定义粉紫主题色 / 绿色焦点态 / 下拉展开 / 搜索筛选 /
  深色，HTML 结构校验无未闭合标签。

---

## 0.9.8 — 顶栏：搜索框圆角 / 品牌居中 / 图标关闭（2026-11-04）

### 修复与优化

- **搜索框圆角不协调的根因**：egui 0.37 的 `TextEdit` 高度由「文字 +
  内边距」决定（实测 22px），而框的圆角是固定 10px —— 半径几乎等于
  半高，看着是胶囊；下拉/按钮约 30px 高、同样 10px 圆角却是小圆角
  矩形，视觉曲率对不上。且焦点态描边走 `selection.stroke`、非焦点走
  `bg_stroke`，切状态时边框还会再变一次。
  改成**自绘外壳**：固定 260×34、12px 圆角、焦点时主题色 2px 光晕 +
  白底，TextEdit 用 `Frame::NONE` 只负责输入；放大镜与 `Ctrl K` 提示
  自绘，整块外壳可点聚焦；焦点且未输入时收起 hint（无框内边距为 0，
  否则光标压在第一个字上）。
- **NookSill 品牌对齐导航栏**：`NAV_W` 提到 settings crate 根公开，
  顶栏品牌区占同宽一格并 `centered_and_justified` 居中。实测文字中心
  116.0、导航栏中心 114.5。
- **关闭按钮改成绘制的 × 图标**：28×28 自绘控件，常态主题色、悬停
  浮出主题色淡圆角底；`widget_info` 自报 `Role::Button / "关闭"`，
  读屏与 kittest 仍查得到。深浅两套主题都跟随主题色。
- `settings_shot` 加 `focus` 开关，离屏核对焦点光晕。

---

## 0.9.7 — 主题色 / 控件优化 / exe 应用图标（2026-11-04）

### 新增

- **主题色设置**：外观页新增「主题色」卡 —— 下拉选「跟随系统」（读 DWM
  强调色，明度归一化到 190、无彩时回落默认蓝）或「自定义」（取色器，
  存 COLORREF）。键 `UiAccent`（-1 = 跟随系统）。
- **同步到 Dock**：`UiAccentDock` 开关打开时，主题色同时作用到 Dock 栏体
  渐变（按色相掺入 16%~30%）与运行指示点；关闭时维持原配色。运行时
  解析值存在 `Config::runtime_accent`，**不写 ini**（系统色是环境值）。
- **主题色贯穿设置窗口 UI**：导航选中块、开关开启态、滑杆填充、选区、
  提示圆点悬停、行悬停底色、超链接、输入框焦点全部走同一份主题色；
  放在 `gpu::accent::apply_visuals` 里，用 `Ui::visuals_mut` 而不是
  `Context::set_visuals`（后者每帧标脏 = 每帧重绘，破坏空闲不提交）。
- **元素优化**：按钮 / 开关 / 下拉 / 输入框圆角统一到 10（浮层 12），
  与 18px 卡片不再是两套设计；选中态不再二次淡化（之前淡到几乎看不见）。
- **exe 应用图标**：`assets/nooksill.ico` 多尺寸（16…256，内嵌 PNG，
  深色 Dock 胶囊 + 四个彩色图标点），`build.rs` + `winresource` 把资源
  编进 `nooksill-dock.exe` 的 ID 1；主窗口、菜单窗口、设置窗口的
  `WNDCLASSW.hIcon` 都挂上它。没装 Windows SDK / 找不到 rc.exe 时
  只警告，不影响构建与运行。
- `settings_shot` 支持 `accent=RRGGBB`，离屏核对任意主题色。

### 验证

三套自定义主题色（蓝 / 粉 / 绿）离屏出图，控件彩色像素随色变化；
exe 内检出 7 张图标 PNG（资源嵌入成功）。

---

## 0.9.9 — 三处毛玻璃（ADR-057，2026-11-05）

设置窗口 / 右键菜单 / 起动台终于用上了 `UiFrost` / `MenuFrost` / `LpFrost` 与
`blur` 这几个此前只存不用的键。

### 实现

- 引入 [`glass-backdrop`](https://github.com/MCheng404/glass-backdrop-rs)
  （固定 rev `747457c`）。只取三样：**GDI 桌面捕获**、与 WGSL 同源的
  **高斯权重模型**、**圆角 SDF**。
- 新增 `nooksill-gpu::frost`：开窗前抓一次窗口背后的桌面 → CPU 可分离高斯
  （25% 分辨率）→ 圆角 alpha 遮罩 → `egui::TextureHandle`，由
  `FrameRenderer` 画在 UI 最底层。
- 三处面板底色从 246/250 的不透明降到 140/255 的半透明 tint，让模糊的桌面
  透上来；捕获失败自动退回原底色。
- 设置窗口新增 `NOOKSILL_DEBUG_OPEN=settings`，无人值守验收不用再猜点击坐标。

### 为什么不用上游的 SPIR-V / liquid-glass

`glass-backdrop` 给的是「Vulkan 交换链 + `fs_glass` 全屏 pass + 预乘 alpha
合成」。我们的渲染是 `egui → 离屏 sRGB 纹理 → CPU 回读 → 预乘 BGRA DIB →
UpdateLayeredWindow`，**没有交换链**（ADR-008）。而且 `fs_glass` 的折射 /
色散 / 2.5D 高光边与项目的纯平基调相冲突。所以在 CPU 上把模糊做完，
当一张普通纹理画进圆角里 —— 是亚克力，不是液体玻璃。

### 已知取舍

- **只在开窗那一刻抓一次**：窗口开着时桌面变化、窗口移动都不会重抓。
  上游的 WM_MOVE 重抓与 30 Hz worker 是给常驻窗口用的；我们的面板是短命弹层。
- 没有 `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`：我们在
  `ShowWindow` 之前抓，自己还没上屏，天然不会递归抓到自己；而那个 API
  会让所有截图工具都拍不到分层窗口。
- 菜单默认仍是关的（`MenuFrost=0`，与上游一致）。

### 测试

`gpu::frost` 5 条：常量图不变、脉冲能量守恒、非法缓冲不 panic、圆角遮罩
（含 radius=0 必须特判，否则上游 SDF 会把方角面板钉在 50% 覆盖率）、
底色透明度。

---

## 0.9.8 — 分割线拖动缩放 Dock（ADR-056，2026-11-04）

`[behavior] SepScale` 从"只有键、没有行为"变成可用功能：**按住图标之间的
分割线上下拖动，按比例调整图标与放大尺寸**。默认关，设置页「交互」里有开关。

### 实现

- 拖动语义照上游 `CcMove` / `SepScale`：16 ms 节流、步长 0.6、图标夹
  `24..128`、放大尺寸按**初始比例**同比缩放后夹 `32..220`。
- **只在尺寸真变时**改配置并重建窗口 —— 鼠标停着不动不会闪。
- 分割线上光标是 `IDC_SIZENS`（上下双箭头）；`WM_MOUSEMOVE` 与
  `WM_SETCURSOR` 共用一处判断，拖拽期间不会退回箭头。
- 缩放不是点击：松手不启动应用；捕获被抢走时停止缩放，已改的尺寸不回滚。
- 写盘走既有的 400 ms 防抖队列，不在拖动过程中逐次全量回写 ini。

### 单位与偏差

`dy` 用客户区**物理像素**，与上游一致；`icon_size` / `zoom_size` 是配置里的
逻辑值。因此高 DPI 下拖同样的物理距离，尺寸变化会比 96 dpi 时稍慢 ——
这是上游就有的行为，保持一致优先于"修正"它。

### 测试

- `core::layout::sep_scale`：方向、比例、四个夹紧边界、回到起点不累积误差。
- `gpu::dock`：开关打开时接管按下 / 不进条目拖拽 / 抬起不点击；开关关闭时
  维持原行为；尺寸不变不重复标脏；捕获丢失后停止缩放。

---

## 0.9.7 — 控制中心磁贴可拖动排序（ADR-055，2026-11-04）

控制中心的 8 个磁贴不再顺序写死：**按住任意磁贴在格子间拖动即可换位**，
松手立即生效并写进 `[widgets] Order`。

### 新增

- **拖动重排**：超过 6 逻辑点的位移才判定为拖动（否则算点击），落到别的
  格子上**即时换位**，与上游 `CcMove` 一致 —— 3 列小格子里拖影比"看着它
  挪过去"更难读。
- **`[widgets] Order`**：逗号分隔的磁贴 id（`0` 无线 / `1` 蓝牙 / `2` 深色 /
  `3` 定位 / `4` 静音 / `5` 设置 / `6` 节能 / `7` 热点 / `8` 有线）。
  没拖过就不写这一行。
- `control::tiles_ordered` / `parse_order` / `order_text`：顺序在 core 里
  排，纯函数，可单测。

### 设计取舍

- 顺序里存**磁贴 id**而不是位置：开关某个磁贴不会把其它的挤乱。
- **关掉的磁贴保留它的位置**（`CcEnergy` / `CcHotspot`），重新打开回到原处。
- **有线 / 无线共用一个槽位**：插拔网线只换图标，位置不动。
- `Order` 里没提到的磁贴追加在末尾 —— 新版本加磁贴、或用户手改配置只写了
  一部分，都靠这条兜住。

### 测试

core 新增 8 条（id 往返、脏数据跳过、序列化往返、按序排列、关掉的保住位置、
网络槽位就地切换、未列出项追加、空串即默认）。

---

## 0.9.6 — 起动台标签组改名 / 删除 + 过滤视图拖动排序（ADR-054，2026-11-04）

### 新增

- **标签组改名 / 删除**：右键标签栏任意一组 →「重命名…」/「删除这一组」。
  改名时该标签描一圈；删除后分类号自动前移，正停在被删那组就退回「全部」。
  只剩一组时菜单里不给「删除」。
- **过滤视图内拖动排序**：分类 / 搜索视图里拖应用即可重排，落点语义是
  "插到这个应用前面"。只重排、不合并文件夹（过滤结果里相邻的两个应用在
  完整视图里未必相邻，就地合成会让人回头找不到）。
- 过滤视图的顺序改成**跟着顶层格子走**（原来按应用表顺序，也就是扫开始
  菜单的顺序，跟界面上看到的不是一回事）。

### 修复

- **改名框压在标签栏上**：原来在 y=96 另画一个 260×30 的浮层，而标签栏是
  92..120 —— 两个字叠在一起。现在**搜索框就地变成改名框**（位置固定、描
  accent 边、右侧给"回车确认 · Esc 取消"提示），浮层整块删掉。
- **「移动到标签组…」里的 ✓**：字体里没有那个勾，画出来是豆腐块。改成
  在组名后加「（当前）」。
- 改动 `launchpad.dat` 后只在**关窗时**写盘（既有行为，本轮未改）：
  进程被强杀会丢。已知，见 ADR-054 末段。

---

## 0.9.5 — 起动台分类 / 标签 / A–Z（ADR-053，2026-11-04）

起动台从"能翻页能搜索"补到"能分组"：

### 新增

- **标签组**：默认三组「办公 / 系统 / 工具」，可加组（最多 8 组）。
  `launchpad.dat` 新增 `G` 行存组名、`C` 行存手工分类，老文件照常读。
- **启发式分类**：按路径与名字归类（Office 系 → 办公；System32 / SysWOW64 /
  `shell:` → 系统；其余 → 工具）。
- **分类栏**：搜索框下方一排胶囊（全部 + 各组 + ＋），点一下就只看这一组。
  过滤视图只列应用、不列文件夹 —— 分类是给应用分的。
- **右键「移动到标签组…」**：二级菜单选组，当前组标「（当前）」，
  还能「自动分类（清除）」退回启发式。
- **A–Z 按钮**：按名字重排顶层格子（大小写不敏感，存回 dat；之后再拖拽仍可改）。
- 诊断探针 `cargo run -p nooksill-core --example lp_dump -- <launchpad.dat>`。

### 修复（两个都是本轮自己写出来的）

- **标签组读回来只剩最后一组**：判断"文件里有 G 行就以文件为准"写成了每次
  push 前都比一遍 `== 默认三组`，读完第三组正好又命中，第四行把前三组清空。
  改成一次性标志位，并补了 `default_tags_survive_a_roundtrip` 钉住。
- **分类器把所有应用判成"系统"**：照抄原版关键词 `\Windows\`，而起动台存的是
  开始菜单的 `.lnk` 路径 —— `C:\ProgramData\Microsoft\Windows\Start Menu\…`
  全都命中（实测 190/190）。改成只认 `\Windows\System32\` 与 `SysWOW64`。

---

## 0.9.4 — 内置条目图标与起动台键盘操作（ADR-052，2026-11-04）

### 新增

- **内置条目图标**（`nooksill-gpu::glyph`）：`nooksill:start` 画四格窗、
  `nooksill:launchpad` 画九宫格点阵。全部用 `painter` 现画 —— 不用 emoji
  （字体缺字就是豆腐块），也不用渐变（纯平基调）。此前这两个条目会去取
  shell 图标，取不到就拿"首字占位"，Dock 上于是立着一块写着「起」的方块。
- **控制中心小组件卡片**：补上滑杆图标。条带一直给它留着 0.72·栏高 的宽度，
  但从来没人画 —— 点得到、看不见，像胶囊上缺了一块。
- **起动台键盘操作**：方向键移动选中格（左右到头翻页）、Home / End 定位、
  PageUp / PageDown 翻页、Enter 打开。选中格画一圈高亮。

### 修复

- **点「起动台」条目没反应**：`nooksill:launchpad` 被当成普通文件交给
  `ShellExecute`，必然失败。现在走内置动作，直接开起动台；`nooksill:start`
  同理（按一下 Win 键）。
- 起动台方向键分支一度把 Esc 吞掉（`match` 里写了 `_ => return None`），
  已改成 if 链，Esc 逐层后退的行为恢复。

---

## 0.9.3 — 小组件字号随 DPI 自适应（ADR-051，2026-11-04）

高 DPI 下时间/日期小得看不清，而 `CPU xx%` 反而偏大 —— 同一份代码里混了
两种单位：时钟写死"固定 14 物理像素"，资源卡片则把物理 `bar_h` 当逻辑点用
（DPI 乘了两遍）。

### 修复

- **时钟 / 日期**：字号从卡片**逻辑高度**推（时间 `0.70·行高`、日期
  `0.44·行高`），100% / 150% / 200% 下是同一物理尺寸；150% 下比原来大约
  一倍，不再比悬停气泡还小。
- **资源监视器卡片**：`0.27·c.height()`（逻辑点），不再受 DPI 二次放大。
- 两者都保留"量真实字宽、装不下就缩"的兜底；横向留白加到 6 逻辑点，
  日期那行不再贴着卡片圆角。
- 时间比日期大一号：汉字同样字号下比数字"重"，同档会让日期反客为主。

### 测试

`widget_font_keeps_logical_size_across_dpi`：在 96 / 120 / 144 / 192 dpi 下
算出的字号换算回逻辑点必须一致（容差 1 点，来自 `bar_h` 的整数取整）。

---

## 0.9.2 — 点面板外面收起小组件（ADR-050，2026-11-04）

小组件的四块面板（日历 / 控制中心 / 资源监视器 / 计时器）现在**点面板以外
的任意位置就收起**，由新键 `[widgets] PanelDismiss`（默认开）控制。

### 新增

- `nooksill-gpu::popup`：全局按键轮询（`Watcher`）＋窗口矩形命中。
- `[widgets] PanelDismiss`：0/1，默认 1。设置界面「附件 → 小组件」里有开关。

### 修复

- **面板不会自动收起**：此前面板用 `SetCapture` 等外面的点击，但 Win32 只
  把鼠标捕获交给前台窗口，而 Dock 是 `WS_EX_NOACTIVATE`，永远不在前台 ——
  捕获设置了，消息永远不来。改成轮询全局按键状态（ADR-050 有完整复盘）。
- **资源监视器点开之后关不掉**：只有 `show()` 没有 `hide()` 的调用点。
- **计时器面板 Esc 不生效**：原来只在输入待办时处理 Esc。

### 行为

- 点别的应用 / 桌面：面板收起，那一下点击照常落到目标上。
- 点 Dock 上的图标：面板收起并正常启动/切换应用。
- 点同一张卡片：切换（再点一次收起）。

---

## 0.9.1 — 设置界面第二轮：顶栏搜索 + 说明气泡（2026-11-03）

把 `docs/settings-prototype.html` 第二轮的设计移植进 Rust。仍然没有新功能；
门禁：clippy `-D warnings` / `fmt --check` 全绿，315 条测试通过
（设置 crate 13 条，其中 2 条是本轮新增的契约测试）。

### 界面（照雏形逐项移植）

- **窗口 893 × 720**（导航栏 193 + 内容区 700），`LOGICAL_SIZE` 更新。
- **顶栏三栏**：左 `NookSill`（20px，去掉「设置」二字）；中搜索框
  （**几何居中** —— 三栏各自绝对定位，不靠 flex margin，否则会跟着左边
  文字宽度偏）；右「状态文本 + 关闭」。底部状态条删掉了，
  "已保存"等反馈挪到顶栏右栏。
- **搜索**：非空时只显示命中的导航标签，当前页被过滤掉时自动跳到第一个
  命中页；`Ctrl+K` 聚焦输入框。过滤词存 `SettingsState::nav_filter`，
  不是配置项。
- **导航**：不分组；整行 40px 圆角块，选中为强调色底（去掉左侧竖条 ——
  雏形改版去掉了它）；宽度写死 193，**不随标签内容变化**。
- **无直线**：卡片去掉 1px 描边，靠投影从底色上托起来（圆角 10 → 18）；
  侧栏的竖直分隔线也删了。
  - 修投影圆角的暗边：`spread: 2` 会让投影矩形外扩但圆角半径不跟着扩，
    暗边从卡片圆角的半透明 AA 边缘里透出来（弧段上实测 214/222 的暗像素，
    直边却干净）。`spread` 归零后只剩贴边处的正常混合。卡片间距同步加大。
- **界面缩放（自定义 DPI）**：常规页「设置界面」卡新增下拉 —— 跟随系统 /
  100% / 110% / 125% / 150% / 175% / 200%。字体、按钮、行高、卡片间距整体
  缩放（egui 的 ppp 是唯一缩放入口）；语义与 Windows 自定义缩放一致，
  是**覆盖**不是叠加。改完由主循环就地重建窗口（渲染器按新 ppp 重建 +
  SetWindowPos 调整尺寸并重新居中），不用重开。键：`UiScale`
  （0 = 跟随系统），只作用于设置窗口。
- **修复：点导航栏当前项之上的项会跳回常规页**。切页在导航循环中途改
  `state.page`，"旧页是否可见"的判断跟着遍历顺序漂 —— 点上面的项时旧页
  排在点击项之后、匹配不上，循环后被"当前页被过滤掉则跳第一页"的逻辑
  覆盖回常规。点下面的项恰好不触发。现在当前页在进循环前快照、点击
  优先于自动跳页，两条 kittest 回归测试守住。
- **移除"已保存"提示**：每次改动 400 ms 后顶栏都闪一次"已保存"是噪音。
  保存失败与其他一次性反馈（恢复默认、自启动失败等）仍会提示。
- **行的说明收进圆形感叹号**：标题旁 15px 圆点，**点击**或**悬停 3 秒**
  展开成 240px 宽的浮层卡片；点击可钉住，点别处 / Esc / 滚动 / Esc 收起。
  行高因此明显变紧。卡片级的说明（如"磨砂未实现"）保持内联没动。

### 实现

- 说明气泡的**展开状态跨帧存放在 `SettingsState::tip`**（归属圆点 id +
  文本 + 锚点矩形 + 是否钉住），悬停计时在 `SettingsState::tip_hover`。
  气泡本体在 `settings_ui` **末尾**统一渲染 —— 那时所有行的状态都已结算，
  不会出现"这帧开着、下帧才更新"的抖动。
- 气泡用 `Order::Tooltip` 的 `Area` 承载，`pivot` 翻转处理"下方放不下翻到
  上方"，不需要提前量高度。
- 搜索框用固定 `egui::Id`：Ctrl+K 要跨帧请求同一个输入框的焦点。

### 磁吸

光标靠近时元素朝它偏移：导航项 2.5px、感叹号圆点 1.2px、说明气泡 1.2px。
偏移量按"光标到元素矩形边缘的距离"衰减（元素内 = 满偏，出半径归零），
`magnet_pull` 是纯几何函数，有单测守住"分档、方向、上限"。

**只偏移绘制，命中测试留在布局位置** —— 3px 的差值对手感无影响，
省掉一套"视觉与点击目标分离"的补偿（HTML 的 transform 同样只挪视觉）。
卡片与按钮由 `Frame` / `Button` 内部绘制，平移需要重建布局，没做。

### 已知取舍

- 悬停 3 秒的计时没法在 kittest 里测（时间驱动的交互），只有点击路径
  有测试。真机悬停即可验证。

---

## 0.9.0 — 设置界面重做：左导航 + 多卡片（ADR-049，2026-11-03）

布局从"一列纵向 9 页"改成**左侧导航栏 + 右侧多卡片**，并把设置界面按页
拆成独立文件。没有新功能，门禁：clippy `-D warnings` / `fmt --check` 全绿，
312 条测试通过（设置 crate 12 条，其中 4 条是本次新增的布局契约测试）。

### 新增

- **`docs/settings-prototype.html`** —— 视觉基准。10 页、28 张卡片、92 行控件，
  与 Rust 端一一对应；改设计先改雏形。
- **一页一个文件**：`pages/general.rs` … `pages/add.rs`，共享控件收进
  `pages/widgets.rs`，下拉选项表收进 `pages/options.rs`。`pages.rs`（653 行）
  删除。加一页 = 加一个文件 + `Page` 枚举加一项。
- **`FrameRenderer::render_rgba`**：画完不呈现，直接把预乘 BGRA 返回。
  `render` 改成它的薄包装，两者共用同一段绘制。
- **`examples/settings_shot.rs`**：把任意一页离屏渲染成 BMP
  （`cargo run -p nooksill-gpu --example settings_shot -- 出图.bmp 外观 dark`）。
  走的是与真实窗口完全相同的 `FrameRenderer` + `settings_win::shell`，
  所以截出来的就是用户会看到的那一帧。

### 修复

- **两列会盖住标题栏**：切矩形时用了 `ui.max_rect()`，它不随已用内容收缩，
  起点仍是窗口顶端。改用 `available_rect_before_wrap()`，并加了一条测试
  （`有标题栏时两列不会盖住标题`）—— 这条测试不模拟外壳就永远发现不了。
- **卡片宽度随内容收缩**：`Frame` 的宽度由内容决定，只有一行 `label` 的卡片
  会缩成一小块（性能页「当前占用」、添加页「重置」），和同列其它卡片对不齐。
  `card()` 现在强制内容撑满可用宽度。
- **说明文字挤在标题右边**：`row()` 把标题和说明并排放，长说明会把右侧控件
  挤出可视区。改成上下排，并给控件预留固定宽度。
- **性能页两个开关与其它行两套写法**：磨砂与"扫描运行中应用"是派生开关
  （不是一对一映射到某个键），原先直接写 `ui.checkbox(...)`，复选框贴在
  文字左边。新增 `toggle_derived` 统一成"标题在左、开关在右"。
- **`numeric_constants_match_table_rows` 失败**：`NUMERIC_KEYS` 已加到 94 行
  （新增 11 个烟雾键），测试清单还停在 83 行。补齐并顺手把定长数组改成
  切片 —— 数组长度写死之后，每加一个键都要改那个数字，而改数字恰恰是这条
  测试要防的"手工同步"。
- **`app_window.rs` 的 `AttachThreadInput` 导错模块**（在
  `Win32::System::Threading`，不在 `Input::KeyboardAndMouse`），整个 workspace
  编不过。

### 布局契约（kittest）

截图要 wgpu，而 `nooksill-settings` 刻意不依赖 wgpu。所以"画在哪"用矩形
断言守：导航项在内容区左侧且整行可点、有标题栏时两列不越界、说明文字排在
标题下方、开关与下拉的右边缘对齐。这四条比截图更适合 CI —— 它们不挑机器，
也不会因为字体微调而误报。

### 已知限制

- **没有在真窗口里跑过**：离屏截图与真实窗口走同一条渲染路径，但分层窗口
  的透明、DPI 缩放、鼠标命中仍只能在有显示器时验证。
- 导航栏不支持滚动。10 页刚好放得下，加到 14 页以上会溢出。

---

## 0.8.6 — 点击应用 / 获取运行中应用（ADR-049，2026-11-03）

用户报"点图标没反应"。查下来三层问题叠在一起，外加一个一直没实现的键。

### 修复

- **点击被自己吞掉**（从 M1 就存在）：`WM_LBUTTONUP` 里先
  `ReleaseCapture()` 再结算点击，而前者会同步触发 `WM_CAPTURECHANGED`
  → `cancel_drag()` 清掉 `press`，于是每次点击都判成"没按在任何图标上"。
  改成先结算再交还捕获，并且 `cancel_drag` 不再清 `click`。
- **前台锁**：`SetForegroundWindow` 对 `WS_EX_NOACTIVATE` 的 Dock 经常
  无效。加三级阶梯（直接置前 → `AttachThreadInput` → ALT 键），每级复核。
- **"再点一次收起"永远不触发**：前台判定只比句柄，多窗口应用 / 桌面 /
  提权窗口下必然为假。改成"句柄 → 同进程 → 自己没被遮挡"三条取或。
- **UWP 宿主**：`ApplicationFrameHost.exe` 不再进运行列表（所有 UWP 应用
  的窗口都归它，按 exe 匹配会把计算出成设置）。

### 新增

- **`ShowRunning`（未固定的运行中应用）**：扫描到的运行应用按需要追加到
  栏体末尾，点击即切换；不写 ini、不参与重排，右键菜单不给结构性命令。
  上限 16 个。临时条目名取 shell 显示名（"Google Chrome"）。
- `core::running::temp_keys` 纯逻辑 + 5 条测试；`app_window` 加
  `in_front` / `unobstructed` / `Activation` 枚举。
- 探针：`cargo run -p nooksill-platform --example activate_probe -- notepad.exe`。
- 诊断：`NOOKSILL_LAUNCH_TRACE=1` 打印每次点击的落点（激活/启动）与结果。
- 扫描按需：`ScanInput::want_running`，指示点全关时不再每 1.2 秒枚举窗口。

### 测试

`click_survives_release_capture`、`lost_capture_drops_the_click`、
`temp_item_has_no_structural_commands`、`temp_*` 五条。

---

## 0.8.5 — 环境烟雾（ADR-048，2026-11-03）

栏体周围加了一层**真的在流动**的烟：上升、卷曲、漂过边缘、随鼠标让开。
算法移植自 Windhawk 的 *Taskbar Smoke*（Rubicus），渲染接到 egui。

### 新增

- `nooksill-core::smoke`：可平铺的域扭曲 FBM 噪声、Stam 稳定流体
  （浮力 / 涡量约束 / 压力投影 / 位移场）、网格 → 预乘 RGBA 光栅化。
- `nooksill-gpu::smoke`：按真实帧间隔驱动模拟、上传纹理、画在栏体之下；
  另有一层"上升的小烟团"粒子。
- `[effect.smoke]` 段十一个键（CONFIG.md 第三节），设置界面新增「效果」页。
- 诊断：`NOOKSILL_SMOKE_TRACE=1` 打印网格 / 位图 / 最大 alpha / 栏体矩形。
- `cargo run -p nooksill-core --example smoke_dump` 把一帧合成到深色背景上
  写成 BMP —— 调参不用起 Dock。

### 与原版的偏离

- 发射区从"整个胶囊"改成"贴轮廓的窄带"（Dock 栏体比任务栏胶囊高一倍多，
  照抄会只看得见沿顶边的一条细线）；
- 不做「经典纹理烟」风格（需要 egui 没有的带纹理描边）；
- 烟画在栏体**之下**（画在上面会把图标染紫）。

### 性能

关着零成本（噪声表与模拟都懒建）；开着按 30 fps 出帧，release 下
模拟 0.6 ms + 光栅化 1.3 ms 每帧。

### 已知限制

- 只在底部停靠生效；
- 栏体只有一两个条目时表现为"一圈不均匀的光环"，条目多了才像飘动的烟。

---

## 0.8.4 — 合并胶囊 / HPad / 托盘小组件（ADR-047，2026-11-02）

v1.6.5 里落在条带模型上的三件事。

### 新增

- **系统托盘小组件**（`TrayW`，默认开）：条带最外侧一个 chevron，点了
  打开 Windows 通知区域（`Win+B` + `Enter`；不抄上游跨进程点 Explorer
  工具栏那段）。
- **合并胶囊**（`WMerge`，默认开）：所有小组件共用一块底盘，成员之间的
  间隔留在胶囊内部 —— 只改绘制，布局一字节未动。
- **HPad**：横向内边距独立成键（默认 12），不再恒等于 `pad + 2`。

### 几何变化（要有心理准备）

`HPad` 默认值 12 vs 原来的 `pad + 2` = 10，**栏体两侧各宽 2 px、里面内容
整体右移 2 px**。这是上游 1.6.5 自己的变化；我们的"与 1.4.0 逐像素一致"
基线到此结束，对拍基准要换成 1.6.5。

### 已知限制

- 托盘卡片只做"打开通知区域"，不显示图标数量，也不能直接操作里面的图标

## 0.8.3 — 吸收 v1.6.5 的 19 个键 + 启动宽限期（ADR-046，2026-11-02）

上游 1.6.5 比发布说明写的大得多（见 `PORTING.md` §八）。这一轮只做两件
**不改变现有渲染**的事，先把数据与闪动问题兜住。

### 新增

- **配置表 99 → 118**：`HPad` / `UiFrost` / `LpFrost` / `TrayW` / `WMerge` /
  `StatusBar` / `SbFrost` / `SbOpacity` / `SbColor` / `SbRadius` / `SbHeight` /
  `SbFollowSys` / `SbTopGap` / `SbMaxAvoid` / `SbFloat` / `SbSideGap` /
  `SbAutoHide` / `SepScale` / `MenuFrost`，默认值与值域照上游 `DefaultCfg()`
  与设置行。新增段 `widgets.statusbar`。
- **启动宽限期**（`occlusion::LAUNCH_GRACE_MS` = 1.5 s）：启动应用后的
  1.5 秒内不做全屏判定，避免新应用以最大化姿态出现时 Dock 闪一下。

### 缺陷修复

- 从上游 1.6.5 升级会**静默丢掉**这 19 个键（`upgrade_legacy` 删段重写，
  表里没有就一起没了）。ADR-038 修过一次同类问题，这是第二次复发。
  新增 `upgrade_legacy_keeps_upstream_v165_keys` 守着。

### 说明

- 这 19 个键目前**只存不用**：状态栏、托盘小组件、毛玻璃、分割线缩放
  都还没实现，写进 ini 只是保证升级不丢配置。动手实现时直接从表里取。

## 0.8.2 — 光标不再是"忙"（ADR-045，2026-11-02）

鼠标一进 Dock 或任何一个自绘面板，光标就变成沙漏/转圈，而且会一直转。
两条独立成因，都实测确认：

- 窗口类 `hCursor` 留空、又不回 `WM_SETCURSOR` 时，Windows 退回的是
  **忙光标**（不是箭头）；
- `SetCapture` 期间系统**根本不发** `WM_SETCURSOR`，于是"弹出即捕获"的
  模态菜单、日历弹层、控制中心、按住拖动——全程忙光标。

### 缺陷修复

- 新增 `gpu::cursor`：类光标、形状映射、客户区坐标三件事收在一处，
  七个窗口类统一给它填 `hCursor`。
- Dock 主窗口、右键菜单、起动台、资源面板补 `WM_SETCURSOR`。
- 持有捕获的窗口（Dock、菜单、日历、控制中心）在 `WM_MOUSEMOVE` 里
  也设一次光标 —— 捕获期间只有这条消息能到。
- 手型光标跟着命中区走：图标上给手型，栏体缝隙/空白给箭头。
- 设置窗口原先在 `WM_SETCURSOR` 里一律设箭头，文本框上永远见不到 I 型
  光标；现在记住上一帧 egui 想要的形状。

### 验证

`scratch/check_cursor.py` 逐个打开七处窗口、把鼠标移进去、读
`GetCursorInfo`，全部为 HAND/ARROW，没有 WAIT。

## 0.8.1 — 中文输入法 + 起动台菜单（ADR-044，2026-11-02）

补上 ADR-043 留的两个缺口：**能打中文**、**起动台能改名 / 加到 Dock**。

### 新增

- **中文输入法**（`gpu::ime`）：接 `WM_IME_STARTCOMPOSITION` /
  `COMPOSITION` / `ENDCOMPOSITION` / `SETCONTEXT`，用
  `ImmGetCompositionStringW` 取组合串与上屏文字，`ImmSetCompositionWindow`
  把候选窗摆到光标处。已接到起动台搜索框、起动台重命名框、计时器待办输入。
- **组合中的拼音可见**：输入框把「已上屏 + 组合中」一起画，组合部分带下划线
  —— 用户得能看出"字还在输入法里"。
- **起动台右键菜单**（画在起动台窗口内部，不另开 HWND）：应用 4 项
  （打开 / 重命名 / 添加到 Dock 栏 / 从起动台移除），文件夹 3 项
  （打开 / 重命名 / 解散文件夹）。
- **重命名**：菜单进入后直接打字，回车确认、Esc 取消；同样支持中文（走 IME）。
- **添加到 Dock**：起动台抛 `LaunchpadCmd` 给主循环，由 `bin/dock.rs` 改配置。

### 缺陷修复

- 起动台**格子本体没进命中区**：`hit_at` 只查 `hits`，而网格只把图标区
  push 进了 `icon_rects` —— 于是点图标不会启动任何东西，右键也永远不出菜单。
  上一轮验收只点了"空白处关闭"，把这个洞漏过去了。
- 起动台窗口补 `WS_EX_TOPMOST`：全屏覆盖层被前台窗口盖住时看起来像没打开。

### 诊断

- `NOOKSILL_LP_TRACE=1`：打印起动台的右键命中与菜单项执行。

## 0.8.0 — 控制中心与起动台（ADR-042 / ADR-043，2026-11-02）

上游 v1.6.0 剩下两块大功能。专注模式（多布局）按需求跳过。

### 新增

- **控制中心**（`gpu::cc_win` + `core::control` + `platform::audio`）：
  点条带上的控制中心卡片弹出。8 个磁贴（无线网络 / 蓝牙 / 深色模式 / 定位 /
  静音 / 设置 / 节能 / 热点）、一条真音量滑块、电量文案。
  - 音量走 Core Audio 的 `IAudioEndpointVolume`，不是模拟按键；
  - 深色模式改注册表并广播 `ImmersiveColorSet`；
  - Wi‑Fi / 蓝牙 / 热点走 `Win+A` 拉系统快速设置（没有稳定的公开接口）；
  - 没有音频设备时滑块降级为禁用态。
- **起动台**（`gpu::launchpad_win` + `core::launchpad` + `platform::launchpad`）：
  右键菜单「起动台」打开全屏网格。
  - 扫开始菜单的两处 `Programs` 目录（深度 4，过滤卸载器 / readme）；
  - 分页（`LpCols × LpRows`）、页码点、滚轮翻页、搜索框；
  - 拖拽重排；拖到图标上合并成文件夹；文件夹弹层；右键移出文件夹；
  - 把文件 / 快捷方式拖进窗口即添加（`WM_DROPFILES`）；
  - 顺序存 `launchpad.dat`，格式与上游一致。

### 缺陷修复

- 控制中心的音量滑块位置：上游按固定 2 行磁贴算 y，而默认是 3 行（节能 +
  热点都开），滑块会压在第三行上。改为按实际行数算。
- `FrameRenderer::render` 在呈现失败时用 `?` 提前返回，跳过了
  `textures_delta.clear()` —— 于是 epaint 的 Drop 断言会把真正的错误
  （"UpdateLayeredWindow 失败"）盖成"纹理增量没人处理"。改为先清理再传播。

### 已知限制

- 起动台**没有中文输入法**（`WM_CHAR` 拿不到拼音组合），英文搜索可用。
- 起动台没有重命名、没有"添加到 Dock"、没有右键菜单（只有文件夹移出）。
- 控制中心没有亮度调节；有线网卡不检测（永远显示「无线网络」）。

## 0.7.0 — 日历 / 农历（ADR-041，2026-11-02）

点时钟卡片弹出日历：年/月翻页、「今天」、周一起始的 6×7 网格，每格下方一行
副标签（节日 / 节气 / 农历日），底部是选中日的详情。

### 新增

- **`core::calendar`：农历 / 节气 / 干支，零 Win32**。
  - `solar_to_lunar` / `lunar_to_solar` 互逆，支持 1901–2099；
  - `term_date` / `next_term`：24 节气（平气公式，与上游一致）；
  - `sub_label`：节日 > 农历节日 > 节气 > 初一显示月名 > 农历日名；
  - `ganzhi_text`：年月日三柱，年以立春换、月以节换；
  - `CalendarState`：翻页 / 选中 / 42 格网格，全部可单测。
- **`gpu::calendar_win`：弹层**（392×480 逻辑像素）。点时钟卡片打开，
  再点一次或点外面关闭；摆在卡片旁边，按停靠边决定方向并夹进显示器。
- **`NOOKSILL_GEOM=1` 之外新增 `examples/calendar`**：命令行打印某月的网格
  与固定点，用来人工核对算法。

### 验证

- **72 684 天逐个往返**（1901-01-01..2099-12-31）全部一致；
- 农历日逐日连续（无跳号、无重号，上月长度只能是 29/30）；
- 2020–2026 七年春节、三个闰月（2017 闰六 / 2020 闰四 / 2023 闰二）对齐公开日期；
- 节气落点全区间检查：立春 2/3–5、春分 3/19–22、夏至 6/20–22、冬至 12/21–23。

### 缺陷修复

- `LYearDays` 的位循环照抄成 `(0x8000..=0x8)` 是**空区间**，每年少算 0~12 天，
  整个农历错位两年多（2020 春节算成 2022-02-20）。改为按位移数遍历。

### 已知限制

- 平气公式与真实节气最多差一天（上游同）。
- 弹层开着时 Dock 收不到鼠标消息（`SetCapture` 的代价，与菜单一致）。
- 右键时钟卡片的「小组件菜单」（换边 / 隐藏）还没接。

## 0.6.0 — 计时器 / 待办小组件（ADR-040，2026-11-02）

### 新增

- **计时器 / 待办小组件**：条带上多一个钟表卡片，点开是面板 ——
  倒计时 / 秒表切换、开始暂停重置、待办列表（打勾 / 删除 / 添加）。
  对应上游 v1.6.0 的 `ToolDraw()` / `ToolUp()`。
- **`core::tool`**：`ToolState` / `Todo` / `ToolMode`，纯逻辑且时间基准
  由调用方传入，全部行为可单测。
- **`[Tool]` 段**读写（`N` / `T0..Tn` / `Total` / `Remain`），与配置和条目
  分开防抖落盘。
- **时长可调**：比上游多两个「−1 分钟 / +1 分钟」按钮（上游写死 5 分钟）。
- **面板高度按待办条数自适应**（上游固定高度，少条目时大片空白）。

### 缺陷修复

- **上游的待办解析 bug**：`0|` 前缀没被剥掉，每轮存读都多叠一层
  （`0|买菜` → `0|0|买菜` → …），而上游每次打勾都会存盘。
  Rust 侧只剥一层，既根治累积又不误伤用户文本里合法的 `1|`。

### 已知限制

- **中文输入法未接**：`WM_CHAR` 能拿到已上屏的字符，但 IME 组合过程中的
  候选窗口没处理，拼音直接打不出来。英文与粘贴可用。

### 实测

- 截图确认：面板显示 3 条待办（含一条已勾选）、倒计时 03:00、
  勾选框与删除叉号正常渲染、高度贴合内容。
- 点「开始」后数字从 05:00 走到 04:58，`[Tool]` 段落盘 `Remain=299`。
- 质量门禁：`fmt --check`、`clippy -D warnings`、220 条测试全绿。

## 0.5.0 — 资源监视器小组件（ADR-039，2026-10-31）

### 新增

- **资源监视器小组件**：条带上多一张「CPU xx%」卡片（双倍宽时显示
  CPU + RAM），点它弹出详情面板 —— 处理器 / 内存 / 磁盘（C:）三条进度条。
  对应上游 v1.6.0 的 `DrawRes()` / `ShowResMon()`。
- **`platform::sysinfo`**：CPU 占用率（`GetSystemTimes` 差值）、
  内存占用（`GlobalMemoryStatusEx`）、磁盘占用（`GetDiskFreeSpaceExW`）。
- **按秒采样、数值变了才出帧** —— 不是每帧调系统 API。
- **`NOOKSILL_GEOM=1` 新增一行** `GEOM widget=... screen=(x,y) size=(w,h)`，
  报小组件卡片的屏幕坐标，自动化脚本照着点。

### 缺陷修复

- **卡片文字会溢出**：上游靠 GDI+ 的 fit-to-rect 自动缩字，egui 不会。
  改为字号按栏高走 + 量了字宽再缩。
- **详情面板半透明且数值消失**：面板传了 `RawInput::default()`，
  没有 `screen_rect`，egui 的绘制区只有一小块。改用 `Input::take()`。

### 实测

- 截图确认：条带上「CPU 11%」完整显示在卡片内；面板显示
  「处理器 4% / 内存 20152 / 64733 MB / 磁盘 可用 50 / 299 GB」。
- 质量门禁：`fmt --check`、`clippy -D warnings`、218 条测试全绿。

## 0.4.0 — 吸收上游 v1.6.0：23 个新键与小组件条带（ADR-038，2026-10-31）

### 新增

- **上游 v1.6.0 的 23 个配置键全部入库**（`Launchpad` `WSide` `WOrder`
  `NotifyW` `FanBg{L,D}` `FanFg{L,D}` `PinMenu` `MultiMode` `UpdChk`
  `UpdLast` `LpCols` `LpRows` `FanDrag` `TrayHide` `CtrlCenter`
  `CcEnergy` `CcHotspot` `ResMon` `ToolW` `WideMask` `ModeW`），
  键表 76 → 99，顺序与上游 `g_tab[]` 一致。
- **小组件条带模型**（`core::layout::WidgetKind` / `WidgetStrip`）：
  把"栏体右边挂一个时钟卡片"泛化成"栏体外侧的一排卡片"，
  支持 `WSide`（左右）、`WOrder`（时钟与通知的先后）、`WideMask`
  （双倍宽）。这是上游六个新小组件的共同前提。

### 缺陷修复

- **从上游 v1.6.0 配置升级到我们的格式会丢 23 个键**。
  `upgrade_legacy` 的做法是"删掉 `[Settings]` 整段再重写"，凡是
  `Key::ALL` 里没有的键就一起没了。新增 `upgrade_legacy_keeps_upstream_v160_keys`
  守着这条。

### 变更

- `Metrics.clock_extra` 更名为 `widget_extra`（= 上游 `g_ce`），
  语义从"时钟卡片占位"变为"小组件条带占位"。
- 时钟卡片的绘制位置改走条带定位（`WidgetStrip::left_of`），
  `WSide=1` 现在真的会生效。

### 已知限制

- 通知 / 控制中心 / 资源监视器 / 计时器 / 专注模式这五个小组件**尚未移植**，
  它们即使配置为开也不占位（`WidgetKind::is_implemented` 只对时钟为真）——
  否则栏体上会留出画不出东西的空白。

### 实测

- 几何回归：同一份对拍 ini，重构前后各跑一次 Dock，`GEOM` 输出逐字节相同
  （时钟开 / 关两种配置各测一次）。v1.4.0 已对拍通过的几何未被破坏。
- 质量门禁：`fmt --check`、`clippy -D warnings`、209 条测试全绿。

## 0.3.2 — 把阻塞调用挪出 UI 线程（ADR-037，2026-10-31）

没有新功能。门禁：clippy `-D warnings` / `fmt --check` 全绿，199 条测试通过。

### 新增

- **`nooksill-gpu::worker`：四个后台线程**，各自一条 `std::sync::mpsc` 通道。

  | 线程 | 接管的阻塞调用 | 原来压在 UI 线程上的频率 |
  |---|---|---|
  | `IconLoader` | `SHGetFileInfoW` + `IImageList::GetIcon` + `GetDIBits` | 启动 |
  | `Scanner` | `EnumWindows` + 每窗口 `OpenProcess` / 8 次查询 | 1.2 s / 100 ms |
  | `DiskWriter` | 读 ini + UTF-16 全量回写 | 拖拽每一步 / 改设置 |
  | `Opener` | `ShellExecuteW` + 激活已有窗口 | 点击 |

- **`Waker` + `WM_APP_WAKE`**：工作线程有结果时 `PostMessage` 打断主循环的
  `MsgWaitForMultipleObjectsEx`。`pump` 对这条消息只打断等待、**不强制出帧**，
  所以"空闲 0 帧"的承诺仍然成立 —— 没有退化成轮询。
- **图标像素处理走 rayon**：缩放判定与圆角遮罩按批 `par_iter`。这是全项目
  唯一的 CPU 密集型工作（每张 256×256 六万多个像素），也是唯一值得上多核的地方。
  其余三条是 IO 等待，并行只会增加线程数。
- **写盘合并**：条目与配置共用一个 400 ms 防抖窗口，一次 flush 只写一遍文件；
  `WriteJob` 允许只写其中一段（另一段原样保留）。

### 修复

- **`platform::shortcut::ensure_com` 的 COM 初始化是进程级的**：原来用
  `static Once`，只有第一个调用的线程被初始化，其余线程上的
  `CoCreateInstance` 会以 `CO_E_NOTINITIALIZED` 失败。单线程时看不出来，
  一引入工作线程就变成"图标偶尔取不到"——没有报错、没有崩溃、只是结果缺失。
  改成 `thread_local!`，并挪到 `platform::win::ensure_com`。
- **拖拽时反复写盘**：`step_reorder` 原来每挪一格就 `Ini::load` + 全量回写一次，
  拖着走五个图标就是五次文件重写。改成标记脏 + 防抖，拖动期间零 IO。
- **图标可能装错条目**：排队时记的是"当时的第 i 项"，期间增删条目后下标会指向
  别人。`ReadyIcon` 带上路径，装之前核对一遍。

### 删除

- `crossbeam-channel` / `arc-swap` 两个"计划中"的 workspace 依赖。前者被
  `std::sync::mpsc` 取代（队列深度只有个位数），后者本项目的跨线程数据是
  值拷贝而不是共享快照，用不上。
- `Win32_Graphics_GdiPlus` feature —— GDI+ 老实现已在 ADR-035 里删除。

### 已知局限

- 运行指示点最多晚 1.2 s 亮起（本来就是 1.2 s 一轮扫描，只是首轮不再同步）。
- 图标从"启动时全部就位"变成"窗口先出来、图标陆续补上"。
- 进程被强杀会丢掉最后一次未落盘的顺序调整（400 ms 窗口内）。
- **没有真机跑过**：`worker` 的纯逻辑部分（写盘合并、图标处理）有单测，
  但线程接线（唤醒、模态菜单期间的排队、退出时的 detach）只能靠人跑一遍。

---

## 0.3.1 — 代码审查与主循环整理（2026-10-31）

没有新功能。门禁：clippy `-D warnings` / `fmt --check` 全绿，
197 条测试通过（119 core + 47 gpu + 22 platform + 1 + 8 settings）。

### 重构

- **`bin/dock.rs` 的 `main` 从 555 行拆开**：16 个局部可变变量收进 `App`
  结构体，循环体拆成 `pump` + 11 个 `step_*` 方法（布局、重布局、重排、
  点击、菜单、设置、写盘、全屏、扫描、自动隐藏、出帧）。
  拆分前插入新功能只能按 `②.52` / `②.535` 这类编号找位置，而编号本身
  已经重复（两个 `②.1`、两个 `②.4`）—— 这是"还在长"的代码最典型的症状。
  `main` 现在 12 行，`run` 44 行，每一步单独可读。
- **调试开关收进 `trace` 模块**：原先 16 处 `std::env::var_os("NOOKSILL_GEOM")`
  散在主循环里，每帧各锁一次进程环境块再线性扫描一遍。现在启动时读一次
  并缓存；16 份重复的魔法字符串也只剩一份，拼错会立刻编译失败。
- **`RawInput` 构造合并**：Dock 与菜单原先各写一遍同样的
  `screen_rect` + `native_pixels_per_point` 设置，现共用 `raw_input()`。
- 三处 `SetWindowPos` 合并成 `App::reposition()`（重布局与重挂路径参数相同）。

### 修复

- **两处生产代码的 `expect`**（菜单窗口、设置窗口"刚建好所以一定有"）改为
  返回错误。逻辑上确实安全，但违反了"失败要能看见"的风格：真出问题时
  应该是错误信息，不是没有上下文的 panic。
- **文档与实现脱节**：
  - `ARCHITECTURE.md` 第一节还在描述 `ArcSwap<Snapshot>`、rayon 工作线程、
    DXGI Desktop Duplication 与 compute 模糊 —— 这些**都没实现**，
    而且 `Cargo.toml` 里那三个依赖至今没有任何 crate 引用。改成如实现所述，
    并把"尚未接入"的清单列清楚。
  - 不变式第 2 条讲的是 `GdiPlus` 的构造/析构顺序，而 GDI+ 老实现已在
    ADR-035 里整体删除。换成 `GWLP_USERDATA` 必须指向 `Box<Dock>` 这一条
    （ADR-036 踩过的野指针）。
- `.gitignore` 补上 `/.workbuddy/`（与 `/.workbuddy-ai/` 并存，工具版本不同）。

### 已知局限

- **默认 `BgMode` 是 `Frost`，但磨砂没实现**，实际画的是渐变；设置界面里
  那一项仍写着"磨砂玻璃"。这是 PORTING.md 第 11 行记录的降级（"关闭磨砂 =
  退回渐变"），M3 落地前无法区分。已在 `paint_bar` 里写明来由。
- 本次重构**没有真机跑过**。`Dock` 的逻辑有 47 条单测覆盖，但 `App` 的
  编排（初始化顺序、模态菜单、AppBar 重挂）只能靠人跑一遍确认。

---

## 0.3.0 — 设置窗口接进 Dock 进程（ADR-036，2026-10-31）

### 新增

- **Dock 进程内的设置窗口**（`gpu::settings_win`）：右键菜单新增「Dock 设置」，
  打开一个独立 HWND 的分层窗口，渲染 `nooksill-settings` 的 9 页 UI。
  同一份 `Config` 被两边共用，改一个滑块下一帧就在栏体上生效，没有 IPC、没有复制。
- **非模态**：不像菜单那样抢走消息循环，可以一边调参数一边看栏体的变化。
- **配置自动落盘**：改动后 400 ms 防抖写回 ini（拖滑块不会逐帧重写文件），
  写之前重新读一遍以保住 `[Items]` 段。
- **停靠边 / 预留 / 隐藏模式变化会重挂**：重新注册 AppBar 并按新边重算窗口位置，
  尺寸变化走已有的热更新路径（ADR-033）。
- **开机自启动开关**（命令 `ToggleAutostart`）读写 `HKCU\...\Run\NookSill`；
  显示的是注册表真值，不是缓存的配置。
- **推荐设置 / 低占用**两组预设，以及"在这一页加分割线 / 回收站"入口。
- `NOOKSILL_SETTINGS_TRACE=1`：打印设置窗口的窗口消息流与交给 egui 的输入事件
  （调试本次接线时的主要手段）。

### 缺陷修复

- `SettingsWindow` 构造后会**移动**（`*slot = Some(sw)`），而 `GWLP_USERDATA`
  里存的是移动前的地址 —— 窗口过程下一次取到的就是野指针，实测表现为
  "点一下设置窗口进程就崩"。改为 `new` 直接返回 `Pin<Box<Self>>`，
  并补了 `Drop` 清零 `USERDATA`。
- 设置窗口原为普通层级，被别的窗口盖住后完全够不着（我们没有托盘图标兜底）。
  补 `WS_EX_TOPMOST`，与 Dock 同层。

### 删除

- `overlay.rs` 与 `bin/spike.rs`（合计约 600 行）是设置窗口的原型实现，
  正式版上线后删除，避免两份实现各自漂移（与 ADR-035 同一思路）。

## 0.2.1 — 删除 GDI+ 老实现（2026-10-30）

`nooksill` / `nooksill-gfx` / `nooksill-ui` 三个 crate 删除，共 **2000 行**
（占当时总代码 12%）。workspace 从 7 个 crate 减到 4 个。

### 删除

| 路径 | 行数 | 内容 |
|---|---|---|
| `crates/nooksill` | 83 | 老的 bin 入口（`NookSill.exe`） |
| `crates/nooksill-gfx` | 969 | GDI+ 封装（canvas / objects / 字体） |
| `crates/nooksill-ui` | 948 | GDI+ 版 Dock 骨架 |

它们是 M0 之前的一次移植尝试，**从来不是行为基准**（基准一直是上游 C 版
源码）。ADR-010 当初留作 fallback，现在 wgpu 路线已跑通、两份实现开始漂移
（`layout.rs` 里记着一例基准宽度算法分歧），且 `cargo build` 会产出两个
可交互的 Dock，调试时先要确认跑的是哪个 —— 成本每次都在付。

### 可见变化

- **可执行文件名只剩 `nooksill-dock.exe`**（原来的 `NookSill.exe` 一并删除）。
- `Cargo.lock` 清理干净，`cargo build --workspace` 少编三个 crate。

### 未做

- 不留 `--features legacy-gdi` 开关：会让"删干净"永远差一步，
  CI 与测试都要为不存在的路径付成本。
- 不另开 `archive/` 分支：git 历史里有，需要时
  `git checkout <sha> -- crates/nooksill-ui` 就能取回。

质量门禁：197 条测试（与删除前一致）、`clippy -D warnings`、`fmt --check`
全绿；启动实测 3 个真实图标正常渲染。

---

## 0.2.0 — 配置格式 v2（2026-10-30）

「去除对上游依赖」的第一步：配置不再照抄上游的键名与分段。
**上游格式仍可直接读，打开一次就就地升级，值一个都不会丢。**

### 变更（对外契约）

- 配置按**语义分段**：`[appearance]` / `[appearance.background]` /
  `[appearance.plate]` / `[appearance.reflect]` / `[layout]` / `[behavior]` /
  `[behavior.group]` / `[theme]` / `[widgets.clock]` / `[menu]` /
  `[advanced]`，取代上游的单 `[Settings]`。
- 键名改小写下划线。两处是修正真实歧义：
  `Mode` → `hide_mode`（原名看不出是"隐藏方式"，与 `BgMode` 混淆）、
  `BgMode` → `fill`（控制的是填充方式，不是开关）。
- 写盘带 `[meta] version=2`。
- 数值键（41 个，多对应未实现功能）**只分区不改名** —— 等做扇面/组时
  连同语义一起定，避免现在拍脑袋。

### 新增

- `Config::upgrade_legacy()`：读到上游格式就就地升级（删旧段 → 按新格式重写）。
- `Config::from_ini` 双格式兼容：新段 + 新键名优先，回退 `[Settings]` + 驼峰名。
- `Key::section()` / `Key::legacy_name()` / `Config::CONFIG_VERSION`。
- `from_name` 同时认新旧两套键名（手写配置、从 C 版抄来的片段都能用）。

### 测试（+8）

核心是 `legacy_and_new_formats_agree_on_every_key`：把**每个键**都改成
非默认值，分别用新旧格式写出读回、逐键比对 —— 键名映射错一位就会红。
另有升级不丢值、升级后不重复迁移、每个键的段名合法、数值键归类正确。

### 实测

上游格式 ini（`IconSize=64 Mode=2 BgMode=0 Edge=1 Opacity=80 Clock=1`）
启动后：日志「配置已从上游格式升级到 v2」，`S=64.0 Z=100.0 clockW=249`
全部取自旧值；落盘为 `[meta] version=2` + `[appearance] icon_size=64`，
`[Settings]` 消失，`[Items]` 保留；二次启动不重复迁移。

质量门禁：197 条测试、`clippy -D warnings`、`fmt --check` 全绿。

### 已知

- `write_ini` 此前从未被调用，现在也只有"升级"这一条路径会写。
  设置窗口接入后才有连续写回。
- 段顺序按首次出现排列，升级后的文件里 `[Items]` 会排在 `[meta]` 之前。
  不影响解析，等设置窗口落地时统一排序。

---

## 0.1.10 — 配置热更新（2026-10-30）

条目列表改动现在即时生效，不用重启。窗口尺寸从建窗起第一次变得可变。

### 新增

- **右键菜单的「从 Dock 移除」「添加分割线」「删除分割线」**。
  上一轮因为「改条目数 → 改窗口尺寸」没打通而留空，现在补齐。
- **窗口尺寸热更新**：`update()` 之后比对 `physical_size_i32()`，
  变了就 `frame.resize()` + `SetWindowPos` 重新居中。
- **图标按路径迁移**：`DockItem` 增加 `path`，替换列表时把已加载的
  `IconTex` 搬到新下标。没有它每次编辑都会让所有图标闪回"首字占位"。
- **最后一项不可删**：Dock 变空壳后用户没有办法恢复（拖入入口还没做），
  所以直接拒绝并打印原因。

### 实测

底边停靠、3 项、初始窗口 `724×186 @(918,1414)`：

| 操作 | 窗口 | ini |
|---|---|---|
| 移除「计算器」 | `664×186 @(948,1414)` | `Count=2` |
| 加分割线 | `678×186 @(941,1414)` | `Count=3`，`I1=3\|0\|\|\|\|0\|` |
| 删分割线 | `664×186 @(948,1414)` | `Count=2` |
| 移除「记事本」 | `604×186 @(978,1414)` | `Count=1` |
| 移除最后一项 | 不变 | 「这是最后一项，不能移除。」 |

每步截图确认：栏体完整居中、圆角未被裁、无残影、图标保持原纹理。

### 已知缺口

- **设置窗口仍未接入 Dock 进程**（`Overlay` 只在 spike 用过），
  所以「改设置项 → 即时生效」还差最后一环。
- 从资源管理器拖入新条目（需要 `WM_DROPFILES` / `IDropTarget`），
  会复用同一套热更新。

质量门禁：189 条测试、`clippy -D warnings`、`fmt --check` 全绿。

---

## 0.1.9 — 右键菜单（2026-10-30）

### 新增

- **右键菜单**（ADR-032）。独立分层窗口 + 模态 `SetCapture`，
  不抢焦点。`nooksill-gpu::menu` 负责项模型、布局与绘制（纯逻辑，15 条测试）；
  `bin/dock.rs` 负责建窗、模态循环与命令执行。
- 菜单项按条目类型生成：普通条目「打开 / 显示·最小化」、
  文件夹「在资源管理器中打开」、运行中的应用「退出应用」、
  空白处「退出 Dock」（红色警告色）。
- `FrameRenderer::resize()`：只重建离屏目标与 DIB，保留 egui 上下文
  与 wgpu 管线。菜单每次打开的高度都不同，没有它就得每次重编管线。
- `launch::open_folder()`：右键「在资源管理器中打开」用，
  与 `launch()` 分开——后者会把给应用用的参数一起传给 explorer。
- `NOOKSILL_MENU_TRACE=1`：打印菜单窗口收到的每条消息，
  排查"点了没反应"用。

### 实测

| 场景 | 结果 |
|---|---|
| 图标右键 | 菜单在光标上方 65 px 弹出，深色圆角面板、白字 |
| 点菜单项 | `cmd=Some(Open)` → `CalculatorApp` 启动 |
| 空白处右键 | 「退出 Dock」，红色文字 |
| 点「退出 Dock」 | 进程正常退出，AppBar 与单实例锁释放 |
| 点菜单外 | `cmd=None`，菜单收起 |

质量门禁：189 条测试、`clippy -D warnings`、`fmt --check` 全绿。

### 已知缺口

- 「从 Dock 移除」「添加分割线」「重命名」「设置图标」暂未提供：
  它们会改变条目数量进而改变栏体宽度与窗口尺寸，需要一整套
  「改配置 → 重建条目 → 重算几何 → 重建离屏目标」的热更新路径。
  分割线上右键因此**不弹菜单**，而不是弹一个空窗口。
- 菜单没有键盘导航（只有 Esc 取消）。
- 应用组 / 速记 / 文件夹扇面的子菜单。

---

## 0.1.8 — 补齐缺失的交互（2026-10-30）

### 新增

- **中键 / Shift+点击 = 强制新实例**：跳过"激活已运行窗口"，直接再开一个。
  原版 `LaunchItem(i, force)` 的同一语义。实测日志 `force=true`。
- **`Ctrl+Alt+D` 强制收起 / 恢复**：常显模式下也能用（大多数用户是 `Mode=0`）。
  用真实按键验证过，不是靠投递消息。
- **启动弹跳**：点击后图标沿栏体厚度方向弹两次并衰减。
  `Bounce` 配置项此前在设置界面里有、但 Dock 侧完全没实现。
  实测中键点击 170 ms 后图标顶端从 y=114 抬到 y=102。
- **全屏时隐藏**：前台是"覆盖整屏且无标题栏"的窗口时整体隐藏，
  退出全屏恢复。实测像素 `(70,70,70)` ↔ 全屏窗口 `(20,24,32)`，
  日志 `[fullscreen] 隐藏 Dock` / `恢复 Dock`。
- **系统主题切换立即生效**（`WM_SETTINGCHANGE` / `ImmersiveColorSet`）。

### 缺陷修复

- **点 Dock 抢焦点**：加 `WM_MOUSEACTIVATE → MA_NOACTIVATE`。
  此前点图标会先把焦点从用户正在用的窗口抢走，表现为"整个桌面闪一下"。
- **拖拽状态可能残留**：加 `WM_CAPTURECHANGED → cancel_drag()`。
  捕获被抢走（UAC / Alt+Tab）后 `press` 不清，下次点击会启动上一个图标。
- **快捷键恢复后 Dock 永久消失**：主循环推进条件漏了 `hide_cur > 0` 这个
  中间态。已收进 `Dock::needs_hide_tick()` 并加回归测试。

### 诊断

- `NOOKSILL_FS_TRACE=1` 打印全屏判定依据（前台类名 / 矩形 / 显示器 /
  是否有标题栏），与真实判定共用 `check_fullscreen()`，不会出现
  "诊断说该隐藏、实际没隐藏"的错位。

质量门禁：174 条测试、`clippy -D warnings`、`fmt --check` 全绿。
空闲 10 秒 CPU 增量仍为 0.000 秒（等待超时从 `INFINITE` 改为 1000 ms
以支持全屏检测，未影响 M0 验收）。

### 已知缺口

- **右键菜单**未实现（`WM_RBUTTONUP` 仍是空处理）。需要独立的菜单窗口，
  因为菜单会超出栏体范围，见下一轮。
- 拖出栏体删除、拖到图标上合并成组、组/速记弹窗、文件夹扇面。
- `WM_DPICHANGED` / `WM_DISPLAYCHANGE`：需要重建离屏纹理与 DIB。

---

## 0.1.7 — 运行状态与点击切换（2026-10-30）

### 新增

- **运行状态识别**（ADR-028）：`platform::app_window` 枚举顶层窗口并过滤出
  正常应用（排除工具窗、cloaked 窗口、桌面宿主等），得到"哪些 exe 在运行"。
  主循环每 1.2 s 扫描一次，只有状态真变化才重绘。
- **点击已运行的应用会切过去**，不再新开实例。`ClickToggle=1` 时行为更接近
  macOS：最小化则恢复、已在前台则最小化、被挡住则置前。
- **`.lnk` 解析**（ADR-029）：`platform::shortcut` 把快捷方式还原成真实 exe
  路径，启动时解析一次并缓存，供运行状态匹配使用。

### 缺陷修复

- **运行指示点无条件绘制**：`DockItem` 原本没有 `running` 字段，
  所有图标下方的指示点一直亮着，用户无法分辨哪个应用真的在运行。
  现在只在 `running` 为真时绘制。

### 已知局限

- Win11 的记事本 / 计算器 / 画图是 Store 应用，`notepad.exe` 只是执行别名，
  真实进程在 `WindowsApps` 下带版本号路径，匹配不上（指示点不亮、点击走新开，
  但系统会转发到已有实例）。要支持需解析 `AppExecutionAlias`。
- 原版 `ToggleApp` 的最小化/恢复会走 `FlyStart` 飞入动画（缩略图吸进 Dock），
  未实现；当前直接恢复/最小化。

质量门禁：163 条测试、`clippy -D warnings`、`fmt --check` 全绿。

---

## 0.1.6 — 桌面预留与智能隐藏（2026-10-30）

### 新增

- **桌面预留**（`Reserve=1`，ADR-026）：通过 AppBar 协议把屏幕边缘一条
  登记为系统级工具栏，最大化窗口不再盖住 Dock。`AppBar` 是 RAII 结构，
  析构时 `ABM_REMOVE` 归还工作区。实测 `rcWork` 底边从 1600 → 1508
  （少 92 px = `barH 76 + margin 12 + 4`）。
- **智能隐藏**（`Mode=2`，ADR-027）：只在 Dock 区域被普通窗口遮住时才收起。
  用 `EnumWindows` 枚举并逐条过滤工具窗、cloaked 窗口、桌面宿主等，
  结果与栏体屏幕矩形求交。

### 改进

- **四种停靠边全部可用**：`layout::window_origin` / `physical_size`
  取代原来硬编码的"底部居中"，左/右停靠会正确转置窗口尺寸。
  实测左侧停靠窗口 `356×724`、栏体竖直、图标纵向排列。
- `Dock.mode` 取代原来的 `auto_hide: bool`：两个 bool 会组合出
  "`auto_hide=false` 却 `smart=true`"这种不存在的状态。
- 遮挡判定可点名盖住者：`NOOKSILL_GEOM=1` 时打印
  `[hide] ... by "窗口标题"`，排查"为什么突然躲起来"不用靠猜。

### 已知缺口

- 运行时改配置仍需重启（M1 的自动保存接线）。
- 拖出栏体删除、拖到图标上合并成组、右键菜单、`ShowRunning`。

### 诊断工具

- `cargo run -q -p nooksill-core --example read_ini -- <ini 路径>`：
  同时打印解析后的枚举值与 ini 里的**原样字符串**，用于区分
  "配置没读到"（raw=None）与"读了但认不出"（raw=Some 但解析值是默认）。
  这轮靠它在 5 分钟内定位了一次「ini 被整份覆盖回默认值」的假 bug。
- 诊断模式下 `[hide]` 日志会点名盖住者：`covered=true by "窗口标题"`。

### 未验证

- `SmartHide` 的 `covered=false` 路径只有单元测试，没有真机端到端验证 ——
  验证期间桌面上始终有最大化窗口，构造不出"无人遮挡"的场景。

质量门禁：164 条测试、`clippy -D warnings`、`fmt --check` 全绿。

---

---

## 0.1.5 — 自动隐藏（2026-10-30）

### 新增

- **自动隐藏**（`Mode=1`）：鼠标离开 450 ms 后栏体滑出窗口，靠近屏幕
  所在边时立刻滑回。与原版 `OnAutoHide()` 同规则：
  收起态只认屏幕边缘，显示态认栏体外扩一圈（含放大溢出区）。
- 拖拽 / 菜单打开期间强制保持显示。

### 缺陷修复

- **收起后无法再弹出**：分层窗口的鼠标命中按 alpha 判定，栏体滑出后
  那片区域 alpha 为 0，事件全部穿透，永远等不到 `WM_MOUSEMOVE`。
  改为 100 ms 轮询 `GetCursorPos()`。
- **轮询导致空闲重绘**：主循环原本每轮都重绘，加轮询后会变成 10 fps
  的无谓渲染。改为 `need_frame` 按需出帧，实测收起态空闲 10 秒 CPU
  增量 0.000 秒。

质量门禁：137 条测试、`clippy -D warnings`、`fmt --check` 全绿。

### 已知缺口

- `SmartHide`（`Mode=2`，被遮挡才收起）未实现，当前按常显处理并在日志中说明。
- `Reserve`（为 Dock 预留桌面工作区，原版走 `SHAppBarMessage`）未接。

---

## 0.1.4 — 拖拽重排（2026-10-30）

### 新增

- **拖拽重排**：按住图标横向拖动即可调整顺序，松手后写回 `[Items]`。
  起拖阈值 6 px（原版行为），锁定状态（`Lock=1`）下不响应拖动。
- **跟手残影**：被拖的图标以 1.15× 尺寸、78% 不透明度跟着光标，
  原槽位让出来；拖动期间不显示名称气泡（会挡住残影）。
- **位移动画**：重排后其余图标从旧位置滑到新位置（`slide` 先补位移再衰减
  0.62，与原版同序）。

### 结构

- 拖拽状态收进 `DragState`（`index / pt / active / press_pt`），
  消灭"半截状态"。
- `Dock` 用 `pending_move` 把重排单向下发给主循环，主循环镜像到完整条目
  列表后写 ini。`Dock` 只持有名称与类型，无法独立完成持久化。

### 修复

- `place_for` 把被拖项自己也数进了"左边有几个"，导致每次多挪一格。
- `release_at` 在拖拽后仍产出点击 → 每次重排都会顺带启动一个应用。

质量门禁：133 条测试、`clippy -D warnings`、`fmt --check` 全绿。

### 已知缺口

- 拖出栏体松手不会删除条目（原版会，并显示"移除"提示）。
- 拖到另一个图标上悬停 220 ms 不会合并成组（组功能未实现）。

---

## 0.1.3 — 可点击的 Dock（2026-10-30）

在此之前 Dock 只是"画出来的"：点图标没有任何反应，悬停放大在动画收敛后
也不工作。这一轮把输入链路接通，并修掉三个"能跑但不工作"的缺陷。

### 新增

- **`platform::launch`**：`ShellExecuteW` 启动条目。分支与原版
  `LaunchPtr()` 一一对应：网址（`app_mode` 时走 Edge/Chrome 的 `--app`）/
  回收站 / 文件（工作目录设为文件所在目录）/ 文件夹。
  组与速记返回明确的"未实现"，不静默失败。
- **点击启动**：`Dock` 记录按下项，抬起时若命中同一项才算点击（原版语义）；
  真正的启动放在主循环，避免 `ShellExecuteW` 阻塞窗口过程。
- **`WM_MOUSELEAVE`**：之前 `hover` 一旦置真就再也不会清，鼠标移开后
  图标一直停在大尺寸。现在用 `TrackMouseEvent(TME_LEAVE)` 收离开通知。
- **Dock 单实例**：两个 Dock 会叠出两个分层窗口，而 `FindWindow` 只返回
  其中一个，调试时极具误导性。

### 缺陷修复

- **鼠标坐标单位错**：`mouse` 存逻辑点、槽位是物理像素，150% DPI 下命中
  整体偏移 1.5 倍（"鼠标移到图标右边才放大"）。统一为物理像素（ADR-023）。
- **静止后不再重布局**：主循环只在 `anim` 时调 `update`，而 `hot` 在
  `update` 里算 —— 动画一收敛，悬停放大就永久失效。改用 `dirty` 标记驱动。

### 验证

- 悬停：注入 `NOOKSILL_FAKE_MOUSE="302,134"`，放大的是第一个图标
  （记事本），气泡文字一致。
- 点击：向窗口投递 `WM_LBUTTONDOWN/UP`（`Dock` 单实例后只有一个窗口），
  日志 `[click] 第 1 项`，随后目标命令写出了标记文件。
- 启动层：`cargo test -p nooksill-platform --lib launch -- --ignored`
  真的启动一次 `cmd /c echo ok > 文件`，断言文件内容。

质量门禁：128 条测试、`clippy -D warnings`、`fmt --check` 全绿。

### 已知缺口

- 组 / 速记 / 文件夹扇面未实现（要等弹窗层）。
- 点击运行中的文件不会激活已有窗口（原版 `ActivateApp`），目前总是新起一个。
- 拖拽重排、右键菜单未实现。

---

## 0.1.2 — Dock 视觉修正与图标接入（2026-10-30）

### 真图标

- 新增 `platform::icon`：`HICON → RGBA`。三层兜底照抄原版 `HiconToGp`：
  彩色位图自带 alpha → 掩码造 alpha（老式 24 位图标）→ `DrawIconEx` 画进
  32 位 DIB。取图标先 JUMBO(256) 再 EXTRALARGE(48)。
- 新增 `core::icon`：适配区域计算（`FitMode` 三档 / `alpha_bbox` /
  `solid_bbox` / 圆角遮罩），全部纯函数、可单测。
  **`solid_bbox` 是必需的**：Win11 计算器这类图标整块不透明，只按 alpha
  求边界会得到一个深色大方块；原版会把同色底板当背景裁掉。
- `gpu::dock` 增加图标纹理缓存，按 bbox 取 UV 子矩形交给 GPU 缩放 ——
  放大动画每帧尺寸都变，CPU 预缩放会糊。

### 视觉缺陷修复

- **栏体填充是方角**（只有描边圆角）：`Shape::gradient_rect` 是 4 顶点
  四边形、没有半径。改为自建三角扇网格并补 1px 羽化（ADR-020）。
- **颜色参数顺序错**：多处照抄 C 的 `ARGBC(a,r,g,b)` 传给 Rust 的
  `(r,g,b,a)`，导致白色分割线变青绿、气泡底变半透明红、指示点外圈变纯红、
  底盘变粉白。新增 `argbc()` 适配层并逐条修正（ADR-021）。
- **Dock 主题取错来源**：用 `ui_is_dark`（设置窗口主题）当成 Dock 主题，
  两者枚举的 0/1 含义相反。改为 `dock_is_dark(theme::is_dark())`。
- **时钟少了年份**：横排误用了竖排的短日期。新增 `core::clock`，
  按 `ClockShow`/`DateFmt`/`ClockWeek`/`ClockOrder` 与停靠方向生成（ADR-022）。
- **时钟两行位置写死**：只显示时间或只显示日期时会错位。改为按行数均分，
  并在时间组与日期组之间画分割线。
- **运行指示点**几何对齐 v1.4.0（`S/12`、`0.34·S`、下限 3.6/7），
  外圈从"描边"改为原版的"向后扩 1px 的半透明黑填充"。

### 与 C 版几何对拍

本地编译上游 v1.4.0 后逐项比对，**所有度量完全相等**
（`lw=1181 lh=186 barH=76 over=96.6 margin=12 clockW=249`），
窗口屏幕矩形也相同。对拍细节与两个坑（C 版 DPI 感知、`ShowRunning`
临时条目）记在 `PORTING.md`。

质量门禁：122 条测试、`clippy -D warnings`、`fmt --check` 全绿。

### 已知缺口

- `ShowRunning`（显示未固定的运行中应用）未实现 —— 这也是"为什么 C 版
  屏幕上多几个图标"的原因。
- 网址 favicon 未实现，走首字占位。
- 自定义图标文件（`icon_src` 指向 png/jpg）未实现，缺图像解码器。

---

## 0.1.1 — 代码整理（2026-10-29）

没有新功能，只做去重、死代码清理与两处真实缺陷修复。质量门禁：
`cargo clippy -D warnings` / `cargo fmt --check` 全绿，100 条测试通过
（原 89 条，新增 12 条槽位布局单测，1 条吞吐 benchmark 标为 ignored）。

### 去重

- **新增 `nooksill-gpu/src/frame.rs`**：把"egui → 离屏纹理 → 回读 → 分层
  窗口"的提交路径抽成 `FrameRenderer`。`overlay.rs`（设置窗口）与
  `bin/dock.rs`（Dock 窗口）原先各写了一遍、逐行相同，任何 wgpu/egui 的
  API 变动都要改两处。`overlay.rs` 662 → 531 行，`bin/dock.rs` 350 → 300 行。
- **槽位布局收进 `core::layout`**：新增 `base_centers` / `magnify_targets` /
  `lerp_to` / `place_slots` / `bars` / `hit_test`，以及 `View`（鼠标 + 悬停 +
  隐藏进度）与 `SlotKind`。GPU 版与 GDI+ 版都改为调用同一份实现。
  `SIZE_LERP` / `SNAP` / `MAG_RADIUS` 三个常量也随之只有一个定义。
- `calc_metrics` 与两处栏宽累加合并成 `span()`：栏体跨度的公式原先出现三次。
- `gpu::dock::colorref_to_rgb` 删除，改用 `core::color::from_colorref`
  （另修了一处偏差：`bg_color2` 缺省时原来按 0.75 压暗，现在用 `color::mix`
  与 GDI+ 版一致）。
- `bar_colors` 的两个分支值完全相同（强调色尚未接 DWM），合并为一条。

### 修复

- **GDI+ 版基准槽位每帧漂移**：`step_layout` 用 `it.cur`（动画中的值）算
  基准宽度，图标放大时基准中心跟着动，`MapActualToBase` 的映射随之失真。
  现在与 GPU 版一样用固定的 `m.s`。
- **GDI+ 版时钟开启时栏体不左移**：`calc_metrics` 已经把 `clock_extra`
  算进逻辑宽度，但布局没减掉它，导致栏体 + 时钟卡片整体偏右、右端被裁。
  现在两版都传 `clock_extra` 给 `base_centers` / `place_slots`。
- **单实例互斥体名没走常量**：`single_instance.rs` 硬编码了
  `"Local\\NookSill_Wangcy_Singleton"`，旁边那句"编译期对齐检查"其实只断言了
  非空。改 `SINGLETON_MUTEX` 不会生效。现在真的用常量构造名字。
- `Key::range()` 去掉 `_ => (0, 1)` 兜底：新增键忘了给值域时会得到编译错误，
  而不是静默落进布尔域。
- `nooksill/src/main.rs` 启动日志里有两个写死的 `0`（占位残留），改为打印
  配置目录。

### 清理

- 删除死代码：`overlay.rs` 的 `PANEL_INSET` / `Layout::panel_rect` /
  `pixels_per_point()` / `layout_panel()`，`platform::theme::read_hkcu_dword`。
- 删除未使用的 `bytemuck` 依赖；`nooksill-gpu` 的三个 `[[bin]]` 改为显式声明。
  顺带修了一个不一致：文档一直写 `nooksill-dock`，但自动发现产出的其实叫
  `dock.exe`，按文档里的命令跑会找不到文件。
- `present.rs` 的 `swap_rb_throughput` 标 `#[ignore]`：它搬 114 MB 只为打印
  数字，是 benchmark 不是测试，不该拖慢每次 `cargo test`。
- `Dock::logical_size` 改名 `physical_size`（它返回的是物理像素）。
- `Dock::paint` 从 230 行拆成 `paint_bar_area` / `paint_clock_card` /
  `paint_separators` / `paint_icons` / `paint_icon` / `paint_running_dot` /
  `paint_hover_bubble`，坐标换算统一走 `Proj { k, tx, ty }`。

### 已知问题

- 本次重构**没有在真机上跑过**（无显示器环境）。像素级验收仍需按
  `PORTING.md` 的流程与 C 版并排截图比对，重点看四边停靠下的槽位与气泡位置。

---

## 0.1.0 — M0 尖刺（2026-10-27）

### 新增工作区

7 个 crate，依赖方向单向：`gpu → settings → core`、`gpu → platform`。

- `nooksill-core`：配置模型（76 键 + 夹紧）、INI 编解码（UTF-16LE/BE/UTF-8）、
  条目与分组序列化、布局数学、DPI 换算。**无 Win32 依赖**，48 条单测。
- `nooksill-platform`：路径（便携模式 + 迁移）、显示器枚举、系统深浅色、
  开机自启动、单实例互斥。含真机测试（显示器枚举）。
- `nooksill-settings`：egui 9 页设置界面，纯 UI 无副作用，产出 `Command`。
  5 条 kittest 无头测试（点击开关、页签切换、派生键映射、命令产出、全页渲染）。
- `nooksill-gpu`：wgpu 设备/表面、egui 提交、Win32 输入映射、
  `WM_NCHITTEST` 命中、消息循环。含 `nooksill-spike` 可执行。
- `nooksill-gfx` / `nooksill-ui` / `nooksill`：GDI+ 遗留实现，作行为参照（ADR-010）。

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

- 新增 `nooksill-gpu/src/dock.rs`：Dock 布局状态机 + egui 绘制。
  几何逐条照抄 C 版 v1.4.0 的 `Paint()`：栏体、时钟卡片、分割线、图标占位、
  运行指示点、悬停气泡、悬停放大。
- 新增可执行文件 `nooksill-dock`：真实 Dock 形态的窗口（不是设置窗），
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
- 新增 `cargo run -p nooksill-core --example config_table`：
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
- 新增 `NOOKSILL_TRACE=1`：每秒打印帧数、hover 位置、egui 的重绘原因，
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
- `nooksill-platform::gpu`：DXGI 枚举适配器（名字 / 厂商 / 显存 / 类型），
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
- `nooksill-gpu` debug 产物 43 MB；release **7.2 MB**。
- 常驻内存 271 MB；同卡空设备基线 dx12 164 MB / vulkan 103 MB / d3d11 34 MB
  （AMD 610M）。ADR-001 复核已执行完毕，结论见 ADR-011（已否决）/ ADR-012。
- 首帧与稳态画面不同（首帧字体/布局尚未稳定），截图核对必须等第二帧之后。
