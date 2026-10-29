//! 布局数学：度量、四边停靠的坐标映射、放大曲线、命中测试。
//!
//! 这一层是**纯函数**，不持有任何状态。原版把这些散在 `CalcMetrics` /
//! `MapR` / `ToLogical` / `HitIndex` 里，与全局 `cfg`、`winW/winH` 耦合，
//! 导致只能在真机上"看着像不像"来验证。拆出来后可以直接断言坐标。
//!
//! 所有魔数都标注了 C 版出处，改动前请先读 `PORTING.md` 的"像素级对齐"一节。

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

/// 一帧的度量结果。对应 C 版全局 `Metrics M` + `g_ce` / `g_clockW` / `g_reflH`。
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Metrics {
    /// 图标基准边长（`M.S`）。
    pub s: f32,
    /// 放大后的边长（`M.Z`）。
    pub z: f32,
    /// 图标间距（`M.g`）。
    pub gap: f32,
    /// 段首尾内边距 = `pad + 2`（`M.padX`）。
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
    /// 时钟卡片占位（横向 = 时钟宽度 + gap + 6；纵向另有上限）。
    pub clock_extra: f32,
    /// 时钟卡片宽度。
    pub clock_w: i32,
    /// 倒影高度（未开启时为 0）。
    pub refl_h: f32,
    /// 逻辑宽度（水平布局时即窗口宽）。
    pub lw: i32,
    /// 逻辑高度（水平布局时即窗口高）。
    pub lh: i32,
}

/// `f32` 夹紧。C 版 `Clampf`。
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

/// 计算一帧的度量。对应 C 版 `CalcMetrics()`（minidock.c:1511）。
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
    // 分割线宽度：上游 v1.4.0 起由 `SepGap` 决定
    // （旧版是写死的 `16 + spacing/2`）。
    let sep0 = cfg.get_i32(Key::SEP_GAP) as f32;
    let pad_x0 = pad0 + 2.0;

    let n_icon = n_items.saturating_sub(n_sep);
    let mut wb = 2.0 * pad_x0 + n_icon as f32 * s0 + n_sep as f32 * sep0;
    if n_items > 1 {
        wb += g0 * (n_items - 1) as f32;
    }
    // 空 Dock 也保持 4 格宽度，否则"拖到这里"的提示文字无处安放。
    if n_items == 0 {
        wb = 2.0 * pad_x0 + s0 * 4.0;
    }

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

    // 倒影高度：上游 v1.4.0 改成可调（`ReflSize` 百分比 + `ReflGap` 间距）。
    let refl_h = if cfg.reflect && cfg.edge == Edge::Bottom {
        s * (cfg.get_i32(Key::REFL_SIZE) as f32 / 100.0) + cfg.get_i32(Key::REFL_GAP) as f32 + 2.0
    } else {
        0.0
    };
    let margin = cfg.margin as f32 + refl_h;
    let bar_h = s + 2.0 * pad + 4.0;

    // 时钟卡片宽度。上游 v1.4.0 起按"显示什么 / 什么格式"算出来，
    // 不再是写死的两个 DPI 常量：
    //   显示内容 `ClockShow`：0 = 都显示，1 = 只日期，2 = 只时间
    //   日期格式 `DateFmt`：0 = 中文（月日），1 = 短格式
    //   显示周几 `ClockWeek` / 显示秒 `ClockSec` 都会改变宽度。
    let mut clock_w = 0;
    if cfg.clock {
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
            clock_w = wt.max(wd);
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
            clock_w = units::scale(lines * 21 + 14, dpi);
        }
    }
    let clock_extra = if cfg.clock {
        clock_w as f32 + gap + 6.0
    } else {
        0.0
    };

    // 栏体之上要留出的空间：放大溢出 + 半个图标（标签气泡）+ 固定余量。
    let mut overhang = (z - s - pad).max(0.0) + s * 0.55 + 34.0 + 8.0;
    if !cfg.edge.is_horizontal() {
        // 竖向停靠时气泡朝内，额外留 170 px 给长名称。
        overhang += 170.0;
    }

    let mut w = wb * fit + (z - s) * 6.0 + s + 260.0 + clock_extra;
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
        clock_extra,
        clock_w,
        refl_h,
        lw: w as i32,
        lh: (bar_h + overhang + margin) as i32 + 2,
    }
}

/// 逻辑坐标 → 窗口坐标。对应 C 版 `MapR()`（minidock.c:331）。
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

/// 窗口坐标 → 逻辑坐标。对应 C 版 `ToLogical()`（minidock.c:339）。
#[must_use]
pub fn to_logical(edge: Edge, px: f32, py: f32, lh: f32) -> (f32, f32) {
    match edge {
        Edge::Top => (px, lh - py),
        Edge::Left => (py, lh - px),
        Edge::Right => (py, px),
        Edge::Bottom => (px, py),
    }
}

/// 鼠标放大的权重曲线：中心 1.0，半径外 0.0，中间为余弦过渡。
///
/// 对应 C 版 `0.5 * (1 + cos(PI * d / R))`。用余弦而不是线性，
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
/// 原版 `HitIndex()` 用的是"最近的槽位中心 + 当前尺寸的一半"，这里保持
/// 同样的判定（用槽位中心距离，而不是矩形包含），因为放大动画期间
/// 图标的实际矩形与槽位并不重合 —— 用矩形判定会导致悬停抖动。
#[must_use]
#[allow(clippy::too_many_arguments)] // C 版就是这一串全局量，拆开反而更难对照
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
        // 放大后相邻图标会重叠，两个都覆盖鼠标点时 C 版选的是左边那个。
        if u >= slot_l[i] - half && u < slot_r[i] + half {
            return Some(i);
        }
    }
    None
}

/// C 版 `MapActualToBase()`：把鼠标的**实际**位置（此时槽位已被放大挪动）
/// 映射回**基准**位置（未放大时的槽位）。
///
/// 放大曲线必须用基准位置算权重：图标一放大，槽位就跟着移动，
/// 直接用实际位置会让"鼠标属于哪个图标"随放大程度漂移。
///
/// `base_c` / `act_c` 分别是基准中心与实际中心。
#[must_use]
pub fn map_actual_to_base(mx: f32, base_c: &[f32], act_c: &[f32]) -> f32 {
    // 数量对不上（条目变了）就放弃映射 —— 对应 C 版 `prevN != nItems` 的保护。
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

    #[test]
    fn empty_dock_keeps_four_slots_wide() {
        let m = calc_metrics(&cfg96(), 96, 0, 0, work());
        // 2*10 + 48*4 = 212，再加固定 260 的交互余量。
        assert_eq!(m.lw, 212 + 260);
        assert!(m.bar_h > m.s, "栏高必须大于图标");
    }

    #[test]
    fn bar_height_matches_c_formula() {
        let c = cfg96();
        let m = calc_metrics(&c, 96, 3, 0, work());
        assert_eq!(m.bar_h, m.s + 2.0 * c.pad as f32 + 4.0);
        assert_eq!(m.pad_x, c.pad as f32 + 2.0);
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
        assert!(with.clock_extra > 0.0);
        // 上游 v1.4.0：默认同时显示时间与日期、带周几、中文日期格式
        // → 宽度取日期那一项（Sc(166)）。
        assert_eq!(with.clock_w, 166);
        assert!(with.lw > without.lw);
    }

    /// 时钟卡片宽度按"显示什么 / 什么格式"变化（上游 v1.4.0 的新算法）。
    #[test]
    fn clock_width_follows_upstream_rules() {
        let mut c = cfg96();
        c.clock = true;

        // 上游选项：0 = 时间+日期，1 = 仅时间，2 = 仅日期。

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

    /// 分割线宽度改由 `SepGap` 决定（上游 v1.4.0）。
    #[test]
    fn separator_width_follows_sep_gap() {
        let mut c = cfg96();
        let before = calc_metrics(&c, 96, 4, 1, work());
        c.apply_i32(Key::SEP_GAP, 20);
        let after = calc_metrics(&c, 96, 4, 1, work());
        assert!(after.sep_w > before.sep_w);
        assert_eq!(after.sep_w, 20.0);
    }

    /// 倒影高度由 `ReflSize` / `ReflGap` 决定（上游 v1.4.0）。
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

    /// C 版 `HitIndex()` 的语义：**从左到右第一个**命中，不是"最近的那个"。
    ///
    /// 放大后相邻图标会重叠，两个都覆盖鼠标点时 C 版选左边那个。
    /// 我们早期实现取最近，与 C 版不一致，命中会差一个图标。
    #[test]
    fn hit_test_takes_first_from_left() {
        let slot_l = [100.0, 160.0, 220.0];
        let slot_r = [130.0, 190.0, 250.0];
        let visible = [true, true, true];
        // gap=40 → 余量 20：145 同时落在图标 0（借了 gap/2）与图标 1 的范围内
        // → 取左边那个（C 版语义）。
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
}
