# 变更记录

格式：里程碑为单位，每条写"做了什么 / 为什么 / 已知问题"。
版本号暂不与里程碑绑定（0.1.0 覆盖 M0）。

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

- `MINIDOCK_FS_TRACE=1` 打印全屏判定依据（前台类名 / 矩形 / 显示器 /
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
- 遮挡判定可点名盖住者：`MINIDOCK_GEOM=1` 时打印
  `[hide] ... by "窗口标题"`，排查"为什么突然躲起来"不用靠猜。

### 已知缺口

- 运行时改配置仍需重启（M1 的自动保存接线）。
- 拖出栏体删除、拖到图标上合并成组、右键菜单、`ShowRunning`。

### 诊断工具

- `cargo run -q -p minidock-core --example read_ini -- <ini 路径>`：
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

- 悬停：注入 `MINIDOCK_FAKE_MOUSE="302,134"`，放大的是第一个图标
  （记事本），气泡文字一致。
- 点击：向窗口投递 `WM_LBUTTONDOWN/UP`（`Dock` 单实例后只有一个窗口），
  日志 `[click] 第 1 项`，随后目标命令写出了标记文件。
- 启动层：`cargo test -p minidock-platform --lib launch -- --ignored`
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

- **新增 `minidock-gpu/src/frame.rs`**：把"egui → 离屏纹理 → 回读 → 分层
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
  `"Local\\MiniDock_Wangcy_Singleton"`，旁边那句"编译期对齐检查"其实只断言了
  非空。改 `SINGLETON_MUTEX` 不会生效。现在真的用常量构造名字。
- `Key::range()` 去掉 `_ => (0, 1)` 兜底：新增键忘了给值域时会得到编译错误，
  而不是静默落进布尔域。
- `minidock/src/main.rs` 启动日志里有两个写死的 `0`（占位残留），改为打印
  配置目录。

### 清理

- 删除死代码：`overlay.rs` 的 `PANEL_INSET` / `Layout::panel_rect` /
  `pixels_per_point()` / `layout_panel()`，`platform::theme::read_hkcu_dword`。
- 删除未使用的 `bytemuck` 依赖；`minidock-gpu` 的三个 `[[bin]]` 改为显式声明。
  顺带修了一个不一致：文档一直写 `minidock-dock`，但自动发现产出的其实叫
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
