# 变更记录

格式：里程碑为单位，每条写"做了什么 / 为什么 / 已知问题"。
版本号暂不与里程碑绑定（0.1.0 覆盖 M0）。

---

## 0.9.1 — 设置界面第二轮：顶栏搜索 + 说明气泡（2026-11-03）

把 `docs/settings-prototype.html` 第二轮的设计移植进 Rust。仍然没有新功能；
门禁：clippy `-D warnings` / `fmt --check` 全绿，315 条测试通过
（设置 crate 13 条，其中 2 条是本轮新增的契约测试）。

### 界面（照雏形逐项移植）

- **窗口 893 × 720**（导航栏 193 + 内容区 700），`LOGICAL_SIZE` 更新。
- **顶栏三栏**：左 `MiniDock`（20px，去掉「设置」二字）；中搜索框
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
  （`cargo run -p minidock-gpu --example settings_shot -- 出图.bmp 外观 dark`）。
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

截图要 wgpu，而 `minidock-settings` 刻意不依赖 wgpu。所以"画在哪"用矩形
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
- 探针：`cargo run -p minidock-platform --example activate_probe -- notepad.exe`。
- 诊断：`MINIDOCK_LAUNCH_TRACE=1` 打印每次点击的落点（激活/启动）与结果。
- 扫描按需：`ScanInput::want_running`，指示点全关时不再每 1.2 秒枚举窗口。

### 测试

`click_survives_release_capture`、`lost_capture_drops_the_click`、
`temp_item_has_no_structural_commands`、`temp_*` 五条。

---

## 0.8.5 — 环境烟雾（ADR-048，2026-11-03）

栏体周围加了一层**真的在流动**的烟：上升、卷曲、漂过边缘、随鼠标让开。
算法移植自 Windhawk 的 *Taskbar Smoke*（Rubicus），渲染接到 egui。

### 新增

- `minidock-core::smoke`：可平铺的域扭曲 FBM 噪声、Stam 稳定流体
  （浮力 / 涡量约束 / 压力投影 / 位移场）、网格 → 预乘 RGBA 光栅化。
- `minidock-gpu::smoke`：按真实帧间隔驱动模拟、上传纹理、画在栏体之下；
  另有一层"上升的小烟团"粒子。
- `[effect.smoke]` 段十一个键（CONFIG.md 第三节），设置界面新增「效果」页。
- 诊断：`MINIDOCK_SMOKE_TRACE=1` 打印网格 / 位图 / 最大 alpha / 栏体矩形。
- `cargo run -p minidock-core --example smoke_dump` 把一帧合成到深色背景上
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

- `MINIDOCK_LP_TRACE=1`：打印起动台的右键命中与菜单项执行。

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
- **`MINIDOCK_GEOM=1` 之外新增 `examples/calendar`**：命令行打印某月的网格
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
- **`MINIDOCK_GEOM=1` 新增一行** `GEOM widget=... screen=(x,y) size=(w,h)`，
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

- **`minidock-gpu::worker`：四个后台线程**，各自一条 `std::sync::mpsc` 通道。

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
- **调试开关收进 `trace` 模块**：原先 16 处 `std::env::var_os("MINIDOCK_GEOM")`
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
  打开一个独立 HWND 的分层窗口，渲染 `minidock-settings` 的 9 页 UI。
  同一份 `Config` 被两边共用，改一个滑块下一帧就在栏体上生效，没有 IPC、没有复制。
- **非模态**：不像菜单那样抢走消息循环，可以一边调参数一边看栏体的变化。
- **配置自动落盘**：改动后 400 ms 防抖写回 ini（拖滑块不会逐帧重写文件），
  写之前重新读一遍以保住 `[Items]` 段。
- **停靠边 / 预留 / 隐藏模式变化会重挂**：重新注册 AppBar 并按新边重算窗口位置，
  尺寸变化走已有的热更新路径（ADR-033）。
- **开机自启动开关**（命令 `ToggleAutostart`）读写 `HKCU\...\Run\MiniDock`；
  显示的是注册表真值，不是缓存的配置。
- **推荐设置 / 低占用**两组预设，以及"在这一页加分割线 / 回收站"入口。
- `MINIDOCK_SETTINGS_TRACE=1`：打印设置窗口的窗口消息流与交给 egui 的输入事件
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

`minidock` / `minidock-gfx` / `minidock-ui` 三个 crate 删除，共 **2000 行**
（占当时总代码 12%）。workspace 从 7 个 crate 减到 4 个。

### 删除

| 路径 | 行数 | 内容 |
|---|---|---|
| `crates/minidock` | 83 | 老的 bin 入口（`MiniDock.exe`） |
| `crates/minidock-gfx` | 969 | GDI+ 封装（canvas / objects / 字体） |
| `crates/minidock-ui` | 948 | GDI+ 版 Dock 骨架 |

它们是 M0 之前的一次移植尝试，**从来不是行为基准**（基准一直是上游 C 版
源码）。ADR-010 当初留作 fallback，现在 wgpu 路线已跑通、两份实现开始漂移
（`layout.rs` 里记着一例基准宽度算法分歧），且 `cargo build` 会产出两个
可交互的 Dock，调试时先要确认跑的是哪个 —— 成本每次都在付。

### 可见变化

- **可执行文件名只剩 `minidock-dock.exe`**（原来的 `MiniDock.exe` 一并删除）。
- `Cargo.lock` 清理干净，`cargo build --workspace` 少编三个 crate。

### 未做

- 不留 `--features legacy-gdi` 开关：会让"删干净"永远差一步，
  CI 与测试都要为不存在的路径付成本。
- 不另开 `archive/` 分支：git 历史里有，需要时
  `git checkout <sha> -- crates/minidock-ui` 就能取回。

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
  不抢焦点。`minidock-gpu::menu` 负责项模型、布局与绘制（纯逻辑，15 条测试）；
  `bin/dock.rs` 负责建窗、模态循环与命令执行。
- 菜单项按条目类型生成：普通条目「打开 / 显示·最小化」、
  文件夹「在资源管理器中打开」、运行中的应用「退出应用」、
  空白处「退出 Dock」（红色警告色）。
- `FrameRenderer::resize()`：只重建离屏目标与 DIB，保留 egui 上下文
  与 wgpu 管线。菜单每次打开的高度都不同，没有它就得每次重编管线。
- `launch::open_folder()`：右键「在资源管理器中打开」用，
  与 `launch()` 分开——后者会把给应用用的参数一起传给 explorer。
- `MINIDOCK_MENU_TRACE=1`：打印菜单窗口收到的每条消息，
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
