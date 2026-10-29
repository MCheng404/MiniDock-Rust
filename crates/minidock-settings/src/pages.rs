//! 设置界面：9 页、纯 egui、无窗口/GPU 依赖。
//!
//! 设计原则只有一条：**控件的取值范围、键名、夹紧规则全部来自 `core::config`**，
//! 这里只负责"长什么样"。这样加一个设置项只改一处（core 的 `Key` 表），
//! 而不是原版那样在 建表 / 绘制 / 命中 / 命令 四处同步修改。
//!
//! 也因此这个模块可以在无显示器环境被 kittest 完整驱动：
//! 没有 HWND、没有 device、没有线程。

use egui::{RichText, Ui};
use minidock_core::color;
use minidock_core::config::{BgMode, Config, Key};

/// 九个页面。顺序 = 侧边导航顺序 = 原版 `BuildRows()` 的 P0..P9。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    General,
    Appearance,
    Position,
    Icons,
    Interaction,
    Extras,
    Performance,
    Library,
    Add,
}

impl Page {
    pub const ALL: [Page; 9] = [
        Page::General,
        Page::Appearance,
        Page::Position,
        Page::Icons,
        Page::Interaction,
        Page::Extras,
        Page::Performance,
        Page::Library,
        Page::Add,
    ];

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Page::General => "常规",
            Page::Appearance => "外观",
            Page::Position => "位置",
            Page::Icons => "图标",
            Page::Interaction => "交互",
            Page::Extras => "附件",
            Page::Performance => "性能",
            Page::Library => "图标库",
            Page::Add => "添加",
        }
    }
}

/// UI 层私有的状态。**不属于** `Config`：页面位置、显示器名字、内存统计
/// 都不该进 ini。
#[derive(Clone, Debug)]
pub struct SettingsState {
    pub page: Page,
    /// 显示器名字列表，由调用方（platform 层）填。
    pub monitors: Vec<String>,
    /// 开机自启动的当前实际状态（注册表里的真值，不是配置项）。
    pub auto_start: bool,
    pub config_dir: String,
    /// 性能页显示的统计文本，由 GPU 层每帧刷新。
    pub stats: String,
    /// 可选适配器（不含"自动"）。由 platform 层枚举后填入。
    pub adapters: Vec<String>,
    /// 当前**实际**在用的渲染器名字。由 GPU 层填，用户不能直接改它。
    pub active_adapter: String,
    /// 配置请求的适配器与实际情况不一致时的提示（空 = 无异常）。
    pub adapter_note: String,
    /// 底部状态条。
    pub status: String,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            page: Page::General,
            monitors: vec!["主显示器".to_owned()],
            auto_start: false,
            config_dir: String::new(),
            stats: "尚无采样".to_owned(),
            adapters: Vec::new(),
            active_adapter: String::new(),
            adapter_note: String::new(),
            status: String::new(),
        }
    }
}

/// 需要主程序执行的动作（打开对话框、写注册表、增删条目……）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Close,
    ResetDefaults,
    LowPower,
    Recommended,
    OpenConfigDir,
    ToggleAutostart(bool),
    PickIconDir,
    AutoMatch,
    AddUrl,
    AddFile,
    AddFolder,
    NewGroup,
    AddSeparator,
    AddTrash,
    ExportAll,
    Import,
}

/// 一帧的结果。
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub changed: bool,
    pub command: Option<Command>,
}

/// 控件用的枚举选项表（值 = ini 里的整数码）。
const THEME_OPTS: &[(i32, &str)] = &[(0, "深色"), (1, "浅色"), (2, "跟随系统")];
/// 注意：`UiTheme` 的 0/1 与 `DockTheme` **相反**（历史遗留，见 core 文档）。
const UI_THEME_OPTS: &[(i32, &str)] = &[(0, "浅色"), (1, "深色"), (2, "跟随系统")];
const BG_OPTS: &[(i32, &str)] = &[(0, "纯色"), (1, "渐变"), (2, "磨砂玻璃")];
const EDGE_OPTS: &[(i32, &str)] = &[
    (0, "屏幕底部"),
    (1, "屏幕顶部"),
    (2, "屏幕左侧"),
    (3, "屏幕右侧"),
];
const MODE_OPTS: &[(i32, &str)] = &[(0, "常驻显示"), (1, "自动隐藏"), (2, "智能隐藏")];
// 底盘选项：上游 v1.4.0 把旧的 `PlateDark`/`PlateLight` 两键
// 换成了「类型 + 配色 + 不透明度」三键。
const PLATE_OPTS: &[(i32, &str)] = &[(0, "无底盘"), (1, "纯色底盘"), (2, "半透明底盘")];
const PLATE_THEME_OPTS: &[(i32, &str)] = &[(0, "浅色"), (1, "深色"), (2, "跟随 Dock 主题")];
const DOT_OPTS: &[(i32, &str)] = &[(0, "横条"), (1, "圆点")];
// 上游 v1.4.0 新增的选项表（文案照抄上游设置行）。
const MENU_THEME_OPTS: &[(i32, &str)] = &[(0, "浅色"), (1, "深色"), (2, "跟随系统")];
const MIN_STYLE_OPTS: &[(i32, &str)] = &[
    (0, "压缩飞入（类似 macOS）"),
    (1, "弹性飞入（类似手机，非线性）"),
];
const CLOCK_SHOW_OPTS: &[(i32, &str)] = &[
    (0, "时间 + 日期（上下分栏）"),
    (1, "仅显示时间"),
    (2, "仅显示日期"),
];
const DATE_FMT_OPTS: &[(i32, &str)] =
    &[(0, "2026年10月22日"), (1, "2026/10/22"), (2, "2026-10-22")];
const BROWSER_OPTS: &[(i32, &str)] = &[
    (0, "自动（Edge 优先）"),
    (1, "Microsoft Edge"),
    (2, "Google Chrome"),
    (3, "Mozilla Firefox"),
    (4, "系统默认浏览器"),
];

/// 渲染整个设置界面（顶部页签 + 内容 + 状态条）。
pub fn settings_ui(ui: &mut Ui, cfg: &mut Config, state: &mut SettingsState) -> Outcome {
    let mut ctx = Ctx {
        cfg,
        out: Outcome::default(),
    };

    ui.horizontal_wrapped(|ui| {
        for page in Page::ALL {
            if ui
                .selectable_label(state.page == page, page.title())
                .clicked()
            {
                state.page = page;
            }
        }
    });
    ui.separator();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match state.page {
            Page::General => ctx.page_general(ui, state),
            Page::Appearance => ctx.page_appearance(ui),
            Page::Position => ctx.page_position(ui, state),
            Page::Icons => ctx.page_icons(ui),
            Page::Interaction => ctx.page_interaction(ui),
            Page::Extras => ctx.page_extras(ui),
            Page::Performance => ctx.page_performance(ui, state),
            Page::Library => ctx.page_library(ui),
            Page::Add => ctx.page_add(ui),
        });

    ui.separator();
    if state.status.is_empty() {
        ui.small("空闲时不重绘；所有设置项即时生效并自动保存。");
    } else {
        ui.small(&state.status);
    }
    ctx.out
}

struct Ctx<'a> {
    cfg: &'a mut Config,
    out: Outcome,
}

impl Ctx<'_> {
    fn toggle(&mut self, ui: &mut Ui, key: Key, label: &str, note: &str) {
        let mut v = self.cfg.get_i32(key) != 0;
        if ui.checkbox(&mut v, label).changed() {
            self.cfg.apply_i32(key, i32::from(v));
            self.out.changed = true;
        }
        if !note.is_empty() {
            ui.label(RichText::new(note).small().weak());
        }
    }

    fn slider(&mut self, ui: &mut Ui, key: Key, label: &str, suffix: &str) {
        let (lo, hi) = key.slider_range().unwrap_or_else(|| key.range());
        let mut v = self.cfg.get_i32(key);
        if ui
            .add(
                egui::Slider::new(&mut v, lo..=hi)
                    .text(label)
                    .suffix(suffix),
            )
            .changed()
        {
            self.cfg.apply_i32(key, v);
            self.out.changed = true;
        }
    }

    fn combo(&mut self, ui: &mut Ui, key: Key, label: &str, opts: &[(i32, &str)]) {
        let before = self.cfg.get_i32(key);
        let mut cur = before;
        ui.horizontal(|ui| {
            // 标签与下拉框必须显式关联：egui 不会自动把相邻 label 当作
            // 控件的 accessible name，kittest 的 a11y 检查会直接判失败。
            let label_resp = ui.label(label);
            let text = opts
                .iter()
                .find(|(v, _)| *v == cur)
                .map_or("—", |(_, t)| *t);
            let combo = egui::ComboBox::from_id_salt(key.name())
                .selected_text(text)
                .show_ui(ui, |ui| {
                    for (v, t) in opts {
                        ui.selectable_value(&mut cur, *v, *t);
                    }
                });
            combo.response.labelled_by(label_resp.id);
        });
        if cur != before {
            self.cfg.apply_i32(key, cur);
            self.out.changed = true;
        }
    }

    /// 显卡选择。与 `combo` 的区别：值是**字符串**，且不走 `Key` 表
    /// （`[Graphics]` 段是 Rust 版独有的，见 ADR-012）。
    fn adapter_combo(&mut self, ui: &mut Ui, state: &mut SettingsState) {
        let mut cur = self.cfg.adapter.clone();
        ui.horizontal(|ui| {
            let label_resp = ui.label("显卡");
            let text = if cur.is_empty() {
                "自动（推荐）"
            } else {
                cur.as_str()
            };
            let combo = egui::ComboBox::from_id_salt("gpu_adapter")
                .selected_text(text)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut cur, String::new(), "自动（推荐）");
                    for name in &state.adapters {
                        ui.selectable_value(&mut cur, name.clone(), name);
                    }
                });
            combo.response.labelled_by(label_resp.id);
        });

        if cur != self.cfg.adapter {
            self.cfg.adapter = Config::sanitize_adapter(&cur);
            self.out.changed = true;
        }

        // 实际生效的那块卡与配置可能不同（名字对不上会回退），必须显示真值，
        // 否则用户会以为"选了就生效了"。
        if state.active_adapter.is_empty() {
            ui.small("当前实际使用：尚未初始化");
        } else {
            ui.small(format!("当前实际使用：{}", state.active_adapter));
        }
        if !state.adapter_note.is_empty() {
            ui.colored_label(ui.visuals().warn_fg_color, &state.adapter_note);
        }
        if state.adapters.is_empty() {
            ui.label(
                RichText::new("没有枚举到可用显卡，只能用自动。")
                    .small()
                    .weak(),
            );
        }
    }

    fn button(&mut self, ui: &mut Ui, label: &str, cmd: Command) {
        if ui.button(label).clicked() {
            self.out.command = Some(cmd);
        }
    }

    /// 自定义颜色：`-1` = 未启用，否则是 COLORREF。
    fn color_row(&mut self, ui: &mut Ui, key: Key, label: &str) {
        let cur = self.cfg.get_i32(key);
        let mut enabled = cur >= 0;
        ui.horizontal(|ui| {
            if ui.checkbox(&mut enabled, label).changed() {
                let v = if enabled { 0x0033_6699 } else { -1 };
                self.cfg.apply_i32(key, v);
                self.out.changed = true;
            }
            if enabled {
                let c = color::from_colorref(cur.max(0) as u32);
                let mut arr = [color::red(c), color::green(c), color::blue(c)];
                if ui.color_edit_button_srgb(&mut arr).changed() {
                    let v = color::to_colorref(color::rgb(arr[0], arr[1], arr[2])) as i32;
                    self.cfg.apply_i32(key, v);
                    self.out.changed = true;
                }
            }
        });
    }

    fn page_general(&mut self, ui: &mut Ui, state: &mut SettingsState) {
        ui.heading("启动与常规");
        let mut auto = state.auto_start;
        if ui.checkbox(&mut auto, "开机自启动").changed() {
            state.auto_start = auto;
            self.out.command = Some(Command::ToggleAutostart(auto));
        }
        self.toggle(ui, Key::Lock, "锁定 Dock 布局", "防止误拖动、误移除图标");
        self.toggle(
            ui,
            Key::ShowRunning,
            "显示未固定的运行中应用",
            "在分割线之后显示正在运行的应用，可拖入固定",
        );
        self.toggle(
            ui,
            Key::Hotkey,
            "全局快捷键 Ctrl + Alt + D",
            "随时显示 / 隐藏 Dock",
        );
        self.toggle(
            ui,
            Key::WebAppMode,
            "网页默认以应用窗口打开",
            "使用 Edge / Chrome 的 --app 模式",
        );

        ui.add_space(10.0);
        ui.heading("设置界面");
        self.combo(ui, Key::UiTheme, "设置窗口主题", UI_THEME_OPTS);

        ui.add_space(10.0);
        ui.heading("数据");
        ui.horizontal(|ui| {
            ui.label("配置目录");
            ui.monospace(&state.config_dir);
        });
        self.button(ui, "打开配置文件夹", Command::OpenConfigDir);
    }

    fn page_appearance(&mut self, ui: &mut Ui) {
        ui.heading("尺寸");
        self.slider(ui, Key::IconSize, "图标大小", " px");
        self.slider(ui, Key::ZoomSize, "放大后大小", " px");
        self.slider(ui, Key::Spacing, "图标间距", " px");
        self.slider(ui, Key::Padding, "栏内边距（栏高）", " px");
        self.slider(ui, Key::Radius, "栏圆角", " px");

        ui.add_space(10.0);
        ui.heading("背景");
        self.combo(ui, Key::Theme, "Dock 主题", THEME_OPTS);
        self.combo(ui, Key::BgMode, "背景材质", BG_OPTS);
        self.slider(ui, Key::Opacity, "不透明度", " %");
        self.slider(ui, Key::Blur, "磨砂模糊强度", "");
        self.toggle(ui, Key::Border, "显示栏边框", "");
        self.toggle(
            ui,
            Key::AccentTone,
            "跟随系统强调色（纯色 / 渐变）",
            "根据 Windows 强调色生成 Material You 风格色调",
        );
        self.color_row(ui, Key::BgColor1, "自定义颜色 1");
        self.color_row(ui, Key::BgColor2, "自定义颜色 2（渐变终点）");
        self.color_row(ui, Key::BG_COLOR3, "自定义颜色 3（渐变中间色，可选）");
        self.slider(ui, Key::GRAD_A1, "颜色 1 不透明度", " %");
        self.slider(ui, Key::GRAD_A2, "颜色 2 不透明度", " %");
        self.slider(ui, Key::GRAD_A3, "颜色 3 不透明度", " %");
        self.slider(ui, Key::GRAD_ANGLE, "渐变方向", "°");
        self.slider(ui, Key::BORDER_WIDTH, "边框粗细", " px");
        self.combo(ui, Key::MENU_THEME, "右键菜单主题", MENU_THEME_OPTS);
    }

    fn page_position(&mut self, ui: &mut Ui, state: &mut SettingsState) {
        ui.heading("位置");
        self.combo(ui, Key::Edge, "Dock 位置（居中）", EDGE_OPTS);

        // 显示器列表是运行期信息，选项动态生成。
        let before = self.cfg.monitor;
        let mut cur = before;
        ui.horizontal(|ui| {
            let label_resp = ui.label("显示在");
            let text = state
                .monitors
                .get(cur.max(0) as usize)
                .cloned()
                .unwrap_or_else(|| format!("显示器 {}", cur + 1));
            let combo = egui::ComboBox::from_id_salt("monitor")
                .selected_text(text)
                .show_ui(ui, |ui| {
                    for (i, name) in state.monitors.iter().enumerate() {
                        ui.selectable_value(&mut cur, i as i32, name);
                    }
                });
            combo.response.labelled_by(label_resp.id);
        });
        if cur != before {
            self.cfg.apply_i32(Key::Monitor, cur);
            self.out.changed = true;
        }

        self.slider(ui, Key::Margin, "距屏幕边缘", " px");

        ui.add_space(10.0);
        ui.heading("显示模式");
        self.combo(ui, Key::Mode, "模式", MODE_OPTS);
        self.toggle(
            ui,
            Key::Reserve,
            "最大化窗口避让 Dock",
            "仅「常驻显示」模式有效，像 macOS 一样为 Dock 预留桌面空间",
        );
    }

    fn page_icons(&mut self, ui: &mut Ui) {
        ui.heading("图标底盘（macOS 风格圆角方块）");
        self.combo(ui, Key::PLATE_MODE, "底盘类型", PLATE_OPTS);
        self.combo(ui, Key::PLATE_THEME, "底盘配色", PLATE_THEME_OPTS);
        self.slider(ui, Key::PLATE_OPACITY, "底盘不透明度", " %");
        self.slider(ui, Key::PlateRadius, "底盘圆角", " %");

        ui.add_space(10.0);
        ui.heading("分割线");
        self.slider(ui, Key::SEP_WIDTH, "分割线宽度", " px");
        self.slider(ui, Key::SEP_GAP, "分割线间距", " px");
        ui.label(
            RichText::new("圆角 0% = 方形，22% = 默认圆角方块，50% = 圆形。")
                .small()
                .weak(),
        );

        ui.add_space(10.0);
        ui.heading("运行指示");
        self.toggle(ui, Key::Dots, "显示运行中指示", "");
        self.combo(ui, Key::DotStyle, "指示样式", DOT_OPTS);

        ui.add_space(10.0);
        ui.heading("标签");
        self.toggle(ui, Key::Labels, "悬停显示名称标签", "");
    }

    fn page_interaction(&mut self, ui: &mut Ui) {
        ui.heading("交互");
        self.toggle(ui, Key::Zoom, "悬停放大图标", "");
        self.toggle(
            ui,
            Key::FolderFan,
            "点击文件夹时弧形堆叠展开",
            "像 macOS 一样列出最近的文件，点击即可打开",
        );

        ui.add_space(10.0);
        ui.heading("动画");
        self.combo(ui, Key::MIN_STYLE, "最小化动画风格", MIN_STYLE_OPTS);
        self.toggle(ui, Key::Bounce, "启动应用时弹跳", "");
        self.toggle(
            ui,
            Key::MinAnim,
            "最小化缩入 Dock 动画",
            "窗口最小化时缩小飞入对应图标",
        );
        self.toggle(ui, Key::Preview, "悬停运行中应用显示窗口预览", "");
    }

    fn page_extras(&mut self, ui: &mut Ui) {
        ui.heading("Dock 附件");
        self.toggle(
            ui,
            Key::Clock,
            "在 Dock 右侧显示时间与日期",
            "每 30 秒刷新一次，占用极低",
        );
        self.toggle(ui, Key::Reflect, "图标倒影", "仅底部停靠时有效");
        self.slider(ui, Key::REFL_SIZE, "倒影大小", " %");
        self.slider(ui, Key::REFL_GAP, "倒影与 Dock 的距离", " px");
        self.slider(ui, Key::REFL_OPACITY, "倒影不透明度", " %");

        ui.add_space(10.0);
        self.combo(ui, Key::BROWSER, "网页使用的浏览器", BROWSER_OPTS);

        ui.heading("小工具");
        self.toggle(
            ui,
            Key::Notes,
            "速记（点击即可记录文字）",
            "开启后 Dock 上出现「速记」图标",
        );
    }

    fn page_performance(&mut self, ui: &mut Ui, state: &mut SettingsState) {
        ui.heading("图形");
        self.adapter_combo(ui, state);
        ui.label(
            RichText::new("自动：由后端按功耗偏好挑选，通常落在核显上。改动在下次启动生效。")
                .small()
                .weak(),
        );

        ui.add_space(10.0);
        ui.heading("当前占用");
        ui.monospace(&state.stats);

        ui.add_space(10.0);
        ui.heading("高耗能功能（可单独关闭）");

        // 磨砂是 BgMode 的派生状态，不是独立布尔键。
        let mut frost = self.cfg.bg_mode == BgMode::Frost;
        if ui.checkbox(&mut frost, "磨砂玻璃背景").changed() {
            self.cfg.apply_i32(Key::BgMode, if frost { 2 } else { 1 });
            self.out.changed = true;
        }
        ui.label(
            RichText::new("中等：桌面捕获 + GPU 模糊，每 2.5 秒一次")
                .small()
                .weak(),
        );

        let mut scan = self.cfg.scan_enabled();
        if ui
            .checkbox(&mut scan, "扫描运行中应用（指示点 / 运行中列表）")
            .changed()
        {
            self.cfg.apply_i32(Key::Dots, i32::from(scan));
            self.cfg.apply_i32(Key::ShowRunning, i32::from(scan));
            self.out.changed = true;
        }
        ui.label(
            RichText::new("中等：每 2.5 秒枚举窗口；关闭后几乎不做后台扫描")
                .small()
                .weak(),
        );

        self.toggle(
            ui,
            Key::Preview,
            "悬停窗口预览",
            "低：仅悬停时使用系统缩略图",
        );
        self.toggle(ui, Key::Clock, "时间与日期", "低：每 30 秒刷新一次");
        self.combo(ui, Key::CLOCK_SHOW, "时钟显示内容", CLOCK_SHOW_OPTS);
        self.combo(ui, Key::DATE_FMT, "日期格式", DATE_FMT_OPTS);
        self.toggle(ui, Key::CLOCK_WEEK, "日期显示周几", "");
        self.toggle(ui, Key::CLOCK_SEC, "时间显示秒数", "每秒刷新一次，占用略增");
        self.toggle(ui, Key::Reflect, "图标倒影", "低：多画一层半透明图像");
        self.toggle(
            ui,
            Key::MinAnim,
            "最小化缩入动画",
            "低：仅在窗口最小化时运行",
        );

        ui.add_space(10.0);
        ui.heading("系统与兼容");
        self.toggle(
            ui,
            Key::GUARDIAN,
            "防止被清理工具关闭",
            "启动一个极小的守护进程，被结束后自动重开",
        );
        self.toggle(
            ui,
            Key::CLICK_TOGGLE,
            "点击运行中的应用 = 最小化 / 恢复",
            "窗口在最前时再次点击会缩回 Dock",
        );
        self.toggle(ui, Key::MN_AUTO, "右键菜单显示开机自启开关", "");
        self.toggle(ui, Key::MN_NOTES, "右键菜单显示速记开关", "");
        self.toggle(ui, Key::MN_ADD, "空白处右键显示新建项", "");

        ui.add_space(10.0);
        ui.heading("预设");
        ui.horizontal(|ui| {
            self.button(ui, "一键低占用模式", Command::LowPower);
            self.button(ui, "恢复推荐设置", Command::Recommended);
        });
        ui.label(
            RichText::new("空闲时没有常驻动画定时器，只在交互时提交 GPU 帧。")
                .small()
                .weak(),
        );
    }

    fn page_library(&mut self, ui: &mut Ui) {
        ui.heading("从文件夹批量设置图标");
        ui.horizontal(|ui| {
            self.button(ui, "选择图标文件夹…", Command::PickIconDir);
            self.button(ui, "自动匹配并应用", Command::AutoMatch);
        });
        ui.label(
            RichText::new(
                "选择存放图标（png / ico / jpg）的文件夹，文件名与应用名称相同即可自动匹配。",
            )
            .small()
            .weak(),
        );
        ui.add_space(10.0);
        ui.label(RichText::new("条目列表在 M1 接入（需要 core::store 的编辑能力）。").weak());
    }

    fn page_add(&mut self, ui: &mut Ui) {
        ui.heading("添加");
        self.button(ui, "添加网页应用…", Command::AddUrl);
        self.button(ui, "添加应用 / 文件…", Command::AddFile);
        self.button(ui, "添加文件夹…", Command::AddFolder);
        self.button(ui, "新建应用组", Command::NewGroup);
        self.button(ui, "添加回收站", Command::AddTrash);
        self.button(ui, "添加分割线", Command::AddSeparator);

        ui.add_space(10.0);
        ui.heading("导入与导出");
        self.button(ui, "导出完整配置…", Command::ExportAll);
        self.button(ui, "导入配置…", Command::Import);

        ui.add_space(10.0);
        ui.heading("重置");
        self.button(ui, "恢复默认设置", Command::ResetDefaults);
    }
}
