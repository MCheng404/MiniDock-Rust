//! `[Settings]` 段：76 个键的类型化模型、默认值、夹紧规则、读写。
//!
//! 键数与顺序**照抄上游 `g_tab[]`**（v1.4.0，2026-10-28 同步）。
//! 上游一次加了 41 个键，其中绝大多数是"裸 int 数值"（没有枚举语义），
//! 所以它们走 [`NUMERIC_KEYS`] 表而不是变成枚举变体 ——
//! **加一个上游键的代价是一行数据**，不是八处 match。
//!
//! 设计要点（与 C 版的差异，全部是被 issue 追出来的）：
//!
//! 1. **每个键都有明确的取值域**，加载时一律夹紧。原版 `LoadConfig()` 只
//!    夹了 5 个键，`Opacity=1000` 会让 alpha 溢出进颜色通道，
//!    负数会索引越界到选项数组之外（见 `pr_en.md`）。
//! 2. **索引型字段用枚举而不是 `int`**。`theme` / `bg_mode` / `edge` /
//!    `dot_style` / `plate_*` 在 C 里是裸 `int`，直接拿去索引选项数组；
//!    这里 `from_code()` 对任何越界值都回落到默认值，越界在类型层面消失。
//! 3. **键名与顺序照抄 C 的 `g_tab`**。保存时按同一顺序写回，这样同一个
//!    配置在 C 版与 Rust 版之间来回读写不会产生无意义的行序 diff。

use crate::ini::Ini;
use crate::units;

/// `[Settings]` 段的键。顺序 = C 版 `g_tab[]` 的顺序 = 保存顺序。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    IconSize,
    ZoomSize,
    Spacing,
    Margin,
    Padding,
    Radius,
    Opacity,
    Blur,
    Zoom,
    Theme,
    Dots,
    Labels,
    Bounce,
    WebAppMode,
    Border,
    BgMode,
    Mode,
    Reserve,
    Edge,
    Monitor,
    Preview,
    ShowRunning,
    Lock,
    Hotkey,
    DotStyle,
    PlateRadius,
    BgColor1,
    BgColor2,
    AccentTone,
    Reflect,
    Clock,
    FolderFan,
    MinAnim,
    Notes,
    UiTheme,
    /// 裸数值键，下标指向 [`NUMERIC_KEYS`]。
    ///
    /// 之所以不是普通变体：上游 v1.4.0 新增的 41 个键里绝大多数
    /// 在 C 版里就是裸 `int`（`&cfg.xxx` 直接读写），没有枚举语义。
    /// 用表驱动才能让"跟上游同步"变成加一行。
    Numeric(u16),
}

/// 裸数值键的表：`(ini 键名, 最小值, 最大值, 默认值, 是否布尔, 默认值随 DPI 缩放)`。
///
/// 来源：上游 v1.4.0 的 `g_tab[]` 与 `DefaultCfg()`，值域取自上游设置行
/// （`RSl` 的上下限、`RCmb` 的选项数、`RTog` = 布尔、`RCol` = 颜色）。
///
/// 上游对这些 int **不做夹紧**（只夹了一个 float），越界会直接进绘制代码。
/// 这里照旧给出安全范围 —— 这是 Rust 版相对 C 版的加固，不是偏离。
///
/// 顺序即上游 `g_tab[]` 里的顺序：写回 ini 时行序必须与 C 版一致。
pub const NUMERIC_KEYS: &[(&str, i32, i32, i32, bool, bool)] = &[
    ("PlateMode", 0, 2, 0, false, false),
    ("PlateTheme", 0, 2, 2, false, false),
    ("PlateOpacity", 0, 100, 100, false, false),
    ("BgColor3", -1, 16777215, -1, false, false),
    ("MenuTheme", 0, 2, 2, false, false),
    ("BorderColor", -1, 16777215, -1, false, false),
    ("BorderWidth", 1, 6, 1, false, false),
    ("SepColor", -1, 16777215, -1, false, false),
    ("DotColor", -1, 16777215, -1, false, false),
    ("GradAngle", 0, 360, 90, false, false),
    ("GradA1", 0, 100, 100, false, false),
    ("GradA2", 0, 100, 100, false, false),
    ("GradA3", 0, 100, 100, false, false),
    ("ClockShow", 0, 2, 0, false, false),
    ("ClockColLight", -1, 16777215, -1, false, false),
    ("ClockColDark", -1, 16777215, -1, false, false),
    ("ReflSize", 10, 80, 38, false, false),
    ("ReflGap", 0, 30, 3, false, false),
    ("ReflOpacity", 0, 100, 30, false, false),
    ("FanArc", 0, 100, 40, false, false),
    ("FanGap", 0, 40, 9, false, false),
    ("FanCut", 0, 1, 0, false, false),
    ("GroupFrost", 0, 1, 1, true, false),
    ("GroupPos", 0, 1, 0, false, false),
    ("Guardian", 0, 1, 1, true, false),
    ("ClickToggle", 0, 1, 1, true, false),
    ("SepGap", 0, 64, 4, false, true),
    ("SepWidth", 0, 16, 2, false, false),
    ("ClockSec", 0, 1, 0, true, false),
    ("ClockWeek", 0, 1, 1, true, false),
    ("DateFmt", 0, 2, 0, false, false),
    ("ClockOrder", 0, 1, 0, false, false),
    ("ClockBoldT", 0, 1, 1, true, false),
    ("ClockBoldD", 0, 1, 0, true, false),
    ("Browser", 0, 4, 0, false, false),
    ("GrpOpacity", 20, 100, 94, false, false),
    ("GrpBg", -1, 16777215, -1, false, false),
    ("MinStyle", 0, 1, 0, false, false),
    ("MnAuto", 0, 1, 1, true, false),
    ("MnNotes", 0, 1, 1, true, false),
    ("MnAdd", 0, 1, 1, true, false),
];

impl Key {
    // ---- 表驱动键的具名常量 -------------------------------------------------
    // 有了具名常量，代码里就不用写 `Key::Numeric(26)` 这种天书。
    // 索引与 [`NUMERIC_KEYS`] 的行一一对应；有测试守着（见本文件末尾）。

    /// `PlateMode`
    pub const PLATE_MODE: Key = Key::Numeric(0);
    /// `PlateTheme`
    pub const PLATE_THEME: Key = Key::Numeric(1);
    /// `PlateOpacity`
    pub const PLATE_OPACITY: Key = Key::Numeric(2);
    /// `BgColor3`
    pub const BG_COLOR3: Key = Key::Numeric(3);
    /// `MenuTheme`
    pub const MENU_THEME: Key = Key::Numeric(4);
    /// `BorderColor`
    pub const BORDER_COLOR: Key = Key::Numeric(5);
    /// `BorderWidth`
    pub const BORDER_WIDTH: Key = Key::Numeric(6);
    /// `SepColor`
    pub const SEP_COLOR: Key = Key::Numeric(7);
    /// `DotColor`
    pub const DOT_COLOR: Key = Key::Numeric(8);
    /// `GradAngle`
    pub const GRAD_ANGLE: Key = Key::Numeric(9);
    /// `GradA1`
    pub const GRAD_A1: Key = Key::Numeric(10);
    /// `GradA2`
    pub const GRAD_A2: Key = Key::Numeric(11);
    /// `GradA3`
    pub const GRAD_A3: Key = Key::Numeric(12);
    /// `ClockShow`
    pub const CLOCK_SHOW: Key = Key::Numeric(13);
    /// `ClockColLight`
    pub const CLOCK_COL_LIGHT: Key = Key::Numeric(14);
    /// `ClockColDark`
    pub const CLOCK_COL_DARK: Key = Key::Numeric(15);
    /// `ReflSize`
    pub const REFL_SIZE: Key = Key::Numeric(16);
    /// `ReflGap`
    pub const REFL_GAP: Key = Key::Numeric(17);
    /// `ReflOpacity`
    pub const REFL_OPACITY: Key = Key::Numeric(18);
    /// `FanArc`
    pub const FAN_ARC: Key = Key::Numeric(19);
    /// `FanGap`
    pub const FAN_GAP: Key = Key::Numeric(20);
    /// `FanCut`
    pub const FAN_CUT: Key = Key::Numeric(21);
    /// `GroupFrost`
    pub const GROUP_FROST: Key = Key::Numeric(22);
    /// `GroupPos`
    pub const GROUP_POS: Key = Key::Numeric(23);
    /// `Guardian`
    pub const GUARDIAN: Key = Key::Numeric(24);
    /// `ClickToggle`
    pub const CLICK_TOGGLE: Key = Key::Numeric(25);
    /// `SepGap`
    pub const SEP_GAP: Key = Key::Numeric(26);
    /// `SepWidth`
    pub const SEP_WIDTH: Key = Key::Numeric(27);
    /// `ClockSec`
    pub const CLOCK_SEC: Key = Key::Numeric(28);
    /// `ClockWeek`
    pub const CLOCK_WEEK: Key = Key::Numeric(29);
    /// `DateFmt`
    pub const DATE_FMT: Key = Key::Numeric(30);
    /// `ClockOrder`
    pub const CLOCK_ORDER: Key = Key::Numeric(31);
    /// `ClockBoldT`
    pub const CLOCK_BOLD_T: Key = Key::Numeric(32);
    /// `ClockBoldD`
    pub const CLOCK_BOLD_D: Key = Key::Numeric(33);
    /// `Browser`
    pub const BROWSER: Key = Key::Numeric(34);
    /// `GrpOpacity`
    pub const GRP_OPACITY: Key = Key::Numeric(35);
    /// `GrpBg`
    pub const GRP_BG: Key = Key::Numeric(36);
    /// `MinStyle`
    pub const MIN_STYLE: Key = Key::Numeric(37);
    /// `MnAuto`
    pub const MN_AUTO: Key = Key::Numeric(38);
    /// `MnNotes`
    pub const MN_NOTES: Key = Key::Numeric(39);
    /// `MnAdd`
    pub const MN_ADD: Key = Key::Numeric(40);
}

impl Key {
    /// 与上游 `g_tab[]` 逐项对应，**顺序必须一致**：
    /// 保存时按同一顺序写回，两个实现之间来回读写才不会产生行序 diff。
    ///
    /// 76 个键 = 35 个有类型的 + 41 个表驱动的（`Numeric`）。
    pub const ALL: &'static [Key] = &[
        Key::IconSize,
        Key::ZoomSize,
        Key::Spacing,
        Key::Margin,
        Key::Padding,
        Key::Radius,
        Key::Opacity,
        Key::Blur,
        Key::Zoom,
        Key::Theme,
        Key::Dots,
        Key::Labels,
        Key::Bounce,
        Key::WebAppMode,
        Key::Border,
        Key::BgMode,
        Key::Mode,
        Key::Reserve,
        Key::Edge,
        Key::Monitor,
        Key::Preview,
        Key::ShowRunning,
        Key::Lock,
        Key::Hotkey,
        Key::DotStyle,
        Key::PLATE_MODE,
        Key::PLATE_THEME,
        Key::PLATE_OPACITY,
        Key::PlateRadius,
        Key::BgColor1,
        Key::BgColor2,
        Key::BG_COLOR3,
        Key::AccentTone,
        Key::Reflect,
        Key::Clock,
        Key::FolderFan,
        Key::MinAnim,
        Key::Notes,
        Key::UiTheme,
        Key::MENU_THEME,
        Key::BORDER_COLOR,
        Key::BORDER_WIDTH,
        Key::SEP_COLOR,
        Key::DOT_COLOR,
        Key::GRAD_ANGLE,
        Key::GRAD_A1,
        Key::GRAD_A2,
        Key::GRAD_A3,
        Key::CLOCK_SHOW,
        Key::CLOCK_COL_LIGHT,
        Key::CLOCK_COL_DARK,
        Key::REFL_SIZE,
        Key::REFL_GAP,
        Key::REFL_OPACITY,
        Key::FAN_ARC,
        Key::FAN_GAP,
        Key::FAN_CUT,
        Key::GROUP_FROST,
        Key::GROUP_POS,
        Key::GUARDIAN,
        Key::CLICK_TOGGLE,
        Key::SEP_GAP,
        Key::SEP_WIDTH,
        Key::CLOCK_SEC,
        Key::CLOCK_WEEK,
        Key::DATE_FMT,
        Key::CLOCK_ORDER,
        Key::CLOCK_BOLD_T,
        Key::CLOCK_BOLD_D,
        Key::BROWSER,
        Key::GRP_OPACITY,
        Key::GRP_BG,
        Key::MIN_STYLE,
        Key::MN_AUTO,
        Key::MN_NOTES,
        Key::MN_ADD,
    ];

    /// ini 中的键名（大小写照抄原版，Windows profile API 本身不区分大小写）。
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Key::IconSize => "IconSize",
            Key::ZoomSize => "ZoomSize",
            Key::Spacing => "Spacing",
            Key::Margin => "Margin",
            Key::Padding => "Padding",
            Key::Radius => "Radius",
            Key::Opacity => "Opacity",
            Key::Blur => "Blur",
            Key::Zoom => "Zoom",
            Key::Theme => "Theme",
            Key::Dots => "Dots",
            Key::Labels => "Labels",
            Key::Bounce => "Bounce",
            Key::WebAppMode => "WebAppMode",
            Key::Border => "Border",
            Key::BgMode => "BgMode",
            Key::Mode => "Mode",
            Key::Reserve => "Reserve",
            Key::Edge => "Edge",
            Key::Monitor => "Monitor",
            Key::Preview => "Preview",
            Key::ShowRunning => "ShowRunning",
            Key::Lock => "Lock",
            Key::Hotkey => "Hotkey",
            Key::DotStyle => "DotStyle",
            Key::PlateRadius => "PlateRadius",
            Key::BgColor1 => "BgColor1",
            Key::BgColor2 => "BgColor2",
            Key::AccentTone => "AccentTone",
            Key::Reflect => "Reflect",
            Key::Clock => "Clock",
            Key::FolderFan => "FolderFan",
            Key::MinAnim => "MinAnim",
            Key::Notes => "Notes",
            Key::UiTheme => "UiTheme",
            Key::Numeric(i) => NUMERIC_KEYS[i as usize].0,
        }
    }

    /// 大小写不敏感的键名查找。手写 ini 里出现 `iconsize=64` 也要认。
    #[must_use]
    pub fn from_name(s: &str) -> Option<Self> {
        let s = s.trim();
        Self::ALL
            .iter()
            .copied()
            .find(|k| k.name().eq_ignore_ascii_case(s))
    }

    /// 加载时的**硬夹紧**区间 `(min, max)`。这是安全边界，不是 UI 提示范围。
    #[must_use]
    pub fn range(self) -> (i32, i32) {
        if let Key::Numeric(i) = self {
            let row = NUMERIC_KEYS[i as usize];
            return (row.1, row.2);
        }
        match self {
            // 几何量。上限取到 256 是为了给高 DPI 留空间（200% 下 128 px）。
            Key::IconSize => (16, 256),
            Key::ZoomSize => (16, 256),
            Key::Spacing => (0, 64),
            Key::Padding => (0, 64),
            Key::Radius => (0, 120),
            // Margin 原版只夹了下界；这里给上界是因为它直接参与窗口尺寸计算，
            // 过大的值会让窗口远超屏幕。
            Key::Margin => (0, 400),
            Key::Opacity => (0, 100),
            Key::Blur => (1, 32),
            Key::PlateRadius => (0, 50),
            Key::Monitor => (0, 63),
            // 枚举型：区间仅用于 UI 展示，实际由 code 夹紧。
            Key::Theme | Key::UiTheme | Key::BgMode | Key::Mode => (0, 2),
            Key::Edge => (0, 3),
            Key::DotStyle => (0, 1),
            // 颜色：-1 表示"未设置"，0..=0xFFFFFF 是 COLORREF。
            Key::BgColor1 | Key::BgColor2 => (-1, 0x00FF_FFFF),
            // 布尔：只认 0/1。
            _ => (0, 1),
        }
    }

    /// 设置界面里滑杆的**提示范围**（照抄 C 版 `BuildRows()` 的参数）。
    /// 与 `range()` 的区别：这里更窄，因为它是"推荐可调范围"，
    /// 而 `range()` 是"不把程序搞坏"的边界。没有对应滑杆的键返回 `None`。
    #[must_use]
    pub const fn slider_range(self) -> Option<(i32, i32)> {
        Some(match self {
            Key::IconSize => (24, 128),
            Key::ZoomSize => (32, 220),
            Key::Spacing => (0, 30),
            Key::Padding => (2, 30),
            Key::Radius => (0, 40),
            Key::Opacity => (0, 100),
            Key::Blur => (1, 14),
            Key::Margin => (0, 150),
            Key::PlateRadius => (0, 50),
            _ => return None,
        })
    }

    /// 该键是否映射到枚举（决定取值走 `from_code` 还是数值夹紧）。
    ///
    /// 表驱动的数值键**不算枚举**：它们在 C 版里就是裸 int，
    /// 越界时按范围夹紧比"回落到默认变体"更接近上游行为。
    #[must_use]
    pub const fn is_enum(self) -> bool {
        matches!(
            self,
            Key::Theme | Key::UiTheme | Key::BgMode | Key::Mode | Key::Edge | Key::DotStyle
        )
    }

    /// 该键是否为布尔语义（写回 ini 时用 0/1）。
    #[must_use]
    pub fn is_bool(self) -> bool {
        if let Key::Numeric(i) = self {
            return NUMERIC_KEYS[i as usize].4;
        }
        matches!(
            self,
            Key::Zoom
                | Key::Dots
                | Key::Labels
                | Key::Bounce
                | Key::WebAppMode
                | Key::Border
                | Key::Reserve
                | Key::Preview
                | Key::ShowRunning
                | Key::Lock
                | Key::Hotkey
                | Key::AccentTone
                | Key::Reflect
                | Key::Clock
                | Key::FolderFan
                | Key::MinAnim
                | Key::Notes
        )
    }
}

/// Dock 本体主题。`0 = 深色`，`1 = 浅色`，`2 = 跟随系统`。
///
/// 注意与 [`UiTheme`] 的取值**相反**（那是历史遗留：设置窗口先做，
/// 用了"0 = 浅色"的顺序）。这不是笔误，改动会破坏现有 ini。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum DockTheme {
    #[default]
    Dark = 0,
    Light = 1,
    System = 2,
}

/// 设置窗口主题。`0 = 浅色`，`1 = 深色`，`2 = 跟随系统`。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum UiTheme {
    Light = 0,
    Dark = 1,
    #[default]
    System = 2,
}

/// 背景材质。`0 = 纯色`，`1 = 渐变`，`2 = 磨砂玻璃`。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum BgMode {
    Solid = 0,
    Gradient = 1,
    #[default]
    Frost = 2,
}

/// 显示模式。`0 = 常驻`，`1 = 自动隐藏`，`2 = 智能隐藏`。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum DockMode {
    #[default]
    Always = 0,
    AutoHide = 1,
    SmartHide = 2,
}

/// 停靠边。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Edge {
    #[default]
    Bottom = 0,
    Top = 1,
    Left = 2,
    Right = 3,
}

impl Edge {
    /// 是否为水平布局（底部/顶部）。原版 `Horiz()`。
    #[must_use]
    pub const fn is_horizontal(self) -> bool {
        matches!(self, Edge::Bottom | Edge::Top)
    }
}

/// 运行指示样式。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum DotStyle {
    #[default]
    Bar = 0,
    Dot = 1,
}

macro_rules! code_enum {
    ($name:ident { $($variant:ident = $code:expr),+ $(,)? }, default = $default:expr) => {
        impl $name {
            #[must_use]
            pub const fn from_code(v: i32) -> Self {
                match v {
                    $($code => Self::$variant,)+
                    _ => $default,
                }
            }
            #[must_use]
            pub const fn code(self) -> i32 {
                match self {
                    $(Self::$variant => $code,)+
                }
            }
        }
    };
}

code_enum!(DockTheme { Dark = 0, Light = 1, System = 2 }, default = DockTheme::Dark);
code_enum!(UiTheme { Light = 0, Dark = 1, System = 2 }, default = UiTheme::System);
code_enum!(BgMode { Solid = 0, Gradient = 1, Frost = 2 }, default = BgMode::Frost);
code_enum!(DockMode { Always = 0, AutoHide = 1, SmartHide = 2 }, default = DockMode::Always);
code_enum!(Edge { Bottom = 0, Top = 1, Left = 2, Right = 3 }, default = Edge::Bottom);
code_enum!(DotStyle { Bar = 0, Dot = 1 }, default = DotStyle::Bar);

/// `[Settings]` 段的完整模型。
///
/// 字段与 ini 键一一对应；`bool` 对应 0/1，枚举对应上表。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub icon_size: i32,
    pub zoom_size: i32,
    pub spacing: i32,
    pub margin: i32,
    pub pad: i32,
    pub radius: i32,
    pub opacity: i32,
    pub blur: i32,

    pub zoom: bool,
    pub dots: bool,
    pub labels: bool,
    pub bounce: bool,
    pub web_app: bool,
    pub border: bool,
    pub reserve: bool,
    pub preview: bool,
    pub show_running: bool,
    pub lock: bool,
    pub hotkey: bool,
    pub accent_tone: bool,
    pub reflect: bool,
    pub clock: bool,
    pub folder_fan: bool,
    pub min_anim: bool,
    pub notes: bool,

    pub theme: DockTheme,
    pub ui_theme: UiTheme,
    pub bg_mode: BgMode,
    pub mode: DockMode,
    pub edge: Edge,
    pub dot_style: DotStyle,

    pub monitor: i32,
    pub plate_radius: i32,
    /// `-1` = 未自定义；否则是 `COLORREF`（`0x00BBGGRR`）。
    pub bg_color1: i32,
    pub bg_color2: i32,

    /// 表驱动键的值，按 [`NUMERIC_KEYS`] 的顺序存放。
    ///
    /// 用 `Vec` 而不是 41 个字段：这些键绝大多数是裸数值，
    /// 单独开字段只会让"加一个上游键"变成改五个地方。
    pub numeric: Vec<i32>,

    /// 显卡选择。空串 = **自动**（交给后端按功耗偏好挑，通常落到核显）。
    ///
    /// 存的是**适配器名字**而不是索引：索引在换驱动、插拔外接显卡、
    /// 系统更新后都会变，名字才是稳定的标识。名字对不上时后端回退到自动，
    /// 不会因为换机器就起不来。
    ///
    /// 放在独立的 `[Graphics]` 段，不塞进 `[Settings]`：
    /// 那 37 个键与 C 版 `g_tab[]` 一一对应，是刻意保留的对照关系，
    /// 这个键是 Rust 版独有的，混进去会破坏这条不变式。
    pub adapter: String,
}

impl Config {
    /// 原版 `DefaultCfg()`。几何量按 DPI 缩放，其余为固定值。
    ///
    /// **必须**用当前 DPI 构造：ini 里存的是**物理像素**，首次运行时
    /// 由这里的缩放决定默认值；之后 ini 有值就以 ini 为准，不再二次缩放。
    #[must_use]
    pub fn defaults(dpi: i32) -> Self {
        Self {
            icon_size: units::scale(48, dpi),
            zoom_size: units::scale(84, dpi),
            spacing: units::scale(6, dpi),
            margin: units::scale(8, dpi),
            pad: units::scale(8, dpi),
            radius: units::scale(18, dpi),
            opacity: 60,
            blur: 6,

            zoom: true,
            dots: true,
            labels: true,
            bounce: true,
            web_app: true,
            border: true,
            reserve: false,
            preview: true,
            show_running: true,
            lock: false,
            hotkey: true,
            accent_tone: false,
            reflect: false,
            clock: false,
            folder_fan: true,
            min_anim: true,
            notes: false,

            theme: DockTheme::Dark,
            ui_theme: UiTheme::System,
            bg_mode: BgMode::Frost,
            mode: DockMode::Always,
            edge: Edge::Bottom,
            dot_style: DotStyle::Bar,

            monitor: 0,
            plate_radius: 22,
            bg_color1: -1,
            bg_color2: -1,

            numeric: NUMERIC_KEYS
                .iter()
                .map(|row| {
                    if row.5 {
                        units::scale(row.3, dpi)
                    } else {
                        row.3
                    }
                })
                .collect(),

            adapter: String::new(),
        }
    }

    /// 适配器名字的合法化：去首尾空白、剔掉控制字符、限长。
    ///
    /// 为什么要在 core 里做：这个名字会被写进 ini，也会被拿去和后端
    /// 枚举出来的名字比对。带 `\r\n` 或超长的值只会在后端静默匹配失败，
    /// 表现为"选了显卡但没生效"，很难查。
    #[must_use]
    pub fn sanitize_adapter(raw: &str) -> String {
        /// 适配器名字最长见过 60 多字符（含厂商后缀），128 足够且有上界。
        const MAX: usize = 128;
        raw.chars()
            .filter(|c| !c.is_control())
            .collect::<String>()
            .trim()
            .chars()
            .take(MAX)
            .collect()
    }

    /// 把所有字段夹到合法域。幂等。
    pub fn clamp(&mut self) {
        for key in Key::ALL.iter().copied() {
            let v = self.get_i32(key);
            self.apply_i32(key, v);
        }
    }

    /// 读取单个键（枚举返回 `code()`，布尔返回 0/1，颜色返回原值）。
    #[must_use]
    pub fn get_i32(&self, key: Key) -> i32 {
        if let Key::Numeric(i) = key {
            return self.numeric[i as usize];
        }
        match key {
            Key::IconSize => self.icon_size,
            Key::ZoomSize => self.zoom_size,
            Key::Spacing => self.spacing,
            Key::Margin => self.margin,
            Key::Padding => self.pad,
            Key::Radius => self.radius,
            Key::Opacity => self.opacity,
            Key::Blur => self.blur,
            Key::Zoom => i32::from(self.zoom),
            Key::Theme => self.theme.code(),
            Key::Dots => i32::from(self.dots),
            Key::Labels => i32::from(self.labels),
            Key::Bounce => i32::from(self.bounce),
            Key::WebAppMode => i32::from(self.web_app),
            Key::Border => i32::from(self.border),
            Key::BgMode => self.bg_mode.code(),
            Key::Mode => self.mode.code(),
            Key::Reserve => i32::from(self.reserve),
            Key::Edge => self.edge.code(),
            Key::Monitor => self.monitor,
            Key::Preview => i32::from(self.preview),
            Key::ShowRunning => i32::from(self.show_running),
            Key::Lock => i32::from(self.lock),
            Key::Hotkey => i32::from(self.hotkey),
            Key::DotStyle => self.dot_style.code(),
            Key::PlateRadius => self.plate_radius,
            Key::BgColor1 => self.bg_color1,
            Key::BgColor2 => self.bg_color2,
            Key::AccentTone => i32::from(self.accent_tone),
            Key::Reflect => i32::from(self.reflect),
            Key::Clock => i32::from(self.clock),
            Key::FolderFan => i32::from(self.folder_fan),
            Key::MinAnim => i32::from(self.min_anim),
            Key::Notes => i32::from(self.notes),
            Key::UiTheme => self.ui_theme.code(),
            // 数值键在进入 match 前就返回了。
            Key::Numeric(_) => unreachable!("数值键已提前处理"),
        }
    }

    /// 写入单个键，**立即**夹紧。任何来源（ini、设置界面、预设按钮）
    /// 都必须走这里，不允许直接改字段后忘记夹紧。
    pub fn apply_i32(&mut self, key: Key, v: i32) {
        if let Key::Numeric(i) = key {
            let idx = i as usize;
            let v = if key.is_bool() {
                i32::from(v != 0)
            } else {
                let (lo, hi) = key.range();
                v.clamp(lo, hi)
            };
            self.numeric[idx] = v;
            return;
        }
        let v = if key.is_bool() {
            i32::from(v != 0)
        } else if key.is_enum() {
            // 枚举**不做**数值夹紧：99 夹到 2 会静默变成「跟随系统」，
            // 语义完全错了。越界一律由 from_code 回落到默认变体。
            v
        } else {
            let (lo, hi) = key.range();
            v.clamp(lo, hi)
        };
        match key {
            Key::IconSize => self.icon_size = v,
            Key::ZoomSize => self.zoom_size = v,
            Key::Spacing => self.spacing = v,
            Key::Margin => self.margin = v,
            Key::Padding => self.pad = v,
            Key::Radius => self.radius = v,
            Key::Opacity => self.opacity = v,
            Key::Blur => self.blur = v,
            Key::Zoom => self.zoom = v != 0,
            Key::Theme => self.theme = DockTheme::from_code(v),
            Key::Dots => self.dots = v != 0,
            Key::Labels => self.labels = v != 0,
            Key::Bounce => self.bounce = v != 0,
            Key::WebAppMode => self.web_app = v != 0,
            Key::Border => self.border = v != 0,
            Key::BgMode => self.bg_mode = BgMode::from_code(v),
            Key::Mode => self.mode = DockMode::from_code(v),
            Key::Reserve => self.reserve = v != 0,
            Key::Edge => self.edge = Edge::from_code(v),
            Key::Monitor => self.monitor = v,
            Key::Preview => self.preview = v != 0,
            Key::ShowRunning => self.show_running = v != 0,
            Key::Lock => self.lock = v != 0,
            Key::Hotkey => self.hotkey = v != 0,
            Key::DotStyle => self.dot_style = DotStyle::from_code(v),
            Key::PlateRadius => self.plate_radius = v,
            Key::BgColor1 => self.bg_color1 = v,
            Key::BgColor2 => self.bg_color2 = v,
            Key::AccentTone => self.accent_tone = v != 0,
            Key::Reflect => self.reflect = v != 0,
            Key::Clock => self.clock = v != 0,
            Key::FolderFan => self.folder_fan = v != 0,
            Key::MinAnim => self.min_anim = v != 0,
            Key::Notes => self.notes = v != 0,
            Key::UiTheme => self.ui_theme = UiTheme::from_code(v),
            // 数值键在进入 match 前就返回了。
            Key::Numeric(_) => unreachable!("数值键已提前处理"),
        }
    }

    /// 从 `[Settings]` 读取。缺失的键保留默认值。
    ///
    /// 兼容 1.x 的 `Frost` 键：`BgMode` 不存在但有 `Frost` 时，
    /// `Frost=1` → 磨砂，`Frost=0` → 纯色。原版逻辑在 `LoadConfig()`。
    #[must_use]
    pub fn from_ini(ini: &Ini, dpi: i32) -> Self {
        let mut cfg = Self::defaults(dpi);
        if ini.get("Settings", "BgMode").is_none() {
            if let Some(frost) = ini.get_i32("Settings", "Frost") {
                cfg.bg_mode = if frost != 0 {
                    BgMode::Frost
                } else {
                    BgMode::Solid
                };
            }
        }
        for key in Key::ALL.iter().copied() {
            if let Some(raw) = ini.get("Settings", key.name()) {
                // Windows profile API 对非数字返回 0；这里保持同样行为，
                // 但**不**把"解析失败"当成 0 悄悄用下去 —— 夹紧会兜住它。
                let v = raw.trim().parse::<i32>().unwrap_or(0);
                cfg.apply_i32(key, v);
            }
        }
        cfg.adapter = Self::sanitize_adapter(ini.get("Graphics", "Adapter").unwrap_or_default());
        cfg.clamp();
        cfg
    }

    /// 写回 `[Settings]`，保留其它段与未知键。
    pub fn write_ini(&self, ini: &mut Ini) {
        for key in Key::ALL.iter().copied() {
            ini.set_i32("Settings", key.name(), self.get_i32(key));
        }
        // 自动时写空串而不是删键：删掉会让"手动改成自动"在文件里看不出痕迹，
        // 排查用户配置时容易误判成"从没设置过"。
        ini.set("Graphics", "Adapter", &self.adapter);
    }

    /// Dock 主题解析：`System` 时跟随系统。
    #[must_use]
    pub fn dock_is_dark(&self, system_dark: bool) -> bool {
        match self.theme {
            DockTheme::Dark => true,
            DockTheme::Light => false,
            DockTheme::System => system_dark,
        }
    }

    /// 设置窗口主题解析。注意 `UiTheme` 的 0/1 与 `DockTheme` 相反。
    #[must_use]
    pub fn ui_is_dark(&self, system_dark: bool) -> bool {
        match self.ui_theme {
            UiTheme::Light => false,
            UiTheme::Dark => true,
            UiTheme::System => system_dark,
        }
    }

    /// 磨砂背景是否启用（`BgMode == 2`，对应 `GetSpecial(A_FROST)`）。
    #[must_use]
    pub fn frost_enabled(&self) -> bool {
        self.bg_mode == BgMode::Frost
    }

    /// 运行状态扫描是否启用（`Dots` 或 `ShowRunning`，对应 `A_SCAN`）。
    #[must_use]
    pub fn scan_enabled(&self) -> bool {
        self.dots || self.show_running
    }
}

impl Default for Config {
    /// 96 DPI 的默认值。窗口层必须显式传 DPI 用 [`Config::defaults`]。
    fn default() -> Self {
        Self::defaults(96)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_c_version() {
        let c = Config::defaults(96);
        assert_eq!(c.icon_size, 48);
        assert_eq!(c.zoom_size, 84);
        assert_eq!(c.spacing, 6);
        assert_eq!(c.margin, 8);
        assert_eq!(c.pad, 8);
        assert_eq!(c.radius, 18);
        assert_eq!(c.opacity, 60);
        assert_eq!(c.blur, 6);
        assert_eq!(c.bg_mode, BgMode::Frost);
        assert_eq!(c.plate_radius, 22);
        assert_eq!(c.bg_color1, -1);
        assert_eq!(c.ui_theme, UiTheme::System);
        assert!(c.zoom && c.dots && c.labels && c.bounce && c.web_app && c.border);
        assert!(!c.reserve && !c.lock && !c.clock && !c.reflect && !c.notes);
    }

    #[test]
    fn defaults_scale_with_dpi() {
        let c = Config::defaults(144);
        assert_eq!(c.icon_size, 72);
        assert_eq!(c.radius, 27);
        // 非几何量不随 DPI 变。
        assert_eq!(c.opacity, 60);
        assert_eq!(c.plate_radius, 22);
    }

    #[test]
    fn hostile_values_are_clamped() {
        // pr_en.md 第 3 条的原样复现：这些值在原版里直接进几何/alpha 计算。
        let mut c = Config::defaults(96);
        for (key, evil) in [
            (Key::Opacity, 1000),
            (Key::Opacity, -5),
            (Key::IconSize, 999_999),
            (Key::IconSize, i32::MIN),
            (Key::ZoomSize, 1),
            (Key::Blur, 0),
            (Key::Blur, 999),
            (Key::Spacing, -1),
            (Key::Margin, -1),
            (Key::Padding, -1),
            (Key::Radius, -1),
            (Key::PlateRadius, 500),
            (Key::Theme, 99),
            (Key::Theme, -3),
            (Key::UiTheme, i32::MAX),
            (Key::BgMode, 7),
            (Key::Edge, 4),
            (Key::DotStyle, -1),
            (Key::Monitor, -1),
            // 表驱动的键也要夹紧：上游对这些 int 根本不夹，
            // 越界会直接进绘制代码。
            (Key::PLATE_OPACITY, 999),
            (Key::SEP_GAP, -5),
            (Key::GRAD_ANGLE, i32::MAX),
            (Key::BROWSER, 77),
        ] {
            c.apply_i32(key, evil);
            let got = c.get_i32(key);
            let (lo, hi) = key.range();
            assert!(
                (lo..=hi).contains(&got),
                "{} = {evil} -> {got} 越界 [{lo}, {hi}]",
                key.name()
            );
        }
        // 最后一轮 Opacity 是 -5，所以归零；单独再验上界。
        assert_eq!(c.opacity, 0);
        c.apply_i32(Key::Opacity, 250);
        assert_eq!(c.opacity, 100);
        // Blur 的最后一轮是 999 -> 夹到上界 32。
        assert_eq!(c.blur, 32);
        c.apply_i32(Key::Blur, 0);
        assert_eq!(c.blur, 1);
        assert_eq!(c.theme, DockTheme::Dark);
        assert_eq!(c.edge, Edge::Bottom);
        // 底盘三键（上游 v1.4.0 用 PlateMode/PlateTheme/PlateOpacity
        // 取代了旧的 PlateDark/PlateLight）。
        assert_eq!(c.get_i32(Key::PLATE_MODE), 0);
        assert_eq!(c.get_i32(Key::PLATE_THEME), 2);
        assert_eq!(c.get_i32(Key::PLATE_OPACITY), 100);
    }

    /// 表驱动键的具名常量必须与表里的行一一对应。
    ///
    /// 常量里写死了下标，表一重排就全错。这条测试是唯一的护栏：
    /// 任何一次调整 [`NUMERIC_KEYS`] 的顺序，都必须先更新常量再改这里。
    #[test]
    fn numeric_constants_match_table_rows() {
        let pairs: [(Key, &str); 41] = [
            (Key::PLATE_MODE, "PlateMode"),
            (Key::PLATE_THEME, "PlateTheme"),
            (Key::PLATE_OPACITY, "PlateOpacity"),
            (Key::BG_COLOR3, "BgColor3"),
            (Key::MENU_THEME, "MenuTheme"),
            (Key::BORDER_COLOR, "BorderColor"),
            (Key::BORDER_WIDTH, "BorderWidth"),
            (Key::SEP_COLOR, "SepColor"),
            (Key::DOT_COLOR, "DotColor"),
            (Key::GRAD_ANGLE, "GradAngle"),
            (Key::GRAD_A1, "GradA1"),
            (Key::GRAD_A2, "GradA2"),
            (Key::GRAD_A3, "GradA3"),
            (Key::CLOCK_SHOW, "ClockShow"),
            (Key::CLOCK_COL_LIGHT, "ClockColLight"),
            (Key::CLOCK_COL_DARK, "ClockColDark"),
            (Key::REFL_SIZE, "ReflSize"),
            (Key::REFL_GAP, "ReflGap"),
            (Key::REFL_OPACITY, "ReflOpacity"),
            (Key::FAN_ARC, "FanArc"),
            (Key::FAN_GAP, "FanGap"),
            (Key::FAN_CUT, "FanCut"),
            (Key::GROUP_FROST, "GroupFrost"),
            (Key::GROUP_POS, "GroupPos"),
            (Key::GUARDIAN, "Guardian"),
            (Key::CLICK_TOGGLE, "ClickToggle"),
            (Key::SEP_GAP, "SepGap"),
            (Key::SEP_WIDTH, "SepWidth"),
            (Key::CLOCK_SEC, "ClockSec"),
            (Key::CLOCK_WEEK, "ClockWeek"),
            (Key::DATE_FMT, "DateFmt"),
            (Key::CLOCK_ORDER, "ClockOrder"),
            (Key::CLOCK_BOLD_T, "ClockBoldT"),
            (Key::CLOCK_BOLD_D, "ClockBoldD"),
            (Key::BROWSER, "Browser"),
            (Key::GRP_OPACITY, "GrpOpacity"),
            (Key::GRP_BG, "GrpBg"),
            (Key::MIN_STYLE, "MinStyle"),
            (Key::MN_AUTO, "MnAuto"),
            (Key::MN_NOTES, "MnNotes"),
            (Key::MN_ADD, "MnAdd"),
        ];
        for (key, name) in pairs {
            assert_eq!(key.name(), name, "常量与表行错位");
        }
        assert_eq!(NUMERIC_KEYS.len(), pairs.len());
    }

    /// 键的总数与顺序必须与上游 `g_tab[]` 一致。
    ///
    /// 顺序决定写回 ini 的行序：两个实现之间来回读写，
    /// 行序不一致会产生无意义的 diff。
    #[test]
    fn key_list_matches_upstream_order() {
        let names: Vec<&str> = Key::ALL.iter().map(|k| k.name()).collect();
        assert_eq!(names.len(), 76, "上游 v1.4.0 是 76 个键");
        // 抽查几个位置：顺序错了这里就会红。
        assert_eq!(names[0], "IconSize");
        assert_eq!(names[25], "PlateMode");
        assert_eq!(names[28], "PlateRadius");
        assert_eq!(names[31], "BgColor3");
        assert_eq!(names[38], "UiTheme");
        assert_eq!(names[39], "MenuTheme");
        assert_eq!(names[75], "MnAdd");
        // 不许有重复键名。
        let mut seen = std::collections::HashSet::new();
        for n in &names {
            assert!(seen.insert(*n), "键名重复: {n}");
        }
    }

    /// 数值键的默认值与上游 `DefaultCfg()` 一致。
    #[test]
    fn numeric_defaults_match_upstream() {
        let c = Config::defaults(96);
        assert_eq!(c.get_i32(Key::PLATE_MODE), 0);
        assert_eq!(c.get_i32(Key::PLATE_THEME), 2);
        assert_eq!(c.get_i32(Key::PLATE_OPACITY), 100);
        assert_eq!(c.get_i32(Key::MENU_THEME), 2);
        assert_eq!(c.get_i32(Key::BORDER_WIDTH), 1);
        assert_eq!(c.get_i32(Key::GRAD_ANGLE), 90);
        assert_eq!(c.get_i32(Key::REFL_SIZE), 38);
        assert_eq!(c.get_i32(Key::REFL_GAP), 3);
        assert_eq!(c.get_i32(Key::REFL_OPACITY), 30);
        assert_eq!(c.get_i32(Key::FAN_ARC), 40);
        assert_eq!(c.get_i32(Key::FAN_GAP), 9);
        assert_eq!(c.get_i32(Key::GRP_OPACITY), 94);
        assert_eq!(c.get_i32(Key::CLOCK_WEEK), 1);
        assert_eq!(c.get_i32(Key::CLOCK_BOLD_T), 1);
        assert_eq!(c.get_i32(Key::MN_AUTO), 1);
        assert_eq!(c.get_i32(Key::MN_NOTES), 1);
        assert_eq!(c.get_i32(Key::MN_ADD), 1);
        // SepGap 的默认值随 DPI 缩放（上游是 Sc(4)）。
        assert_eq!(c.get_i32(Key::SEP_GAP), 4);
        assert_eq!(Config::defaults(144).get_i32(Key::SEP_GAP), 6);
    }

    #[test]
    fn bool_keys_normalize_to_0_or_1() {
        let mut c = Config::defaults(96);
        c.apply_i32(Key::Dots, 7);
        assert!(c.dots);
        assert_eq!(c.get_i32(Key::Dots), 1);
        c.apply_i32(Key::Dots, -7);
        assert!(c.dots);
        c.apply_i32(Key::Dots, 0);
        assert!(!c.dots);
    }

    #[test]
    fn key_name_lookup_is_case_insensitive() {
        assert_eq!(Key::from_name("iconsize"), Some(Key::IconSize));
        assert_eq!(Key::from_name("  Opacity "), Some(Key::Opacity));
        assert_eq!(Key::from_name("nope"), None);
    }

    #[test]
    fn key_table_matches_c_g_tab() {
        // 76 个键（上游 v1.4.0），顺序即保存顺序；顺序错了会造成 ini diff 噪音。
        // 这里直接把上游的键名列出来，顺序错一个字符都会红。
        const UPSTREAM: [&str; 76] = [
            "IconSize",
            "ZoomSize",
            "Spacing",
            "Margin",
            "Padding",
            "Radius",
            "Opacity",
            "Blur",
            "Zoom",
            "Theme",
            "Dots",
            "Labels",
            "Bounce",
            "WebAppMode",
            "Border",
            "BgMode",
            "Mode",
            "Reserve",
            "Edge",
            "Monitor",
            "Preview",
            "ShowRunning",
            "Lock",
            "Hotkey",
            "DotStyle",
            "PlateMode",
            "PlateTheme",
            "PlateOpacity",
            "PlateRadius",
            "BgColor1",
            "BgColor2",
            "BgColor3",
            "AccentTone",
            "Reflect",
            "Clock",
            "FolderFan",
            "MinAnim",
            "Notes",
            "UiTheme",
            "MenuTheme",
            "BorderColor",
            "BorderWidth",
            "SepColor",
            "DotColor",
            "GradAngle",
            "GradA1",
            "GradA2",
            "GradA3",
            "ClockShow",
            "ClockColLight",
            "ClockColDark",
            "ReflSize",
            "ReflGap",
            "ReflOpacity",
            "FanArc",
            "FanGap",
            "FanCut",
            "GroupFrost",
            "GroupPos",
            "Guardian",
            "ClickToggle",
            "SepGap",
            "SepWidth",
            "ClockSec",
            "ClockWeek",
            "DateFmt",
            "ClockOrder",
            "ClockBoldT",
            "ClockBoldD",
            "Browser",
            "GrpOpacity",
            "GrpBg",
            "MinStyle",
            "MnAuto",
            "MnNotes",
            "MnAdd",
        ];
        assert_eq!(Key::ALL.len(), UPSTREAM.len());
        for (i, name) in UPSTREAM.iter().enumerate() {
            assert_eq!(Key::ALL[i].name(), *name, "第 {i} 个键对不上");
        }
        // 名字必须唯一，否则 from_name 会静默取第一个。
        for (i, k) in Key::ALL.iter().enumerate() {
            for other in &Key::ALL[i + 1..] {
                assert_ne!(k.name(), other.name());
            }
        }
    }

    #[test]
    fn ini_roundtrip_preserves_every_key() {
        let mut c = Config::defaults(96);
        c.apply_i32(Key::IconSize, 64);
        c.apply_i32(Key::Edge, 2);
        c.apply_i32(Key::BgColor1, 0x0011_2233);
        c.apply_i32(Key::Clock, 1);

        let mut ini = Ini::new();
        c.write_ini(&mut ini);
        let back = Config::from_ini(&ini, 96);
        assert_eq!(back, c);
    }

    #[test]
    fn legacy_frost_key_migrates() {
        let mut ini = Ini::new();
        ini.set("Settings", "Frost", "1");
        assert_eq!(Config::from_ini(&ini, 96).bg_mode, BgMode::Frost);

        let mut ini = Ini::new();
        ini.set("Settings", "Frost", "0");
        assert_eq!(Config::from_ini(&ini, 96).bg_mode, BgMode::Solid);

        // 新键优先：同时存在时以 BgMode 为准。
        let mut ini = Ini::new();
        ini.set("Settings", "Frost", "1");
        ini.set("Settings", "BgMode", "0");
        assert_eq!(Config::from_ini(&ini, 96).bg_mode, BgMode::Solid);
    }

    #[test]
    fn missing_and_garbage_values_fall_back() {
        let mut ini = Ini::new();
        ini.set("Settings", "Opacity", "abc");
        ini.set("Settings", "IconSize", "");
        let c = Config::from_ini(&ini, 96);
        assert_eq!(c.opacity, 0); // 与 GetPrivateProfileInt 一致：非数字 -> 0
        assert_eq!(c.icon_size, 16); // 0 被夹到下限
    }

    #[test]
    fn ui_theme_codes_are_inverted_relative_to_dock_theme() {
        // 这条测试是防回归用的：两套枚举的 0/1 含义相反，改错会静默反转主题。
        assert_eq!(DockTheme::from_code(0), DockTheme::Dark);
        assert_eq!(UiTheme::from_code(0), UiTheme::Light);
        let c = Config {
            theme: DockTheme::Dark,
            ui_theme: UiTheme::Light,
            ..Config::defaults(96)
        };
        assert!(c.dock_is_dark(false));
        assert!(!c.ui_is_dark(true));
    }

    /// 全部 76 个键都要能写进 ini 再读回来（值在合法域内时原样往返）。
    ///
    /// 键表改成表驱动之后，这条测试是唯一能证明"表里的每一行都通了"的护栏。
    #[test]
    fn every_key_roundtrips_through_ini() {
        let mut c = Config::defaults(96);
        // 给每个键一个合法且互不相同的值：枚举用 1，布尔用 1，数值用范围内中点。
        for key in Key::ALL.iter().copied() {
            let (lo, hi) = key.range();
            // 布尔与枚举都用 1（合法的中间值），数值键取范围中点。
            let v = if key.is_bool() || key.is_enum() {
                1
            } else if lo < 0 {
                0
            } else {
                lo + (hi - lo) / 2
            };
            c.apply_i32(key, v);
        }
        let mut ini = Ini::new();
        c.write_ini(&mut ini);
        let back = Config::from_ini(&ini, 96);
        for key in Key::ALL.iter().copied() {
            assert_eq!(
                back.get_i32(key),
                c.get_i32(key),
                "{} 往返后变了",
                key.name()
            );
        }
    }

    #[test]
    fn adapter_defaults_to_auto() {
        // 空串 = 自动。默认必须是自动，否则新用户会被钉在某块卡上。
        assert!(Config::defaults(96).adapter.is_empty());
    }

    #[test]
    fn adapter_roundtrips_through_graphics_section() {
        let mut c = Config::defaults(96);
        c.adapter = "NVIDIA GeForce RTX 5070 Laptop GPU".to_owned();
        let mut ini = Ini::new();
        c.write_ini(&mut ini);

        // 必须落在独立段里，不能污染与 C 版对照的 [Settings]。
        assert_eq!(
            ini.get("Graphics", "Adapter"),
            Some("NVIDIA GeForce RTX 5070 Laptop GPU")
        );
        assert_eq!(ini.get("Settings", "Adapter"), None);

        let back = Config::from_ini(&Ini::parse(&ini.to_text()), 96);
        assert_eq!(back.adapter, c.adapter);
    }

    #[test]
    fn adapter_auto_roundtrips_as_empty_string() {
        let c = Config::defaults(96);
        let mut ini = Ini::new();
        c.write_ini(&mut ini);
        assert_eq!(ini.get("Graphics", "Adapter"), Some(""));
        assert!(Config::from_ini(&ini, 96).adapter.is_empty());
    }

    #[test]
    fn adapter_sanitize_trims_strips_controls_and_caps_length() {
        assert_eq!(
            Config::sanitize_adapter("  AMD Radeon 610M  "),
            "AMD Radeon 610M"
        );
        // 换行会让 ini 值在多行格式里断开，必须剔掉。
        assert_eq!(Config::sanitize_adapter("Intel\r\nUHD\t620"), "IntelUHD620");
        let long = "x".repeat(500);
        assert_eq!(Config::sanitize_adapter(&long).chars().count(), 128);
        assert_eq!(Config::sanitize_adapter("   "), "");
    }

    #[test]
    fn adapter_from_ini_is_sanitized() {
        let mut ini = Ini::new();
        ini.set("Graphics", "Adapter", "  NVIDIA RTX 5070\r\n");
        assert_eq!(Config::from_ini(&ini, 96).adapter, "NVIDIA RTX 5070");
    }
}
