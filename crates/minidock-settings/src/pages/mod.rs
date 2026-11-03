//! 设置界面：9 页、纯 egui、无窗口/GPU 依赖。
//!
//! 布局是「左侧导航栏 + 右侧多卡片内容区」。雏形见
//! `docs/settings-prototype.html` —— 那是 Rust 端的施工图：页面、卡片、
//! 行一一对应。改设计先改雏形，再改这里。
//!
//! 每个页面单独一个文件（`general.rs` … `add.rs`），共享控件在
//! `widgets.rs`。这样"加一页设置"= 加一个文件，不动别人。
//!
//! 设计原则只有一条：**控件的取值范围、键名、夹紧规则全部来自
//! `core::config`**，这里只负责"长什么样"。加一个设置项只改一处（core 的
//! `Key` 表），而不是原版那样在建表 / 绘制 / 命中 / 命令四处同步修改。
//!
//! 也因此这个模块可以在无显示器环境被 kittest 完整驱动：
//! 没有 HWND、没有 device、没有线程。

mod add;
mod appearance;
mod effect;
mod extras;
mod general;
mod icons;
mod interaction;
mod library;
mod options;
mod performance;
mod position;
mod widgets;

use egui::Ui;
use minidock_core::config::Config;

pub(crate) use widgets::Ctx;

/// 十个页面。顺序 = 侧边导航顺序。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    General,
    Appearance,
    Position,
    Icons,
    Interaction,
    Extras,
    Effect,
    Performance,
    Library,
    Add,
}

impl Page {
    pub const ALL: [Page; 10] = [
        Page::General,
        Page::Appearance,
        Page::Position,
        Page::Icons,
        Page::Interaction,
        Page::Extras,
        Page::Effect,
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
            Page::Effect => "效果",
            Page::Performance => "性能",
            Page::Library => "图标库",
            Page::Add => "添加",
        }
    }

    /// 页头下面那行小字。
    #[must_use]
    pub const fn subtitle(self) -> &'static str {
        match self {
            Page::General => "启动行为、锁定与全局快捷键。",
            Page::Appearance => "栏体尺寸、背景材质与配色。",
            Page::Position => "停靠边、显示器与显示模式。",
            Page::Icons => "底盘、分割线、运行指示与标签。",
            Page::Interaction => "悬停放大、文件夹展开与动画。",
            Page::Extras => "时钟、倒影、小组件与浏览器。",
            Page::Effect => "栏体周围的环境烟雾。",
            Page::Performance => "显卡选择、实时占用与高耗能功能开关。",
            Page::Library => "从文件夹批量设置图标。",
            Page::Add => "往 Dock 里加东西，以及配置的导入导出。",
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
    /// 顶栏搜索框的内容：非空时只显示命中的导航标签。
    pub nav_filter: String,
    /// 当前展开的说明气泡（点击钉住 / 悬停 3 秒自动展开）。
    pub tip: Option<Tip>,
    /// 正在悬停的提示点：悬停满 3 秒才展开，需要跨帧计时。
    pub tip_hover: Option<(egui::Id, std::time::Instant)>,
}

/// 一个展开中的说明气泡。
#[derive(Clone, Debug)]
pub struct Tip {
    /// 归属的圆点：同一个圆点反复悬停不会重建，点击靠它判定"收起还是展开"。
    pub id: egui::Id,
    pub text: String,
    /// 圆点的矩形，气泡锚在它下方（放不下时翻到上方）。
    pub anchor: egui::Rect,
    /// 点击钉住的气泡不会因移开光标而消失。
    pub pinned: bool,
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
            nav_filter: String::new(),
            tip: None,
            tip_hover: None,
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

/// 渲染整个设置界面（左侧导航 + 右侧卡片内容）。
///
/// 状态文本不在这里画：它属于顶栏（`settings_win::shell` 的右栏）。
///
/// # 为什么是「两列」而不是 egui 的 `SidePanel`
///
/// `SidePanel` 按**根 Ui** 的可用区域布局，而设置窗口外面还套了一层
/// 标题栏的子 Ui —— 在子 Ui 里开面板会跟标题栏抢地方。这里改成在一
/// 个 Ui 里手工切两列：想放哪放哪，也不依赖窗口外壳的形状。
pub fn settings_ui(ui: &mut Ui, cfg: &mut Config, state: &mut SettingsState) -> Outcome {
    let mut out = Outcome::default();
    {
        let mut ctx = Ctx {
            cfg,
            state,
            out: &mut out,
        };

        // 两列的矩形：左栏固定宽，右栏吃剩下的。
        //
        // 用 `new_child(max_rect)` 而不是 egui 的 `SidePanel`：面板按**根 Ui**
        // 的可用区域布局，而设置窗口外面还套了一层标题栏的子 Ui ——
        // 在子 Ui 里开面板会跟标题栏抢地方。手工切矩形就没有这个约束。
        //
        // 起点必须用 `available_rect_before_wrap()` 而不是 `max_rect()`：
        // `max_rect` 不随内容收缩，外壳已经画掉的标题栏高度它不认，两列会
        // 直接盖到标题上（第一版就是这样）。
        let rect = ui.available_rect_before_wrap();
        let nav_rect =
            egui::Rect::from_min_size(rect.min, egui::vec2(widgets::NAV_W, rect.height()));
        let content_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + widgets::NAV_W + 14.0, rect.top()),
            egui::pos2(rect.right(), rect.bottom()),
        );

        // 左栏底色：比窗口底色略深，形成"导航区"的分区感。窗口本身是分层
        // 窗口，半透明底由外壳画，这里只叠一层不透明度很低的填充。
        ui.painter().rect_filled(
            nav_rect,
            egui::CornerRadius::same(18),
            widgets::sidebar_fill(ui),
        );

        // ① 左栏：导航。内缩 6 px，让选中块的圆角不贴边。
        let mut nav_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(nav_rect.shrink2(egui::vec2(6.0, 6.0)))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        widgets::sidebar(&mut nav_ui, ctx.state);

        // ② 右栏：页头 + 卡片内容
        let mut content_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(content_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        let page = ctx.state.page;
        widgets::page_header(&mut content_ui, page);
        content_ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(&mut content_ui, |ui| match page {
                Page::General => general::page(&mut ctx, ui),
                Page::Appearance => appearance::page(&mut ctx, ui),
                Page::Position => position::page(&mut ctx, ui),
                Page::Icons => icons::page(&mut ctx, ui),
                Page::Interaction => interaction::page(&mut ctx, ui),
                Page::Extras => extras::page(&mut ctx, ui),
                Page::Effect => effect::page(&mut ctx, ui),
                Page::Performance => performance::page(&mut ctx, ui),
                Page::Library => library::page(&mut ctx, ui),
                Page::Add => add::page(&mut ctx, ui),
            });

        // ③ 说明气泡最后渲染：此刻所有行的悬停/点击状态都已结算，
        // 不会出现"这一帧气泡开着、下一帧状态才更新"的抖动。
        if let Some(tip) = ctx.state.tip.clone() {
            widgets::render_tip(ui, &tip);
        }
    }
    out
}
