//! 布局数学：度量、四边停靠的坐标映射、放大曲线、命中测试。
//!
//! 这一层是**纯函数**，不持有任何状态：同样的输入永远得到同样的输出，
//! 可以直接断言坐标，不需要建窗口看效果。
//!
//! 魔数都标了来源，改动前请先读 `PORTING.md` 的"像素级对齐"一节。

use crate::config::{Config, Edge, Key};
use crate::units;

/// 整型矩形。对应 Win32 `RECT`（左闭右开）。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    #[must_use]
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    #[must_use]
    pub const fn width(self) -> i32 {
        self.right - self.left
    }

    #[must_use]
    pub const fn height(self) -> i32 {
        self.bottom - self.top
    }
}

/// 启动弹跳的持续时间（毫秒）。
pub const BOUNCE_MS: u64 = 1100;

/// 启动弹跳的偏移量（逻辑像素，向栏体外侧为正）。
///
/// 曲线：
///
/// ```text
/// x = t / 1100
/// off = |sin(x · 2π)| · (1 − x · 0.55) · S · 0.48
/// ```
///
/// 也就是**两次**完整弹跳（`|sin|` 让它在 0.5 与 1.0 处各达到峰值），
/// 幅度按 `1 − 0.55x` 线性衰减。返回 0 表示这一帧不动画。
///
/// `S` 参与计算意味着高 DPI 下弹得更高 —— 与图标尺寸同比，观感才一致。
#[must_use]
pub fn bounce_offset(elapsed_ms: u64, s: f32) -> f32 {
    if elapsed_ms >= BOUNCE_MS {
        return 0.0;
    }
    let x = elapsed_ms as f32 / BOUNCE_MS as f32;
    let phase = x * std::f32::consts::PI * 2.0;
    phase.sin().abs() * (1.0 - x * 0.55) * s * 0.48
}

/// 这一帧是否需要继续推进弹跳动画。
#[must_use]
pub fn bounce_active(elapsed_ms: u64) -> bool {
    elapsed_ms < BOUNCE_MS
}

/// Dock 窗口左上角的屏幕坐标（物理像素）。
///
///
/// `reserved` 是 AppBar 实际拿到的那块矩形（`None` = 没预留桌面空间）。
/// 预留成功后窗口要**贴在预留区边上**，否则栏体会和"被让出来的桌面"
/// 差一截，最大化窗口下沿会盖住 Dock。
///
/// 竖直方向（左/右停靠）时窗口尺寸是 `(lh, lw)`，调用方负责传入正确的
/// `win_w` / `win_h` —— 这里只做定位，不猜尺寸。
#[must_use]
pub fn window_origin(
    edge: Edge,
    work: Rect,
    reserved: Option<Rect>,
    win_w: i32,
    win_h: i32,
) -> (i32, i32) {
    let center_x = work.left + (work.width() - win_w) / 2;
    let center_y = work.top + (work.height() - win_h) / 2;
    match edge {
        // 横排：水平居中，纵向贴顶或贴底。
        Edge::Top => (center_x, reserved.map_or(work.top, |r| r.top)),
        Edge::Bottom => (center_x, reserved.map_or(work.bottom, |r| r.bottom) - win_h),
        // 竖排：垂直居中，横向贴左或贴右。
        Edge::Left => (reserved.map_or(work.left, |r| r.left), center_y),
        Edge::Right => (reserved.map_or(work.right, |r| r.right) - win_w, center_y),
    }
}

/// 窗口的物理尺寸：横排是 `(lw, lh)`，竖排要转置成 `(lh, lw)`。
///
/// 横排取 `(lw, lh)`，竖排交换两者。
/// 这里独立成函数是因为它被"建窗口"和"AppBar 厚度"两处共用，
/// 抄错一次就会出现竖排 Dock 只有一条细缝的情况。
#[must_use]
pub fn physical_size(edge: Edge, lw: i32, lh: i32) -> (i32, i32) {
    if edge.is_horizontal() {
        (lw, lh)
    } else {
        (lh, lw)
    }
}

/// AppBar 需要预留的厚度（物理像素）。
///
/// 栏体 + 下方留白 + 4 px 余量。
/// 多出的 4 px 是为了让"最大化窗口的下沿"不贴住栏体边缘。
#[must_use]
pub fn reserve_thickness(bar_h: f32, margin: f32) -> i32 {
    (bar_h + margin + 4.0) as i32
}

/// 小组件类型。
///
/// **下标即配置语义**：`WideMask` 的位掩码按这个序号取位
/// 换序会让老配置的双倍宽设置错位。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum WidgetKind {
    /// 时钟卡片（v1.4.0 就有的那个）。
    Clock,
    /// 通知小组件。
    Notify,
    /// 控制中心。
    CtrlCenter,
    /// 资源监视器。
    Resource,
    /// 计时器 / 待办。
    Tool,
    /// 专注模式。
    Focus,
    /// 系统托盘（v1.6.5）：点一下打开 Windows 通知区域。
    Tray,
}

impl WidgetKind {
    /// 枚举序号。`WideMask` 按它取位。
    #[must_use]
    pub const fn index(self) -> u32 {
        match self {
            Self::Clock => 0,
            Self::Notify => 1,
            Self::CtrlCenter => 2,
            Self::Resource => 3,
            Self::Tool => 4,
            Self::Focus => 5,
            // 顺序固定，`WideMask` 按序号取位。
            Self::Tray => 6,
        }
    }

    /// 该小组件是否**已实现**。
    ///
    /// 只有已实现的才进条带 —— 没实现的留出位置却画不出东西，
    /// 栏体上就是一段空白，比不做还糟。
    ///
    /// 已实现：时钟（v1.4.0 就有）、资源监视器（ADR-039）、
    /// 计时器 / 待办（ADR-040）。
    /// 其余三个（通知 / 控制中心 / 专注模式）尚未实现。
    #[must_use]
    pub const fn is_implemented(self) -> bool {
        matches!(
            self,
            Self::Clock | Self::Resource | Self::Tool | Self::CtrlCenter | Self::Tray
        )
    }

    /// 该小组件是否被配置启用。
    #[must_use]
    pub fn is_enabled(self, cfg: &Config) -> bool {
        match self {
            Self::Clock => cfg.clock,
            Self::Notify => cfg.get_i32(Key::NOTIFY_W) != 0,
            Self::CtrlCenter => cfg.get_i32(Key::CTRL_CENTER) != 0,
            Self::Resource => cfg.get_i32(Key::RES_MON) != 0,
            Self::Tool => cfg.get_i32(Key::TOOL_W) != 0,
            Self::Focus => cfg.get_i32(Key::MODE_W) != 0,
            Self::Tray => cfg.get_i32(Key::TRAY_W) != 0,
        }
    }

    /// 该小组件是否被配置为双倍宽（`WideMask` 的对应位）。
    #[must_use]
    pub fn is_wide(self, cfg: &Config) -> bool {
        cfg.get_i32(Key::WIDE_MASK) & (1 << self.index()) != 0
    }

    /// 该小组件的宽度。
    ///
    /// 时钟卡片不走这个公式 —— 它的宽度由内容决定（时间 / 日期的排版），
    /// 见 [`clock_width`]。
    #[must_use]
    pub fn width(self, cfg: &Config, bar_h: f32) -> i32 {
        match self {
            Self::Clock => 0, // 由 clock_width 单独算
            Self::Notify => (bar_h * 0.92) as i32,
            Self::CtrlCenter => (bar_h * if self.is_wide(cfg) { 1.6 } else { 0.72 }) as i32,
            Self::Resource => (bar_h * if self.is_wide(cfg) { 2.2 } else { 1.16 }) as i32,
            Self::Tool => (bar_h * if self.is_wide(cfg) { 1.6 } else { 0.72 }) as i32,
            Self::Focus => (bar_h * 0.72) as i32,
            // 托盘卡片固定宽度，不支持双倍宽。
            Self::Tray => (bar_h * 0.72) as i32,
        }
    }
}

/// 小组件条带：栏体外侧的那一排卡片。
///
///
/// **只有时钟一个成员时，它与 v1.4.0 的行为逐像素一致** ——
/// 这条等价是本重构的硬约束：v1.4.0 的几何已经逐项比对通过，
/// 换成条带模型后也不能弄坏。
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct WidgetStrip {
    /// 条带里的 widget（固定顺序：时钟与通知的先后由
    /// `WOrder` 决定，其余固定在后面）。
    pub items: [Option<(WidgetKind, i32)>; 7],
    /// 条带成员数。
    pub len: usize,
    /// 条带整体占位：各成员宽度 + 间距。
    ///
    /// v1.4.0 只有时钟时等于 `clock_w + gap + 6`。
    pub extra: f32,
    /// 条带在栏体的哪一侧：右（false）/ 左（true）。
    pub on_left: bool,
}

impl WidgetStrip {
    /// 构建条带：成员宽度 + 间距累加。
    ///
    /// 顺序：
    /// `ord[6] = { wOrder ? NOTIFY : CLOCK, wOrder ? CLOCK : NOTIFY, CC, RES, TOOL, MODE }`
    /// —— `WOrder` 只决定时钟与通知谁在前，其余四个位置固定。
    ///
    /// `clock_w` 是时钟卡片自己的宽度（由内容决定，不走 `width()` 公式）。
    #[must_use]
    pub fn build(cfg: &Config, bar_h: f32, gap: f32, clock_w: i32) -> Self {
        // `WOrder` 只交换前两位，其余固定，
        // 托盘永远排在最后一个。
        let order = if cfg.get_i32(Key::W_ORDER) != 0 {
            [
                WidgetKind::Notify,
                WidgetKind::Clock,
                WidgetKind::CtrlCenter,
                WidgetKind::Resource,
                WidgetKind::Tool,
                WidgetKind::Focus,
                WidgetKind::Tray,
            ]
        } else {
            [
                WidgetKind::Clock,
                WidgetKind::Notify,
                WidgetKind::CtrlCenter,
                WidgetKind::Resource,
                WidgetKind::Tool,
                WidgetKind::Focus,
                WidgetKind::Tray,
            ]
        };

        let mut items = [None; 7];
        let mut len = 0usize;
        let mut extra = 0.0f32;
        for kind in order {
            if !kind.is_enabled(cfg) || !kind.is_implemented() {
                continue;
            }
            let w = if kind == WidgetKind::Clock {
                clock_w
            } else {
                kind.width(cfg, bar_h)
            };
            items[len] = Some((kind, w));
            len += 1;
            // 占位累加：成员宽度 + 间距 + 6。
            extra += w as f32 + gap + 6.0;
        }
        Self {
            items,
            len,
            extra,
            on_left: cfg.get_i32(Key::W_SIDE) != 0,
        }
    }

    /// 条带里某个 widget 的宽度。找不到返回 `None`。
    #[must_use]
    pub fn width_of(&self, kind: WidgetKind) -> Option<i32> {
        self.items[..self.len]
            .iter()
            .flatten()
            .find(|(k, _)| *k == kind)
            .map(|(_, w)| *w)
    }

    /// 迭代条带成员（按顺序）。
    pub fn iter(&self) -> impl Iterator<Item = (WidgetKind, i32)> + '_ {
        self.items[..self.len].iter().flatten().copied()
    }

    /// 条带里第 `i` 个 widget 的左边界（逻辑像素）。
    ///
    /// 右侧时从栏体右缘往外排，左侧时从栏体左缘往左排。
    /// `bar_l` / `bar_w` 是栏体的位置与宽度。
    #[must_use]
    pub fn left_of(&self, i: usize, bar_l: f32, bar_w: f32, gap: f32) -> Option<f32> {
        self.items.get(i)?.as_ref()?;
        let gap_w = gap + 6.0;
        if self.on_left {
            // Lay out right-to-left from the bar's left edge.
            let mut u = bar_l - gap_w;
            for j in (0..self.len).rev() {
                // `build()` fills the first `len` slots, but the fields are
                // public. A hole must resolve to `None`, not panic the UI.
                let (_, wj) = self.items[j]?;
                u -= wj as f32;
                if j == i {
                    return Some(u);
                }
                u -= gap_w;
            }
            None
        } else {
            // Lay out left-to-right from the bar's right edge.
            let mut u = bar_l + bar_w + gap_w;
            for j in 0..self.len {
                if j == i {
                    return Some(u);
                }
                let (_, wj) = self.items[j]?;
                u += wj as f32 + gap_w;
            }
            None
        }
    }
}

/// 时钟卡片宽度。
///
/// 显示内容 `ClockShow`：0 = 都显示，1 = 只日期，2 = 只时间。
/// 日期格式 `DateFmt`：0 = 中文（月日），1 = 短格式。
/// `ClockWeek` / `ClockSec` 都会改变宽度。
#[must_use]
pub fn clock_width(cfg: &Config, dpi: i32) -> i32 {
    if !cfg.clock {
        return 0;
    }
    let has_time = cfg.get_i32(Key::CLOCK_SHOW) != 2;
    let has_date = cfg.get_i32(Key::CLOCK_SHOW) != 1;
    if cfg.edge.is_horizontal() {
        let wt = if has_time {
            units::scale(
                if cfg.get_i32(Key::CLOCK_SEC) != 0 {
                    92
                } else {
                    74
                },
                dpi,
            )
        } else {
            0
        };
        let wd = if has_date {
            let base = if cfg.get_i32(Key::DATE_FMT) == 0 {
                if cfg.get_i32(Key::CLOCK_WEEK) != 0 {
                    166
                } else {
                    124
                }
            } else if cfg.get_i32(Key::CLOCK_WEEK) != 0 {
                146
            } else {
                110
            };
            units::scale(base, dpi)
        } else {
            0
        };
        wt.max(wd)
    } else {
        let lines = i32::from(has_time)
            + if has_date {
                if cfg.get_i32(Key::CLOCK_WEEK) != 0 {
                    2
                } else {
                    1
                }
            } else {
                0
            };
        units::scale(lines * 21 + 14, dpi)
    }
}

/// 一帧的度量结果。
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Metrics {
    /// 图标基准边长（`M.S`）。
    pub s: f32,
    /// 放大后的边长（`M.Z`）。
    pub z: f32,
    /// 图标间距（`M.g`）。
    pub gap: f32,
    /// 段首尾的**横向**内边距（`M.padX` = 配置 `HPad`）。
    ///
    /// v1.6.5 之前它恒等于 `pad + 2`，之后由独立键控制 —— 上下与左右
    /// 可以分别调（默认 8 / 12，而不是同一个 8）。
    pub pad_x: f32,
    /// 分割线宽度（`M.sepW`）。
    pub sep_w: f32,
    /// 栏内边距（`M.pad`）。
    pub pad: f32,
    /// 栏高 = `S + 2*pad + 4`（`M.barH`）。
    pub bar_h: f32,
    /// 栏体上方需要预留的高度（放大 + 标签气泡）。
    pub overhang: f32,
    /// 距屏幕边缘 + 倒影预留（`M.margin`）。
    pub margin: f32,
    /// 小组件条带。
    ///
    /// v1.4.0 只有一个时钟卡片，v1.6.0 把它泛化成"一排卡片"。
    pub widgets: WidgetStrip,
    /// 条带整体占位：栏体要让出这么宽的地方给卡片们。
    ///
    /// 只有时钟时 = `clock_w + gap + 6`。
    pub widget_extra: f32,
    /// 时钟卡片宽度（绘制时钟卡片时要用；条带里 Clock 成员的宽度）。
    pub clock_w: i32,
    /// 倒影高度（未开启时为 0）。
    pub refl_h: f32,
    /// 逻辑宽度（水平布局时即窗口宽）。
    pub lw: i32,
    /// 逻辑高度（水平布局时即窗口高）。
    pub lh: i32,
}

/// `f32` 夹紧。
#[must_use]
pub fn clampf(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// 栏体的基准横向跨度：`2*pad_x + n_icon*s + n_sep*sep_w + gap*(n-1)`。
///
/// 这个式子在三处出现（度量、基准中心、实际槽位），是栏体几何的唯一定义。
/// 空 Dock 例外：保持 4 格宽度，否则"拖到这里"的提示文字无处安放。
fn span(pad_x: f32, s: f32, gap: f32, sep_w: f32, n_icon: usize, n_sep: usize) -> f32 {
    let n = n_icon + n_sep;
    if n == 0 {
        return 2.0 * pad_x + s * 4.0;
    }
    let mut w = 2.0 * pad_x + n_icon as f32 * s + n_sep as f32 * sep_w;
    w += gap * (n - 1) as f32;
    w
}

/// 计算一帧的度量。
///
/// `n_items` 是**全部**条目数（含分割线），`n_sep` 是其中分割线的数量；
/// `work` 是目标显示器的**工作区**（已排除任务栏）。
#[must_use]
pub fn calc_metrics(cfg: &Config, dpi: i32, n_items: usize, n_sep: usize, work: Rect) -> Metrics {
    let s0 = cfg.icon_size as f32;
    // 放大尺寸不得小于基准尺寸，否则计算出的 `overhang` 会变成负数。
    let z0 = if cfg.zoom {
        cfg.zoom_size.max(cfg.icon_size) as f32
    } else {
        s0
    };
    let g0 = cfg.spacing as f32;
    let pad0 = cfg.pad as f32;
    // 分割线宽度由 `SepGap` 决定
    // （旧版是写死的 `16 + spacing/2`）。
    let sep0 = cfg.get_i32(Key::SEP_GAP) as f32;
    // 横向内边距自 v1.6.5 起独立成 `HPad`，
    // 不再是 `pad + 2`。默认值因此从 10 变成 12 —— 栏体两侧各宽 2 px，
    // 这是 1.6.5 起对外可见的变化。
    let pad_x0 = cfg.get_i32(Key::HPAD).max(0) as f32;

    let n_icon = n_items.saturating_sub(n_sep);
    let wb = span(pad_x0, s0, g0, sep0, n_icon, n_sep);

    let along = if cfg.edge.is_horizontal() {
        work.width() as f32
    } else {
        work.height() as f32
    };
    let limit = along - 40.0;
    let mut fit = 1.0_f32;
    if wb > limit && (wb - 2.0 * pad_x0).abs() > f32::EPSILON {
        // 图标太多、屏宽不够时整体等比缩小，下限 25%。
        fit = clampf((limit - 2.0 * pad_x0) / (wb - 2.0 * pad_x0), 0.25, 1.0);
    }

    let s = s0 * fit;
    let z = z0 * fit;
    let gap = g0 * fit;
    let sep_w = sep0 * fit;
    let pad = pad0;
    let pad_x = pad_x0;

    // 倒影高度可调：`ReflSize` 百分比 + `ReflGap` 间距。
    let refl_h = if cfg.reflect && cfg.edge == Edge::Bottom {
        s * (cfg.get_i32(Key::REFL_SIZE) as f32 / 100.0) + cfg.get_i32(Key::REFL_GAP) as f32 + 2.0
    } else {
        0.0
    };
    let margin = cfg.margin as f32 + refl_h;
    let bar_h = s + 2.0 * pad + 4.0;

    // 小组件条带。v1.4.0 只有时钟一个成员，
    // 这里的 `extra` 恰好等于旧版的 `clock_extra`。
    let clock_w = clock_width(cfg, dpi);
    let widgets = WidgetStrip::build(cfg, bar_h, gap, clock_w);
    let widget_extra = widgets.extra;

    // 栏体之上要留出的空间：放大溢出 + 半个图标（标签气泡）+ 固定余量。
    let mut overhang = (z - s - pad).max(0.0) + s * 0.55 + 34.0 + 8.0;
    if !cfg.edge.is_horizontal() {
        // 竖向停靠时气泡朝内，额外留 170 px 给长名称。
        overhang += 170.0;
    }

    let mut w = wb * fit + (z - s) * 6.0 + s + 260.0 + widget_extra;
    if n_items == 0 {
        w = wb + 260.0;
    }
    if w > along {
        w = along;
    }

    Metrics {
        s,
        z,
        gap,
        pad_x,
        sep_w,
        pad,
        bar_h,
        overhang,
        margin,
        widgets,
        widget_extra,
        clock_w,
        refl_h,
        lw: w as i32,
        lh: (bar_h + overhang + margin) as i32 + 2,
    }
}

/// 逻辑坐标 → 窗口坐标。
///
/// 关键点：`lh` 只用于顶部/左侧的翻转；调用方必须传**当前逻辑高度**。
#[must_use]
pub fn map_rect(edge: Edge, u: f32, v: f32, w: f32, h: f32, lh: f32) -> (f32, f32, f32, f32) {
    match edge {
        Edge::Top => (u, lh - v - h, w, h),
        Edge::Left => (lh - v - h, u, h, w),
        Edge::Right => (v, u, h, w),
        Edge::Bottom => (u, v, w, h),
    }
}

/// 窗口坐标 → 逻辑坐标。
#[must_use]
pub fn to_logical(edge: Edge, px: f32, py: f32, lh: f32) -> (f32, f32) {
    match edge {
        Edge::Top => (px, lh - py),
        Edge::Left => (py, lh - px),
        Edge::Right => (py, px),
        Edge::Bottom => (px, py),
    }
}

/// 光标是否算"贴着 Dock"。
///
/// `hot` 是整条停靠带（图标 + 小组件）在**屏幕**物理坐标下的外接矩形，
/// `inner` 是朝屏幕里的伸展量（图标放大后的高度、气泡探出去的部分）。
/// `work` 是当前显示器的工作区 —— `Reserve=1` 时它的边界就是 Dock 的内缘，
/// `Reserve=0` 时是屏幕边。`keep` 表示 Dock 当前是展开的。
///
/// ## 为什么必须只有一套判定
///
/// 早先按状态各用一套：收起态"贴边 + 落在窗口宽度内"，展开态"落在图标条
/// 内"。两者在小组件那一段不重合 —— 光标挪到时钟上时展开态判"不靠近"、
/// 收起态立刻判"靠近"，于是 Dock 每 450 ms 收起又弹出一次（实测日志
/// `target=false near=true` / `target=true near=false` 交替）。
///
/// 现在两态共用这一个函数，差别只有容差：展开态的容差更大（[`HYSTERESIS`]），
/// 也就是施密特触发器 —— 光标在边界附近慢慢挪不会让 Dock 反复抽搐。
#[must_use]
pub fn near_dock(
    edge: Edge,
    cursor: (i32, i32),
    hot: (f32, f32, f32, f32),
    work: Rect,
    inner: f32,
    keep: bool,
) -> bool {
    /// 光标与停靠带之间的容差（物理像素）。图标放大是渐变的，边缘那几
    /// 个像素判进判出会让人以为 Dock "抖"。
    const SLACK: f32 = 8.0;
    /// 已经展开时额外放宽的量：收起要"真的走远了"才算数。
    const HYSTERESIS: f32 = 32.0;
    /// 贴边判定的容差：屏幕边沿本身只有一两个像素好点。
    const EDGE_REVEAL: f32 = 2.0;

    let slack = if keep { SLACK + HYSTERESIS } else { SLACK };
    let (x, y, w, h) = hot;
    // 朝屏幕里伸展：底停靠往上、顶停靠往下、左停靠往右、右停靠往左。
    let (x, y, w, h) = match edge {
        Edge::Bottom => (x, y - inner, w, h + inner),
        Edge::Top => (x, y, w, h + inner),
        Edge::Left => (x, y, w + inner, h),
        Edge::Right => (x - inner, y, w + inner, h),
    };
    let (cx, cy) = (cursor.0 as f32, cursor.1 as f32);

    // ① 落在停靠带（含伸展量与容差）里。
    if cx >= x - slack && cx <= x + w + slack && cy >= y - slack && cy <= y + h + slack {
        return true;
    }

    // ② 贴住工作区边沿、且沿停靠方向落在带子跨度内 —— 收起态的"探头"。
    let (along, at_edge) = match edge {
        Edge::Bottom => (
            cx >= x && cx <= x + w,
            cy >= work.bottom as f32 - EDGE_REVEAL,
        ),
        Edge::Top => (cx >= x && cx <= x + w, cy <= work.top as f32 + EDGE_REVEAL),
        Edge::Left => (cy >= y && cy <= y + h, cx <= work.left as f32 + EDGE_REVEAL),
        Edge::Right => (
            cy >= y && cy <= y + h,
            cx >= work.right as f32 - EDGE_REVEAL,
        ),
    };
    along && at_edge
}
/// 鼠标放大的权重曲线：中心 1.0，半径外 0.0，中间为余弦过渡。
///
/// 权重 = `0.5 * (1 + cos(PI * d / R))`。用余弦而不是线性，
/// 是因为线性在边界处一阶不连续，视觉上会有"折角"。
#[must_use]
pub fn magnify_weight(distance: f32, radius: f32) -> f32 {
    if radius <= 0.0 || distance.abs() >= radius {
        return 0.0;
    }
    let t = std::f32::consts::PI * distance.abs() / radius;
    0.5 * (1.0 + t.cos())
}

/// 命中测试：给定逻辑横坐标，返回条目下标。
///
/// 判据是"最近的槽位中心 + 当前尺寸的一半"，而不是矩形包含：
/// 同样的判定（用槽位中心距离，而不是矩形包含），因为放大动画期间
/// 图标的实际矩形与槽位并不重合 —— 用矩形判定会导致悬停抖动。
#[must_use]
#[expect(
    clippy::too_many_arguments,
    reason = "geometry bundle maps 1:1 to upstream"
)]
pub fn hit_index(
    u: f32,
    v: f32,
    slot_l: &[f32],
    slot_r: &[f32],
    visible: &[bool],
    gap: f32,
    bar_top: f32,
    zoom: f32,
    lh: f32,
) -> Option<usize> {
    // 纵向先过范围：栏体上方（放大溢出区）与窗口下方都不算命中。
    if v < bar_top - zoom - 16.0 || v > lh {
        return None;
    }
    let half = gap / 2.0;
    let n = slot_l.len().min(slot_r.len());
    for i in 0..n {
        // 正在消失的图标（cur <= 1）不参与命中。
        if !visible.get(i).copied().unwrap_or(false) {
            continue;
        }
        // 取**从左到右第一个**命中，不是"最近的那个"：
        // 放大后相邻图标会重叠，两个都覆盖鼠标点时取左边那个。
        if u >= slot_l[i] - half && u < slot_r[i] + half {
            return Some(i);
        }
    }
    None
}

/// 把鼠标的**实际**位置（此时槽位已被放大挪动）
/// 映射回**基准**位置（未放大时的槽位）。
///
/// 放大曲线必须用基准位置算权重：图标一放大，槽位就跟着移动，
/// 直接用实际位置会让"鼠标属于哪个图标"随放大程度漂移。
///
/// `base_c` / `act_c` 分别是基准中心与实际中心。
#[must_use]
pub fn map_actual_to_base(mx: f32, base_c: &[f32], act_c: &[f32]) -> f32 {
    // 数量对不上（条目变了）就放弃映射。
    if base_c.len() != act_c.len() {
        return mx;
    }
    let n = base_c.len();
    if n == 0 {
        return mx;
    }
    if n == 1 {
        return base_c[0] + (mx - act_c[0]);
    }
    // 落在两端之外：按端点做等距平移。
    if mx <= act_c[0] {
        return base_c[0] + (mx - act_c[0]);
    }
    if mx >= act_c[n - 1] {
        return base_c[n - 1] + (mx - act_c[n - 1]);
    }
    // 落在某两个实际中心之间：线性插值到基准中心区间。
    for i in 0..n - 1 {
        if mx >= act_c[i] && mx < act_c[i + 1] {
            let d = act_c[i + 1] - act_c[i];
            let t = if d > 0.001 { (mx - act_c[i]) / d } else { 0.0 };
            return base_c[i] + t * (base_c[i + 1] - base_c[i]);
        }
    }
    mx
}

// ---------------------------------------------------------------------------
// 槽位布局：从度量到"每个图标画在哪、多大、鼠标命中谁"
// ---------------------------------------------------------------------------
//
// 这一段曾经在 GPU 版与 GDI+ 版各写一遍、常量也各写一遍，两份实现已经开始
// 漂移（GDI+ 版算基准宽度时用了动画中的 `cur`，导致基准槽位每帧漂移；
// GPU 版用固定的 `s`，是对的）。收进 core 后只有一份实现，且能单测覆盖。
// GDI+ 那一路已随 `minidock-ui` / `minidock-gfx` 一并删除（ADR-035）。

/// 放大动画的每帧收敛系数。
pub const SIZE_LERP: f32 = 0.38;
/// 动画收敛阈值：差值小于它就直接吸附。
pub const SNAP: f32 = 0.3;
/// 放大影响的半径系数。
pub const MAG_RADIUS: f32 = 2.6;

/// 槽位类型。分割线的槽宽由 `SepGap` 决定，与图标不同。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SlotKind {
    Icon,
    Separator,
}

impl SlotKind {
    #[must_use]
    pub const fn is_separator(self) -> bool {
        matches!(self, Self::Separator)
    }

    /// 该槽位未放大时的宽度。
    #[must_use]
    pub const fn base_width(self, m: &Metrics) -> f32 {
        if self.is_separator() {
            m.sep_w
        } else {
            m.s
        }
    }
}

/// 栏体与图标的纵向基线。四边停靠共用同一套基线（横向翻转由 `map_rect` 负责）。
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Bars {
    /// 栏体上边界。
    pub top: f32,
    /// 栏体下边界。
    pub bottom: f32,
    /// 图标底边（图标从这里往上长）。
    pub icon_bottom: f32,
}

/// 由度量推出纵向基线。
#[must_use]
pub fn bars(m: &Metrics) -> Bars {
    let bottom = m.lh as f32 - m.margin - 2.0;
    Bars {
        top: bottom - m.bar_h,
        bottom,
        icon_bottom: bottom - m.pad - 2.0,
    }
}

/// 基准跨度（未放大时的栏宽）。
#[must_use]
pub fn base_width(m: &Metrics, kinds: &[SlotKind]) -> f32 {
    let n_sep = kinds.iter().filter(|k| k.is_separator()).count();
    span(m.pad_x, m.s, m.gap, m.sep_w, kinds.len() - n_sep, n_sep)
}

/// 基准中心（未放大时的槽位中心），用于算放大权重。
///
/// `widget_extra` 是小组件条带的占位宽度：有它时整条栏体要左移一半。
#[must_use]
pub fn base_centers(m: &Metrics, kinds: &[SlotKind], widget_extra: f32) -> Vec<f32> {
    let left = (m.lw as f32 - base_width(m, kinds) - widget_extra) / 2.0;
    let mut x = left + m.pad_x;
    let mut out = Vec::with_capacity(kinds.len());
    for &kind in kinds {
        let w = kind.base_width(m);
        out.push(x + w / 2.0);
        x += w + m.gap;
    }
    out
}

/// 一帧的交互状态。三个字段总是一起出现，打包后调用点也更好读。
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct View {
    /// 鼠标位置，单位与 `Metrics` 一致（**物理像素**）；横坐标为负表示不在窗口内。
    ///
    /// 必须与 `slots.slot_l` / `base_c` 同单位，否则命中与放大权重都会偏移
    /// 一个 DPI 缩放因子。
    pub mouse: (f32, f32),
    /// 鼠标是否在窗口内。
    pub hover: bool,
    /// 隐藏进度：0 = 完全显示，1 = 完全隐藏。
    pub hide_cur: f32,
}

/// 本帧每个槽位的目标边长。
///
/// 权重必须用**基准**位置算：图标一放大槽位就移动，直接用实际位置会让
/// "鼠标属于哪个图标"随放大程度漂移。
#[must_use]
pub fn magnify_targets(
    m: &Metrics,
    kinds: &[SlotKind],
    base_c: &[f32],
    prev_act_c: &[f32],
    view: View,
    zoom: bool,
) -> Vec<f32> {
    let mag = zoom && view.hover && !kinds.is_empty() && view.mouse.0 >= 0.0 && view.hide_cur < 0.5;
    if !mag {
        return kinds.iter().map(|k| k.base_width(m)).collect();
    }
    let u = map_actual_to_base(view.mouse.0, base_c, prev_act_c);
    let r = (m.s + m.gap) * MAG_RADIUS;
    kinds
        .iter()
        .enumerate()
        .map(|(i, k)| {
            if k.is_separator() {
                m.sep_w
            } else {
                let w = magnify_weight(base_c.get(i).copied().unwrap_or(0.0) - u, r);
                m.s + (m.z - m.s) * w
            }
        })
        .collect()
}

/// 向目标值指数逼近一帧。返回是否发生了移动（即还需要下一帧）。
pub fn lerp_to(cur: &mut f32, target: f32) -> bool {
    let d = target - *cur;
    if d.abs() > SNAP {
        *cur += d * SIZE_LERP;
        true
    } else {
        *cur = target;
        false
    }
}

/// 本帧的实际槽位（放大之后）。全部物理像素。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Slots {
    /// 栏体左边界。
    pub bar_l: f32,
    /// 栏体宽度。
    pub bar_w: f32,
    /// 每个槽位的左边界。
    pub slot_l: Vec<f32>,
    /// 每个槽位的右边界。
    pub slot_r: Vec<f32>,
    /// 每个槽位的实际中心。
    pub act_c: Vec<f32>,
}

/// 由当前边长推出实际槽位。`cur` 的顺序与条目一致。
///
/// 栏宽超过可用宽度时整体截断（`max_w`），此时右端图标会被裁掉 ——
/// 宁可裁边，也不要让栏体溢出屏幕。
#[must_use]
pub fn place_slots(m: &Metrics, cur: &[f32], widget_extra: f32) -> Slots {
    let n = cur.len();
    let total = if n == 0 {
        2.0 * m.pad_x + m.s * 4.0
    } else {
        2.0 * m.pad_x + cur.iter().sum::<f32>() + m.gap * (n - 1) as f32
    };
    let bar_w = total.min(m.lw as f32 - 4.0 - widget_extra);
    let bar_l = (m.lw as f32 - bar_w - widget_extra) / 2.0;

    let mut x = bar_l + m.pad_x;
    let mut slot_l = Vec::with_capacity(n);
    let mut slot_r = Vec::with_capacity(n);
    let mut act_c = Vec::with_capacity(n);
    for &c in cur {
        slot_l.push(x);
        slot_r.push(x + c);
        act_c.push(x + c / 2.0);
        x += c + m.gap;
    }
    Slots {
        bar_l,
        bar_w,
        slot_l,
        slot_r,
        act_c,
    }
}

/// 命中测试。隐藏中（`hide_cur >= 0.5`）或鼠标不在窗口内一律不命中。
///
/// 用本帧槽位而不是上一帧：否则气泡会比图标慢一帧。
#[must_use]
pub fn hit_test(m: &Metrics, bars: &Bars, slots: &Slots, cur: &[f32], view: View) -> Option<usize> {
    if !view.hover || view.hide_cur >= 0.5 {
        return None;
    }
    // 正在消失的图标（`cur <= 1`）不参与命中。
    let visible: Vec<bool> = cur.iter().map(|c| *c > 1.0).collect();
    hit_index(
        view.mouse.0,
        view.mouse.1,
        &slots.slot_l,
        &slots.slot_r,
        &visible,
        m.gap,
        bars.top,
        m.z,
        m.lh as f32,
    )
}

/// 拖动分割线缩放时的新尺寸（`SepScale`）。
///
/// `dy` 是鼠标相对按下时的**物理像素**位移：往上拖为负 → 图标变大。
/// `0.6` 是上游的细步长（一次 `WM_MOUSEMOVE` 不会跳一整格）；16 ms 节流
/// 在窗口层做，这里只管算。
///
/// 返回 `(icon, zoom)`：图标夹在 `24..128`，放大尺寸按**初始比例**同比缩放
/// 后夹在 `32..220`。两个数还会被 [`crate::config::Config::apply_i32`] 再夹
/// 一次（那层夹的是配置本身的取值范围）。
///
/// 为什么用 `zoom0 / icon0` 而不是每帧的当前值：以拖动起点为基准，
/// 来回拖不会累积取整误差，松手回到原位时尺寸也回到原值。
#[must_use]
pub fn sep_scale(dy: f32, icon0: i32, zoom0: i32) -> (i32, i32) {
    // 截断而不是四舍五入：与上游 `(int)(dy * 0.6f)` 一致，慢速拖动时
    // 不会因为浮点抖动在同一格上反复 ±1。
    let delta = (dy * 0.6) as i32;
    let icon = (icon0 - delta).clamp(24, 128);
    let base = icon0.max(1) as f32;
    let zoom = (zoom0 as f32 * icon as f32 / base).round() as i32;
    (icon, zoom.clamp(32, 220))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn cfg96() -> Config {
        Config::defaults(96)
    }

    fn work() -> Rect {
        Rect::new(0, 0, 1920, 1040)
    }

    /// A hole in the strip resolves to `None` instead of panicking.
    #[test]
    fn sparse_widget_strip_does_not_panic() {
        let mut items = [None; 7];
        items[0] = Some((WidgetKind::Clock, 100));
        items[1] = None; // hole
        items[2] = Some((WidgetKind::Resource, 90));
        let strip = WidgetStrip {
            items,
            len: 3,
            extra: 0.0,
            on_left: false,
        };

        // i=0 returns before reading the hole.
        assert_eq!(strip.left_of(0, 500.0, 200.0, 8.0), Some(714.0));
        assert_eq!(strip.left_of(2, 500.0, 200.0, 8.0), None);
        assert_eq!(strip.left_of(9, 500.0, 200.0, 8.0), None);

        // Left-side layout walks backwards; only the leftmost slot sees the hole.
        let left = WidgetStrip {
            on_left: true,
            ..strip
        };
        assert_eq!(left.left_of(2, 500.0, 200.0, 8.0), Some(396.0));
        assert_eq!(left.left_of(0, 500.0, 200.0, 8.0), None);
    }

    #[test]
    fn empty_dock_keeps_four_slots_wide() {
        let m = calc_metrics(&cfg96(), 96, 0, 0, work());
        // 2*12 + 48*4 = 216，再加固定 260 的交互余量。
        // 12 是 `HPad` 的默认值（v1.6.5 起横向内边距独立，原来是 `pad + 2` = 10）。
        assert_eq!(m.lw, 216 + 260);
        assert!(m.bar_h > m.s, "栏高必须大于图标");
    }

    #[test]
    fn bar_height_matches_c_formula() {
        let c = cfg96();
        let m = calc_metrics(&c, 96, 3, 0, work());
        assert_eq!(m.bar_h, m.s + 2.0 * c.pad as f32 + 4.0);
        // 横向内边距自 v1.6.5 起由 `HPad` 单独控制，不再是 `pad + 2`。
        assert_eq!(m.pad_x, c.get_i32(Key::HPAD) as f32);
        assert_ne!(m.pad_x, c.pad as f32 + 2.0, "两者默认值已经不同");
        assert_eq!(m.s, c.icon_size as f32);
    }

    #[test]
    fn too_many_items_shrink_instead_of_overflowing() {
        let c = cfg96();
        let m = calc_metrics(&c, 96, 128, 0, Rect::new(0, 0, 800, 600));
        assert!(m.s < c.icon_size as f32, "应当等比缩小");
        assert!(m.s >= c.icon_size as f32 * 0.25, "不得小于 25%");
        assert!(m.lw <= 800, "逻辑宽度不得超出工作区");
    }

    #[test]
    fn clock_adds_width_only_when_enabled() {
        let mut c = cfg96();
        let without = calc_metrics(&c, 96, 3, 0, work());
        c.clock = true;
        let with = calc_metrics(&c, 96, 3, 0, work());
        assert!(with.widget_extra > 0.0);
        // 默认同时显示时间与日期、带周几、中文日期格式
        // → 宽度取日期那一项（Sc(166)）。
        assert_eq!(with.clock_w, 166);
        assert!(with.lw > without.lw);
    }

    /// 时钟卡片宽度按"显示什么 / 什么格式"变化。
    #[test]
    fn clock_width_follows_upstream_rules() {
        let mut c = cfg96();
        c.clock = true;

        // 选项：0 = 时间+日期，1 = 仅时间，2 = 仅日期。

        // 仅显示时间（ClockShow=1），不带秒 → Sc(74)。
        c.apply_i32(Key::CLOCK_SHOW, 1);
        assert_eq!(calc_metrics(&c, 96, 3, 0, work()).clock_w, 74);

        // 仅显示时间 + 带秒 → Sc(92)。
        c.apply_i32(Key::CLOCK_SEC, 1);
        assert_eq!(calc_metrics(&c, 96, 3, 0, work()).clock_w, 92);

        // 仅显示日期（ClockShow=2），中文格式 + 周几 → Sc(166)。
        c.apply_i32(Key::CLOCK_SHOW, 2);
        c.apply_i32(Key::CLOCK_SEC, 0);
        assert_eq!(calc_metrics(&c, 96, 3, 0, work()).clock_w, 166);

        // 短格式 + 周几 → Sc(146)。
        c.apply_i32(Key::DATE_FMT, 1);
        assert_eq!(calc_metrics(&c, 96, 3, 0, work()).clock_w, 146);

        // 短格式 + 不带周几 → Sc(110)。
        c.apply_i32(Key::CLOCK_WEEK, 0);
        assert_eq!(calc_metrics(&c, 96, 3, 0, work()).clock_w, 110);
    }

    /// 小组件条带 —— 只开时钟时必须与 v1.4.0 等价。
    ///
    /// 这是本次重构的硬约束：v1.4.0 的几何已经逐项比对通过，
    /// 换成条带模型后不能有一像素的差别。
    #[test]
    fn widget_strip_matches_v140_when_only_clock() {
        let mut c = cfg96();
        c.clock = true;
        // 必须显式关掉另外三个已实现的卡片：默认
        // `ResMon=1` / `ToolW=1` / `CtrlCenter=1`，而 v1.4.0 压根没有它们
        // —— 带着就不可能与 v1.4.0 等价了。
        c.apply_i32(Key::RES_MON, 0);
        c.apply_i32(Key::TOOL_W, 0);
        c.apply_i32(Key::CTRL_CENTER, 0);
        c.apply_i32(Key::TRAY_W, 0);
        let m = calc_metrics(&c, 96, 3, 0, work());

        // 条带里只有时钟一个成员。
        assert_eq!(m.widgets.len, 1);
        assert_eq!(m.widgets.width_of(WidgetKind::Clock), Some(166));

        // v1.4.0 的 `g_ce = g_clockW + M.g + 6`。
        let legacy = m.clock_w as f32 + m.gap + 6.0;
        assert!(
            (m.widget_extra - legacy).abs() < 0.01,
            "条带占位 {} 与 v1.4.0 的 {} 不一致",
            m.widget_extra,
            legacy
        );
        // 旧的 `clock_extra` 概念已被 `widget_extra` 取代。
        assert!((m.widget_extra - m.clock_w as f32 - m.gap - 6.0).abs() < 0.01);
    }

    /// 时钟关闭时条带为空，占位为 0。
    #[test]
    fn widget_strip_is_empty_without_clock() {
        let mut c = cfg96();
        // 五个已实现的默认都开着，要关就都关。
        c.apply_i32(Key::RES_MON, 0);
        c.apply_i32(Key::TOOL_W, 0);
        c.apply_i32(Key::CTRL_CENTER, 0);
        c.apply_i32(Key::TRAY_W, 0);
        let m = calc_metrics(&c, 96, 3, 0, work());
        assert_eq!(m.widgets.len, 0);
        assert_eq!(m.widget_extra, 0.0);
        assert!(m.widgets.width_of(WidgetKind::Clock).is_none());
    }

    /// 未实现的小组件不进条带 —— 否则栏体上会留出一段空白。
    ///
    /// 默认配置把控制中心 / 资源监视器 / 计时器 / 专注模式全打开，
    /// 我们还没实现的（通知 / 专注模式）即使键是 1 也不该占位。
    #[test]
    fn unimplemented_widgets_do_not_take_space() {
        let mut c = cfg96();
        c.clock = true;
        // 默认值就是这些全开。
        assert_eq!(c.get_i32(Key::CTRL_CENTER), 1);
        assert_eq!(c.get_i32(Key::TOOL_W), 1);
        assert_eq!(c.get_i32(Key::MODE_W), 1);

        let m = calc_metrics(&c, 96, 3, 0, work());
        // 已实现的五个都进条带：时钟 / 控制中心 / 资源监视器 / 计时器 / 托盘。
        assert_eq!(
            m.widgets.len, 5,
            "时钟 + 控制中心 + 资源监视器 + 计时器 + 托盘"
        );
        assert!(m.widgets.width_of(WidgetKind::CtrlCenter).is_some());
        assert!(m.widgets.width_of(WidgetKind::Resource).is_some());
        assert!(m.widgets.width_of(WidgetKind::Tool).is_some());
        assert!(m.widgets.width_of(WidgetKind::Tray).is_some());
        // 这两个还没实现，不能占位。
        assert!(m.widgets.width_of(WidgetKind::Notify).is_none());
        assert!(m.widgets.width_of(WidgetKind::Focus).is_none());
    }

    /// 资源监视器卡片进条带（1.6.0 新增的第一个小组件）。
    #[test]
    fn resource_widget_enters_the_strip() {
        let mut c = cfg96();
        c.clock = true;
        c.apply_i32(Key::RES_MON, 1);
        c.apply_i32(Key::TOOL_W, 0);
        c.apply_i32(Key::CTRL_CENTER, 0);
        c.apply_i32(Key::TRAY_W, 0);
        let m = calc_metrics(&c, 96, 3, 0, work());

        assert_eq!(m.widgets.len, 2, "时钟 + 资源监视器");
        assert!(m.widgets.width_of(WidgetKind::Resource).is_some());
        // 顺序：时钟在前，资源监视器在控制中心之后（已关掉）。
        let kinds: Vec<WidgetKind> = m.widgets.iter().map(|(k, _)| k).collect();
        assert_eq!(kinds[0], WidgetKind::Clock);
        assert_eq!(kinds[1], WidgetKind::Resource);
    }

    /// 托盘卡片：默认开启（`TrayW=1`），宽度 = 栏高 × 0.72，永远排最后。
    #[test]
    fn tray_widget_is_last_and_fixed_width() {
        let c = cfg96();
        assert_eq!(c.get_i32(Key::TRAY_W), 1, "1.6.5 起默认开");

        let m = calc_metrics(&c, 96, 3, 0, work());
        let kinds: Vec<WidgetKind> = m.widgets.iter().map(|(k, _)| k).collect();
        assert_eq!(
            kinds.last().copied(),
            Some(WidgetKind::Tray),
            "托盘应当在条带最外侧"
        );

        let bar_h = m.bar_h;
        assert_eq!(
            m.widgets.width_of(WidgetKind::Tray),
            Some((bar_h * 0.72) as i32)
        );
        // `WOrder` 只交换时钟与通知，不影响托盘的位置。
        let mut swapped = c;
        swapped.apply_i32(Key::W_ORDER, 1);
        let m2 = calc_metrics(&swapped, 96, 3, 0, work());
        let kinds2: Vec<WidgetKind> = m2.widgets.iter().map(|(k, _)| k).collect();
        assert_eq!(kinds2.last().copied(), Some(WidgetKind::Tray));
    }

    /// 计时器卡片进条带，宽度 = 栏高 × 0.72（双倍宽系数 1.6）。
    #[test]
    fn tool_widget_enters_the_strip() {
        let mut c = cfg96();
        c.apply_i32(Key::TOOL_W, 1);
        c.apply_i32(Key::RES_MON, 0);

        let m = calc_metrics(&c, 96, 3, 0, work());
        // 用真实的栏高，别写死 —— cfg96 的 bar_h 是 `s + 2*pad + 4`。
        let bar_h = m.bar_h;
        assert_eq!(
            m.widgets.width_of(WidgetKind::Tool),
            Some((bar_h * 0.72) as i32)
        );

        // WideMask 的第 4 位 = W_TOOL。
        c.apply_i32(Key::WIDE_MASK, 1 << 4);
        assert_eq!(WidgetKind::Tool.width(&c, bar_h), (bar_h * 1.6) as i32);
    }

    /// 资源卡片宽度 = 栏高 × 1.16（双倍宽时 × 2.2）。
    #[test]
    fn resource_widget_width_follows_upstream() {
        let mut c = cfg96();
        c.apply_i32(Key::RES_MON, 1);
        let bar_h = 76.0_f32;
        assert_eq!(WidgetKind::Resource.width(&c, bar_h), (bar_h * 1.16) as i32);
        // WideMask 的第 3 位 = W_RES。
        c.apply_i32(Key::WIDE_MASK, 1 << 3);
        assert_eq!(WidgetKind::Resource.width(&c, bar_h), (bar_h * 2.2) as i32);
    }

    /// `ResMon=0` 时资源卡片不进条带。
    #[test]
    fn resource_widget_respects_config() {
        let mut c = cfg96();
        c.clock = true;
        c.apply_i32(Key::RES_MON, 0);
        c.apply_i32(Key::TOOL_W, 0);
        c.apply_i32(Key::CTRL_CENTER, 0);
        c.apply_i32(Key::TRAY_W, 0);
        let m = calc_metrics(&c, 96, 3, 0, work());
        assert_eq!(m.widgets.len, 1);
        assert!(m.widgets.width_of(WidgetKind::Resource).is_none());
    }

    /// 条带在右侧时：第一个成员紧贴栏体右缘，间隔 `gap + 6`。
    #[test]
    fn widget_strip_positions_on_the_right() {
        let mut c = cfg96();
        c.clock = true;
        let m = calc_metrics(&c, 96, 3, 0, work());
        let strip = &m.widgets;
        let bar_l = 100.0_f32;
        let bar_w = 300.0_f32;

        let left = strip.left_of(0, bar_l, bar_w, m.gap).expect("应当有位置");
        assert!(
            (left - (bar_l + bar_w + m.gap + 6.0)).abs() < 0.01,
            "左侧条带的第一个成员应当紧贴栏体右缘，实际 {left}"
        );
    }

    /// 条带在左侧时（`WSide=1`）：紧贴栏体左缘，往左排。
    #[test]
    fn widget_strip_positions_on_the_left() {
        let mut c = cfg96();
        c.clock = true;
        // 只留时钟，才能验证"紧贴栏体左缘"这个简单关系。
        c.apply_i32(Key::RES_MON, 0);
        c.apply_i32(Key::TOOL_W, 0);
        c.apply_i32(Key::CTRL_CENTER, 0);
        c.apply_i32(Key::TRAY_W, 0);
        c.apply_i32(Key::W_SIDE, 1);
        let m = calc_metrics(&c, 96, 3, 0, work());
        assert!(m.widgets.on_left, "应当记为左侧条带");

        let bar_l = 100.0_f32;
        let bar_w = 300.0_f32;
        let left = m
            .widgets
            .left_of(0, bar_l, bar_w, m.gap)
            .expect("应当有位置");
        // 左侧：`u = barL - gapW` 再减去自身宽度。
        let want = bar_l - (m.gap + 6.0) - m.clock_w as f32;
        assert!(
            (left - want).abs() < 0.01,
            "左侧条带位置 {left} 与预期 {want} 不符"
        );
    }

    /// 左侧条带里，靠栏体的是**最后一个**成员（倒着排）。
    #[test]
    fn widget_strip_left_side_order_is_reversed() {
        let mut c = cfg96();
        c.clock = true;
        c.apply_i32(Key::RES_MON, 1);
        c.apply_i32(Key::TOOL_W, 0);
        c.apply_i32(Key::CTRL_CENTER, 0);
        c.apply_i32(Key::TRAY_W, 0);
        c.apply_i32(Key::W_SIDE, 1);
        let m = calc_metrics(&c, 96, 3, 0, work());
        let bar_l = 100.0_f32;
        let bar_w = 300.0_f32;

        // 条带顺序：时钟(0)、资源(1)。左侧时**最后一个**贴栏体。
        let clock = m.widgets.left_of(0, bar_l, bar_w, m.gap).expect("时钟");
        let res = m.widgets.left_of(1, bar_l, bar_w, m.gap).expect("资源");
        assert!(
            res > clock,
            "左侧条带里资源卡片({res})应当在时钟({clock})右边、贴着栏体"
        );
        // 资源卡片右缘贴着栏体左缘（间隔 gap + 6）。
        let res_w = m.widgets.width_of(WidgetKind::Resource).expect("宽度");
        let gap_w = m.gap + 6.0;
        assert!((res + res_w as f32 - (bar_l - gap_w)).abs() < 0.01);
    }

    /// `WSide` 不改变占位宽度，只改变位置。
    #[test]
    fn widget_side_does_not_change_extra() {
        let mut c = cfg96();
        c.clock = true;
        let right = calc_metrics(&c, 96, 3, 0, work());
        c.apply_i32(Key::W_SIDE, 1);
        let left = calc_metrics(&c, 96, 3, 0, work());
        assert!(
            (right.widget_extra - left.widget_extra).abs() < 0.01,
            "左右两侧的占位应当相同"
        );
        assert_ne!(right.widgets.on_left, left.widgets.on_left);
    }

    /// 小组件宽度公式。
    ///
    /// 只测已实现的时钟（宽度由内容定），其余的按公式核对，
    /// 等实现时直接可用。
    #[test]
    fn widget_widths_follow_upstream_formulas() {
        let c = cfg96();
        let bar_h = 76.0_f32; // 默认栏高（48 + 2*8 + 4）
        assert_eq!(WidgetKind::Notify.width(&c, bar_h), (bar_h * 0.92) as i32);
        assert_eq!(
            WidgetKind::CtrlCenter.width(&c, bar_h),
            (bar_h * 0.72) as i32
        );
        assert_eq!(WidgetKind::Resource.width(&c, bar_h), (bar_h * 1.16) as i32);
        assert_eq!(WidgetKind::Tool.width(&c, bar_h), (bar_h * 0.72) as i32);
        assert_eq!(WidgetKind::Focus.width(&c, bar_h), (bar_h * 0.72) as i32);
    }

    /// 双倍宽由 `WideMask` 的对应位决定（位序 = 枚举序号）。
    #[test]
    fn wide_mask_doubles_widget_width() {
        let mut c = cfg96();
        let bar_h = 76.0_f32;
        assert_eq!(WidgetKind::Resource.width(&c, bar_h), (bar_h * 1.16) as i32);
        // 第 3 位 = W_RES（枚举：CLOCK=0 NOTIFY=1 CC=2 RES=3）。
        c.apply_i32(Key::WIDE_MASK, 1 << 3);
        assert_eq!(WidgetKind::Resource.width(&c, bar_h), (bar_h * 2.2) as i32);
        // 别的位不影响它。
        c.apply_i32(Key::WIDE_MASK, 1 << 5);
        assert_eq!(WidgetKind::Resource.width(&c, bar_h), (bar_h * 1.16) as i32);
    }

    /// 分割线宽度由 `SepGap` 决定。
    #[test]
    fn separator_width_follows_sep_gap() {
        let mut c = cfg96();
        let before = calc_metrics(&c, 96, 4, 1, work());
        c.apply_i32(Key::SEP_GAP, 20);
        let after = calc_metrics(&c, 96, 4, 1, work());
        assert!(after.sep_w > before.sep_w);
        assert_eq!(after.sep_w, 20.0);
    }

    /// 倒影高度由 `ReflSize` / `ReflGap` 决定。
    #[test]
    fn reflection_height_follows_config() {
        let mut c = cfg96();
        c.reflect = true;
        c.apply_i32(Key::REFL_SIZE, 50);
        c.apply_i32(Key::REFL_GAP, 10);
        let m = calc_metrics(&c, 96, 3, 0, work());
        // S * 50% + 10 + 2
        assert!((m.refl_h - (m.s * 0.5 + 12.0)).abs() < 0.01, "{m:?}");
    }

    #[test]
    fn reflection_only_on_bottom_edge() {
        let mut c = cfg96();
        c.reflect = true;
        let bottom = calc_metrics(&c, 96, 3, 0, work());
        assert!(bottom.refl_h > 0.0);
        c.edge = Edge::Top;
        let top = calc_metrics(&c, 96, 3, 0, work());
        assert_eq!(top.refl_h, 0.0, "倒影只在底部停靠时有效");
    }

    #[test]
    fn metric_scales_with_dpi() {
        let c = Config::defaults(144);
        let m = calc_metrics(&c, 144, 3, 0, Rect::new(0, 0, 2560, 1400));
        assert_eq!(m.s, 72.0);
    }

    #[test]
    fn edge_mapping_roundtrips() {
        let lh = 400.0;
        // `to_logical` 是**点**映射，而 `map_rect` 是矩形映射：顶部/左侧的矩形
        // 是翻转贴上去的，所以反算必须取翻转后的那条边，不是左上角。
        for edge in [Edge::Bottom, Edge::Top, Edge::Left, Edge::Right] {
            let (x, y, w, h) = map_rect(edge, 10.0, 20.0, 30.0, 40.0, lh);
            let (px, py) = match edge {
                Edge::Bottom => (x, y),
                Edge::Top => (x, y + h),
                Edge::Left => (x + w, y),
                Edge::Right => (x, y),
            };
            let (u, v) = to_logical(edge, px, py, lh);
            assert!((u - 10.0).abs() < 0.001, "{edge:?} u={u}");
            assert!((v - 20.0).abs() < 0.001, "{edge:?} v={v}");
            if edge == Edge::Left || edge == Edge::Right {
                assert_eq!((w, h), (40.0, 30.0), "竖向停靠要交换宽高");
            } else {
                assert_eq!((w, h), (30.0, 40.0));
            }
        }
    }

    #[test]
    fn magnification_curve_is_smooth_at_edges() {
        assert!((magnify_weight(0.0, 100.0) - 1.0).abs() < 1e-6);
        assert!((magnify_weight(100.0, 100.0)).abs() < 1e-6);
        assert_eq!(magnify_weight(200.0, 100.0), 0.0);
        // 中点 = 0.5
        let mid = magnify_weight(50.0, 100.0);
        assert!((mid - 0.5).abs() < 1e-6);
        // 单调递减
        let mut prev = f32::MAX;
        for i in 0..=10 {
            let w = magnify_weight(i as f32 * 5.0, 100.0);
            assert!(w <= prev + 1e-6);
            prev = w;
        }
    }

    /// **从左到右第一个**命中，不是"最近的那个"。
    ///
    /// 放大后相邻图标会重叠，两个都覆盖鼠标点时取左边那个。
    /// 取"最近"会差一个图标。
    /// 弹跳曲线：起止为 0、中途有峰、幅度单调衰减。
    ///
    /// 这是"点击启动后图标弹一下"的观感基础。把公式抄错（比如漏掉
    /// `|sin|` 就会变成半个正弦，图标只会晃一次）不会崩，只会让动效变怪。
    #[test]
    fn bounce_curve_starts_and_ends_at_zero() {
        let s = 52.0;
        assert_eq!(bounce_offset(0, s), 0.0, "起点必须为 0，否则会瞬移");
        assert_eq!(bounce_offset(BOUNCE_MS, s), 0.0, "结束必须为 0");
        assert_eq!(bounce_offset(BOUNCE_MS + 500, s), 0.0, "超出时长后恒为 0");
        assert!(!bounce_active(BOUNCE_MS));
    }

    /// 峰值出现在 `|sin|` 的极值处（x = 0.25 / 0.75），且第二次低于第一次。
    #[test]
    fn bounce_has_two_decaying_peaks() {
        let s = 52.0;
        let p1 = bounce_offset(BOUNCE_MS / 4, s);
        let p2 = bounce_offset(BOUNCE_MS * 3 / 4, s);
        assert!(p1 > 0.0 && p2 > 0.0);
        assert!(p1 > p2, "第二峰应当衰减：{p1} vs {p2}");
        // 峰值约为 S·0.48·衰减系数：第一次 ≈ 52*0.48*0.8625 ≈ 21.5。
        assert!((p1 - s * 0.48 * 0.8625).abs() < 0.5, "峰值 {p1} 与公式不符");
    }

    /// 偏移始终非负：曲线用来"往外弹"，方向由绘制端决定（减还是加）。
    #[test]
    fn bounce_offset_is_never_negative() {
        for t in (0..BOUNCE_MS).step_by(17) {
            assert!(bounce_offset(t, 52.0) >= 0.0, "t={t}");
        }
    }

    /// 四条边的定位规则。
    ///
    /// 底边是最常走的路径，但其余三条写错了不会崩、只会把 Dock 送到屏幕上
    /// 某个奇怪的位置，所以四条一起测。
    #[test]
    fn window_origin_places_the_dock_on_each_edge() {
        let work = Rect::new(0, 0, 1920, 1040);
        let (w, h) = (800, 100);

        // 无预留：贴工作区边。
        assert_eq!(
            window_origin(Edge::Bottom, work, None, w, h),
            (560, 940),
            "底边：水平居中 + 贴底"
        );
        assert_eq!(window_origin(Edge::Top, work, None, w, h), (560, 0));
        assert_eq!(window_origin(Edge::Left, work, None, w, h), (0, 470));
        assert_eq!(window_origin(Edge::Right, work, None, w, h), (1120, 470));

        // 正常预留：AppBar 是从 `rcWork` **内部**切走一块，两者共享同一条外边，
        // 所以窗口位置其实不变 —— 变的是**别的窗口**能不能用这块地方。
        // 这一点很容易误解成"预留后 Dock 会挪位置"，进而把两个坐标系搞混。
        let bottom_bar = Rect::new(0, 948, 1920, 1040);
        assert_eq!(
            window_origin(Edge::Bottom, work, Some(bottom_bar), w, h),
            (560, 940),
            "正常预留不改变窗口位置，只改变 rcWork"
        );
        let top_bar = Rect::new(0, 0, 1920, 92);
        assert_eq!(
            window_origin(Edge::Top, work, Some(top_bar), w, h),
            (560, 0)
        );
        let left_bar = Rect::new(0, 0, 92, 1040);
        assert_eq!(
            window_origin(Edge::Left, work, Some(left_bar), w, h),
            (0, 470)
        );
        let right_bar = Rect::new(1828, 0, 1920, 1040);
        assert_eq!(
            window_origin(Edge::Right, work, Some(right_bar), w, h),
            (1120, 470),
            "右侧预留：窗口右边贴预留区右边"
        );
    }

    /// 预留区被别的工具栏挤过时，窗口要跟着让，不能还按 `rcWork` 定位。
    ///
    /// `ABM_QUERYPOS` 会把请求的矩形调整到"实际能给"的位置，所以
    /// `ab.rc` 不一定贴着工作区外边。此时若仍用 `rcWork` 算坐标，
    /// Dock 会压到邻居的地盘上 —— 两个 AppBar 叠在一起。
    #[test]
    fn window_origin_prefers_the_reserved_rect() {
        let work = Rect::new(0, 0, 1920, 1040);
        // AppBar 实际只拿到一窄条（被别的工具栏挤过）。
        let squeezed = Rect::new(0, 1000, 1920, 1020);
        assert_eq!(
            window_origin(Edge::Bottom, work, Some(squeezed), 800, 100),
            (560, 920)
        );
    }

    /// 竖排时窗口尺寸要转置，否则 Dock 只有一条细缝。
    #[test]
    fn physical_size_transposes_on_vertical_edges() {
        assert_eq!(physical_size(Edge::Bottom, 1181, 186), (1181, 186));
        assert_eq!(physical_size(Edge::Top, 1181, 186), (1181, 186));
        assert_eq!(physical_size(Edge::Left, 1181, 186), (186, 1181));
        assert_eq!(physical_size(Edge::Right, 1181, 186), (186, 1181));
    }

    /// 预留厚度 = 栏高 + 留白 + 4。
    #[test]
    fn reserve_thickness_matches_upstream() {
        // 实测值：barH=76, margin=12 → 92
        assert_eq!(reserve_thickness(76.0, 12.0), 92);
        // 截断而不是四舍五入，与 (int) 强转一致。
        assert_eq!(reserve_thickness(76.9, 12.9), 93);
    }

    #[test]
    fn hit_test_takes_first_from_left() {
        let slot_l = [100.0, 160.0, 220.0];
        let slot_r = [130.0, 190.0, 250.0];
        let visible = [true, true, true];
        // gap=40 → 余量 20：145 同时落在图标 0（借了 gap/2）与图标 1 的范围内
        // → 取左边那个。
        assert_eq!(
            hit_index(145.0, 0.0, &slot_l, &slot_r, &visible, 40.0, 0.0, 88.0, 200.0),
            Some(0)
        );
        assert_eq!(
            hit_index(100.0, 0.0, &slot_l, &slot_r, &visible, 20.0, 0.0, 88.0, 200.0),
            Some(0)
        );
        // 195 落在图标 1 的区间 [150, 200) 里（右开区间）。
        assert_eq!(
            hit_index(195.0, 0.0, &slot_l, &slot_r, &visible, 20.0, 0.0, 88.0, 200.0),
            Some(1)
        );
        // 200 恰好是右边界 → 不命中（区间右开）。
        assert_eq!(
            hit_index(200.0, 0.0, &slot_l, &slot_r, &visible, 20.0, 0.0, 88.0, 200.0),
            None
        );
        // 左侧超出 gap/2 余量 → 不命中。
        assert_eq!(
            hit_index(50.0, 0.0, &slot_l, &slot_r, &visible, 20.0, 0.0, 88.0, 200.0),
            None
        );
    }

    /// 纵向范围检查：`v < bar_top - zoom - 16` 或 `v > lh` 都不命中。
    #[test]
    fn hit_test_respects_vertical_range() {
        let slot_l = [100.0];
        let slot_r = [130.0];
        let visible = [true];
        // bar_top=50, zoom=88 → 上界 50-88-16 = -54；v=0 在范围内。
        assert_eq!(
            hit_index(110.0, 0.0, &slot_l, &slot_r, &visible, 0.0, 50.0, 88.0, 200.0),
            Some(0)
        );
        // 超出上界。
        assert_eq!(
            hit_index(110.0, -60.0, &slot_l, &slot_r, &visible, 0.0, 50.0, 88.0, 200.0),
            None
        );
        // 超出下界。
        assert_eq!(
            hit_index(110.0, 250.0, &slot_l, &slot_r, &visible, 0.0, 50.0, 88.0, 200.0),
            None
        );
    }

    /// `cur <= 1`（正在消失）的图标不参与命中。
    #[test]
    fn hit_test_skips_invisible_slots() {
        let slot_l = [100.0, 160.0];
        let slot_r = [130.0, 190.0];
        let visible = [false, true];
        assert_eq!(
            hit_index(110.0, 0.0, &slot_l, &slot_r, &visible, 0.0, 0.0, 88.0, 200.0),
            None
        );
        assert_eq!(
            hit_index(170.0, 0.0, &slot_l, &slot_r, &visible, 0.0, 0.0, 88.0, 200.0),
            Some(1)
        );
    }

    /// `map_actual_to_base`：把放大后的实际位置映射回基准位置。
    #[test]
    fn actual_to_base_maps_interpolated() {
        let base_c = [100.0, 200.0];
        let act_c = [100.0, 240.0];
        // 实际区间的正中 → 基准区间的正中。
        assert!((map_actual_to_base(170.0, &base_c, &act_c) - 150.0).abs() < 0.01);
        // 端点之外等距平移。
        assert!((map_actual_to_base(50.0, &base_c, &act_c) - 50.0).abs() < 0.01);
        // 数量不一致时放弃映射。
        assert_eq!(map_actual_to_base(170.0, &[100.0], &act_c), 170.0);
    }

    // ---- 槽位布局 ----

    fn kinds(n: usize) -> Vec<SlotKind> {
        vec![SlotKind::Icon; n]
    }

    /// 基准宽度必须与 `calc_metrics` 里的 `wb` 是同一个数 —— 两者若漂移，
    /// 栏体会被画到窗口外（曾经在 GDI+ 版发生过）。
    #[test]
    fn base_width_matches_metrics_formula() {
        let c = cfg96();
        let m = calc_metrics(&c, 96, 3, 0, work());
        assert_eq!(
            base_width(&m, &kinds(3)),
            2.0 * m.pad_x + 3.0 * m.s + 2.0 * m.gap
        );
        // 空 Dock：4 格宽。
        assert_eq!(base_width(&m, &[]), 2.0 * m.pad_x + 4.0 * m.s);
    }

    /// 基准中心应等距、且整体居中（无时钟时）。
    #[test]
    fn base_centers_are_centered_and_evenly_spaced() {
        let c = cfg96();
        let m = calc_metrics(&c, 96, 3, 0, work());
        let centers = base_centers(&m, &kinds(3), 0.0);
        assert_eq!(centers.len(), 3);
        let step = m.s + m.gap;
        assert!((centers[1] - centers[0] - step).abs() < 0.01);
        assert!((centers[2] - centers[1] - step).abs() < 0.01);
        // 首尾对称于窗口中心。
        let mid = (centers[0] + centers[2]) / 2.0;
        assert!((mid - m.lw as f32 / 2.0).abs() < 0.01, "mid={mid}");
    }

    /// 小组件条带占位会把整条栏体左移半个占位宽。
    #[test]
    fn widget_extra_shifts_the_bar_left() {
        let c = cfg96();
        let m = calc_metrics(&c, 96, 3, 0, work());
        let plain = base_centers(&m, &kinds(3), 0.0);
        let with_clock = base_centers(&m, &kinds(3), 120.0);
        assert!((plain[0] - with_clock[0] - 60.0).abs() < 0.01);
    }

    /// 没有悬停时不放大，目标尺寸就是基准尺寸。
    #[test]
    fn magnify_targets_fall_back_to_base_size() {
        let c = cfg96();
        let m = calc_metrics(&c, 96, 3, 0, work());
        let bc = base_centers(&m, &kinds(3), 0.0);
        let t = magnify_targets(&m, &kinds(3), &bc, &bc, View::default(), true);
        assert!(t.iter().all(|v| (*v - m.s).abs() < 0.01));
    }

    /// 鼠标正对某个图标时它最大，越远越小，超出影响半径的保持基准尺寸。
    #[test]
    fn magnify_targets_peak_under_the_mouse() {
        let mut c = cfg96();
        c.zoom = true;
        // 9 个图标：半径 R = (S + g) * 2.6 ≈ 146，而相邻间距只有 S + g = 56，
        // 所以要 3 格以上（168 > 146）才能验证"半径外不放大"。
        let ks = kinds(9);
        let m = calc_metrics(&c, 96, 9, 0, work());
        assert!(m.z > m.s, "前提：放大尺寸大于基准");
        let bc = base_centers(&m, &ks, 0.0);
        let t = magnify_targets(
            &m,
            &ks,
            &bc,
            &bc,
            View {
                mouse: (bc[4], 0.0),
                hover: true,
                hide_cur: 0.0,
            },
            true,
        );

        assert!((t[4] - m.z).abs() < 0.01, "鼠标下的图标应到最大: {t:?}");
        // 相邻图标也在影响半径内（余弦衰减），但必须严格小于峰值。
        assert!(t[3] > m.s && t[3] < m.z, "相邻应部分放大: {t:?}");
        assert!(t[2] < t[3] && t[1] < t[2], "离得越远越小: {t:?}");
        // 距离 4 格 = 224 > R，完全不受影响。
        assert!((t[0] - m.s).abs() < 0.01, "半径外应保持基准: {t:?}");
        assert!((t[8] - m.s).abs() < 0.01);
    }

    /// 分割线的槽宽恒为 `sep_w`，不参与放大。
    #[test]
    fn separators_never_grow() {
        let mut c = cfg96();
        c.zoom = true;
        let m = calc_metrics(&c, 96, 3, 1, work());
        let ks = [SlotKind::Icon, SlotKind::Separator, SlotKind::Icon];
        let bc = base_centers(&m, &ks, 0.0);
        let t = magnify_targets(
            &m,
            &ks,
            &bc,
            &bc,
            View {
                mouse: (bc[1], 0.0),
                hover: true,
                hide_cur: 0.0,
            },
            true,
        );
        assert!((t[1] - m.sep_w).abs() < 0.01);
    }

    /// 隐藏过半后不再放大 —— 否则收起过程中图标会顶着屏幕边缘。
    #[test]
    fn hidden_dock_does_not_magnify() {
        let mut c = cfg96();
        c.zoom = true;
        let m = calc_metrics(&c, 96, 3, 0, work());
        let bc = base_centers(&m, &kinds(3), 0.0);
        let t = magnify_targets(
            &m,
            &kinds(3),
            &bc,
            &bc,
            View {
                mouse: (bc[1], 0.0),
                hover: true,
                hide_cur: 0.6,
            },
            true,
        );
        assert!(t.iter().all(|v| (*v - m.s).abs() < 0.01));
    }

    /// `place_slots` 的槽位必须首尾相接（间隙恰好是 `gap`）。
    #[test]
    fn place_slots_tile_without_overlap() {
        let c = cfg96();
        let m = calc_metrics(&c, 96, 3, 0, work());
        let cur = [m.s; 3];
        let s = place_slots(&m, &cur, 0.0);
        assert_eq!(s.slot_l.len(), 3);
        for i in 0..2 {
            assert!((s.slot_l[i + 1] - s.slot_r[i] - m.gap).abs() < 0.01);
        }
        assert!((s.slot_r[2] - s.slot_l[0] - s.bar_w + 2.0 * m.pad_x).abs() < 0.01);
    }

    /// 空 Dock 的栏宽仍是 4 格 + 两侧内边距。
    #[test]
    fn place_slots_keeps_empty_dock_wide_enough() {
        let c = cfg96();
        let m = calc_metrics(&c, 96, 0, 0, work());
        let s = place_slots(&m, &[], 0.0);
        assert!((s.bar_w - (2.0 * m.pad_x + 4.0 * m.s)).abs() < 0.01);
        assert!(s.slot_l.is_empty());
    }

    /// `lerp_to` 在阈值内直接吸附，且报告"动画结束"。
    #[test]
    fn lerp_snaps_when_close() {
        let mut v = 10.0;
        assert!(!lerp_to(&mut v, 10.2));
        assert_eq!(v, 10.2);

        let mut v = 10.0;
        assert!(lerp_to(&mut v, 20.0));
        assert!(v > 10.0 && v < 20.0, "必须只走一帧的距离: {v}");
    }

    /// `hit_test` 复用本帧槽位，并尊重"隐藏中不命中"。
    #[test]
    fn hit_test_uses_current_slots() {
        let c = cfg96();
        let m = calc_metrics(&c, 96, 3, 0, work());
        let bars = bars(&m);
        let cur = [m.s; 3];
        let s = place_slots(&m, &cur, 0.0);
        let center = s.act_c[1];
        let at = |hover, hide_cur| View {
            mouse: (center, bars.icon_bottom - 5.0),
            hover,
            hide_cur,
        };

        assert_eq!(hit_test(&m, &bars, &s, &cur, at(true, 0.0)), Some(1));
        assert_eq!(
            hit_test(&m, &bars, &s, &cur, at(true, 0.6)),
            None,
            "隐藏过半后不应命中"
        );
        assert_eq!(
            hit_test(&m, &bars, &s, &cur, at(false, 0.0)),
            None,
            "鼠标不在窗口内不应命中"
        );
    }

    /// 纵向基线：栏底到栏顶正好一个 `bar_h`。
    #[test]
    fn bars_are_consistent_with_metrics() {
        let c = cfg96();
        let m = calc_metrics(&c, 96, 3, 0, work());
        let b = bars(&m);
        assert!((b.bottom - b.top - m.bar_h).abs() < 0.01);
        assert!((b.bottom - b.icon_bottom - m.pad - 2.0).abs() < 0.01);
    }

    // ---- 分割线拖动缩放 ----

    /// 往上拖变大、往下拖变小，步长 0.6；放大尺寸按初始比例同比缩放。
    #[test]
    fn sep_scale_follows_drag_direction() {
        assert_eq!(sep_scale(0.0, 64, 96), (64, 96));
        assert_eq!(sep_scale(-100.0, 64, 96), (124, 186), "上拖 100px → +60");
        assert_eq!(
            sep_scale(100.0, 64, 96),
            (24, 36),
            "下拖 100px → -60，触底夹紧"
        );
    }

    /// 两个方向都夹得住，图标 24..128、放大 32..220。
    #[test]
    fn sep_scale_clamps_both_bounds() {
        assert_eq!(sep_scale(-1000.0, 64, 96), (128, 192));
        assert_eq!(sep_scale(1000.0, 64, 96), (24, 36));
        // 放大尺寸自己也要夹：起点图标很大、放大很小时，同比算出来会低于 32。
        assert_eq!(sep_scale(1000.0, 128, 32), (24, 32));
        assert_eq!(sep_scale(-1000.0, 24, 220), (128, 220));
    }

    /// 以拖动起点为基准：来回拖回原位，尺寸也回到原值（不累积取整误差）。
    #[test]
    fn sep_scale_roundtrip_returns_to_start() {
        let (icon, zoom) = sep_scale(-137.0, 64, 96);
        assert!(icon > 64 && zoom > 96);
        // 回到起点只需 dy=0 —— 基准始终是 icon0/zoom0。
        assert_eq!(sep_scale(0.0, 64, 96), (64, 96));
    }

    /// 底停靠：光标在整条带上都算"靠近"，包括图标条右边那一大段小组件。
    #[test]
    fn near_dock_bottom_covers_the_whole_strip() {
        // 图标条 400..1000，小组件接到 1700（热区是两者的并集）。
        let hot = (400.0, 1487.0, 1300.0, 88.0);
        let work = Rect::new(0, 0, 2560, 1600);
        let near = |x, y| near_dock(Edge::Bottom, (x, y), hot, work, 129.0, false);
        let keep = |x, y| near_dock(Edge::Bottom, (x, y), hot, work, 129.0, true);

        assert!(near(700, 1520), "图标上算靠近");
        assert!(near(1500, 1520), "时钟上也算靠近（早先这里判成离开）");
        assert!(near(1690, 1520), "带子右端仍在带内");
        assert!(near(400, 1560), "左边缘");
        // 朝屏幕里伸展 129 px：光标落在放大后的图标上仍算靠近。
        assert!(near(700, 1360), "伸展区里算靠近");
        assert!(!near(700, 1300), "伸展区外不算");
        // 贴边探头：沿停靠方向在跨度内才算。
        assert!(near(1500, 1599), "贴底边 + 在带子跨度内");
        assert!(!near(200, 1599), "贴底边但在带子左边，不算");
        assert!(!near(700, 1200), "离得远不算");
        // 回滞：展开态容差更大 —— 光标在 1358-32 那一圈里，收起态判"离开"、
        // 展开态仍判"在"，于是不会在边界上反复抽搐。
        assert!(keep(700, 1330), "展开时 1330 仍在保留区内");
        assert!(!near(700, 1330), "收起时同一位置算离开");
    }

    /// 上停靠：伸展方向朝下，贴边判定用工作区上沿。
    #[test]
    fn near_dock_top_reaches_downward() {
        let hot = (400.0, 0.0, 1300.0, 88.0);
        let work = Rect::new(0, 0, 2560, 1600);
        let near = |x, y| near_dock(Edge::Top, (x, y), hot, work, 129.0, false);

        assert!(near(700, 40), "栏体上");
        assert!(near(700, 200), "往下伸展的放大区");
        assert!(!near(700, 300), "再往下就离开了");
        // 收起态探头：贴住工作区上沿。
        assert!(near(700, 1), "贴上沿");
    }

    /// 左停靠：横向伸展，贴边用左沿；纵向跨度按热区高度算。
    #[test]
    fn near_dock_left_is_axis_swapped() {
        let hot = (0.0, 400.0, 88.0, 700.0);
        let work = Rect::new(0, 0, 2560, 1600);
        let near = |x, y| near_dock(Edge::Left, (x, y), hot, work, 129.0, false);

        assert!(near(40, 700), "栏体上");
        assert!(near(200, 700), "往右伸展的放大区");
        assert!(!near(300, 700), "太远");
        assert!(near(1, 700), "贴左边");
        assert!(!near(1, 200), "贴左边但不在跨度内");
    }
}
