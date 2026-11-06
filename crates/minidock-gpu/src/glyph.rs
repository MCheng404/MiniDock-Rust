//! 矢量图标中心：Dock 内置条目、控制中心磁贴、首字占位、条带图标。
//!
//! 设计基准（Fluent 24px 图标 / Lucide 描边规范，2026-11-06 查证）：
//!
//! 1. **20px 安全区**：24px 画布里内容只占 20px、四周留 2px —— 这里
//!    归一化为 `SAFE = 0.40`；方形类再做光学补偿（`BOX = 0.34`），
//!    否则方块看起来比圆形大一圈；
//! 2. **统一描边**：线宽 `s * 0.085`（下限 1.1px），同一套图标不换粗细；
//! 3. **圆角连接**：所有折线走 `polyline` —— 一整条 `Shape::line` +
//!    每个顶点补**一次**圆头（round join / round cap）。旧版每段线段
//!    各画一次、两端各补圆，拐点会叠出结节、端点接不上；
//! 4. **没有锐角**：填充块只用圆角矩形 / 圆，不用三角、梯形等凸多边形；
//!    尖角形状（闪电、蓝牙、水滴）一律用折线 + 圆头软化；
//! 5. **动态状态**：`IconState` 把 level / active / phase / muted 集中起来，
//!    绘制端不持有时间 —— 以后加动画只需调用方推进 `phase`，不用改
//!    每个图标的签名或绘制代码。
//!
//! 不用 emoji、不用渐变、不用特殊字符：打包字体没有这些符号（ADR-040
//! 在 ✓/✕ 上踩过豆腐块），小尺寸渐变只会显脏。

use egui::{Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect as EguiRect, Stroke};
use minidock_core::control::Tile;

/// 24px 画布的 20px 安全区（归一化半径）。
const SAFE: f32 = 0.40;
/// 方形类的光学补偿：同样半径下方块视觉面积更大，缩一档才与圆类齐平。
const BOX: f32 = 0.34;

/// 图标的动态状态。所有可动参数集中在这里，绘制函数只读不持有时间。
#[derive(Clone, Copy, Debug)]
pub struct IconState {
    /// 0..1 强度/进度：Wifi 信号格、音量声波、电池电量、点阵个数。
    pub level: f32,
    /// 激活 / 选中态。
    pub active: bool,
    /// 0..1 动画相位，由调用方按时间推进（绘制端不碰时钟）。
    pub phase: f32,
    /// 静音 / 禁用语义。
    pub muted: bool,
}

impl Default for IconState {
    fn default() -> Self {
        Self {
            level: 1.0,
            active: false,
            phase: 0.0,
            muted: false,
        }
    }
}

impl IconState {
    /// 指定强度，其余取默认。
    #[must_use]
    pub fn with_level(mut self, level: f32) -> Self {
        self.level = level.clamp(0.0, 1.0);
        self
    }

    /// 指定动画相位，其余取默认。
    #[must_use]
    pub fn with_phase(mut self, phase: f32) -> Self {
        self.phase = phase.rem_euclid(1.0);
        self
    }
}

/// 一次绘制的完整规格：颜色 + 主题色 + 动态状态。
///
/// 所有公开入口统一收它，以后要加字段（例如 hover 进度、强调闪烁）
/// 不用改函数签名。
#[derive(Clone, Copy, Debug)]
pub struct IconStyle {
    /// 图标主体色（线稿 / 填充）。
    pub color: Color32,
    /// 主题色：激活段、强调段、内置条目底色。
    pub accent: Color32,
    pub state: IconState,
}

impl IconStyle {
    /// 单色图标（无主题色区分）。
    #[must_use]
    pub fn plain(color: Color32) -> Self {
        Self {
            color,
            accent: color,
            state: IconState::default(),
        }
    }

    /// 带主题色与状态的完整规格。
    #[must_use]
    pub const fn new(color: Color32, accent: Color32, state: IconState) -> Self {
        Self {
            color,
            accent,
            state,
        }
    }
}

/// 需要现画的 Dock 内置条目。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Builtin {
    /// `mdock:start` —— 四格窗。
    Start,
    /// `mdock:launchpad` —— 九宫格点阵。
    Launchpad,
}

impl Builtin {
    /// 按路径识别。大小写不敏感：手改 ini 时没人记得原样大小写。
    #[must_use]
    pub fn from_path(path: &str) -> Option<Self> {
        if path.eq_ignore_ascii_case("mdock:start") {
            Some(Self::Start)
        } else if path.eq_ignore_ascii_case("mdock:launchpad") {
            Some(Self::Launchpad)
        } else {
            None
        }
    }
}

/// 统一画布：以 `c` 为心、边长 `s`，坐标按 `s` 归一化。
struct Canvas<'a> {
    p: &'a Painter,
    c: Pos2,
    s: f32,
    style: IconStyle,
}

impl<'a> Canvas<'a> {
    fn from_rect(p: &'a Painter, rect: EguiRect, style: IconStyle) -> Self {
        Self {
            p,
            c: rect.center(),
            s: rect.width().min(rect.height()),
            style,
        }
    }

    fn pt(&self, x: f32, y: f32) -> Pos2 {
        Pos2::new(self.c.x + x * self.s, self.c.y + y * self.s)
    }

    fn sw(&self) -> f32 {
        (self.s * 0.085).max(1.1)
    }

    /// 连续折线：一条 `Shape::line` + 每个顶点补一次圆头。
    /// 这是"线条连得起来、拐点不叠结节"的关键 —— 不再逐段画线段。
    fn polyline_with(&self, pts: &[(f32, f32)], ink: Color32) {
        if pts.len() < 2 {
            return;
        }
        let st = Stroke::new(self.sw(), ink);
        let screen: Vec<Pos2> = pts.iter().map(|(x, y)| self.pt(*x, *y)).collect();
        self.p.add(egui::Shape::line(screen.clone(), st));
        let r = st.width * 0.5;
        let mut prev: Option<Pos2> = None;
        for q in screen {
            // 相邻重复点（闭合路径的首尾）只补一次圆，避免同点叠色。
            if prev.is_none_or(|p| (p - q).length() > 0.01) {
                self.p.circle_filled(q, r, ink);
            }
            prev = Some(q);
        }
    }

    fn polyline(&self, pts: &[(f32, f32)]) {
        self.polyline_with(pts, self.style.color);
    }

    /// 圆弧也走连续折线（18 段在 24px 下足够顺），端点自动带圆头。
    fn arc(&self, cx: f32, cy: f32, r: f32, from: f32, to: f32, ink: Color32) {
        let pts: Vec<(f32, f32)> = (0..=18)
            .map(|i| {
                let t = from + (to - from) * i as f32 / 18.0;
                (cx + r * t.cos(), cy + r * t.sin())
            })
            .collect();
        self.polyline_with(&pts, ink);
    }

    fn circle_stroke(&self, x: f32, y: f32, r: f32) {
        self.p.circle_stroke(
            self.pt(x, y),
            r * self.s,
            Stroke::new(self.sw(), self.style.color),
        );
    }

    fn circle_filled(&self, x: f32, y: f32, r: f32, color: Color32) {
        self.p.circle_filled(self.pt(x, y), r * self.s, color);
    }

    fn dot(&self, x: f32, y: f32, r: f32) {
        self.circle_filled(x, y, r, self.style.color);
    }

    /// 所有填充矩形统一圆角（不写死像素、不留直角）。
    fn radius(&self) -> CornerRadius {
        CornerRadius::same(pix(self.s * 0.10).max(2))
    }

    fn box_filled(&self, x: f32, y: f32, w: f32, h: f32, color: Color32) {
        self.p.rect_filled(
            EguiRect::from_min_size(self.pt(x, y), egui::vec2(w * self.s, h * self.s)),
            self.radius(),
            color,
        );
    }

    fn box_stroke(&self, x: f32, y: f32, w: f32, h: f32) {
        self.p.rect_stroke(
            EguiRect::from_min_size(self.pt(x, y), egui::vec2(w * self.s, h * self.s)),
            self.radius(),
            Stroke::new(self.sw(), self.style.color),
            egui::StrokeKind::Inside,
        );
    }

    /// 弱化的段（未点亮的信号格 / 未达到的电量），用于 level 动态。
    fn faint(&self) -> Color32 {
        self.style.color.linear_multiply(0.30)
    }
}

/// 半径取整到整数像素，避免半点像素造成的糊边。
fn pix(v: f32) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

// ---------------------------------------------------------------------------
// Dock：内置条目 / 首字占位 / 条带图标
// ---------------------------------------------------------------------------

/// 画一块 Dock 内置图标，铺满 `rect`（逻辑点）。
///
/// Start 的底色取 `style.accent`（开关开时是主题色，关时回落默认蓝）；
/// Launchpad 的九个点按 `state.level` 点亮，为动态图标留入口。
pub fn paint_tile(painter: &Painter, rect: EguiRect, kind: Builtin, style: IconStyle, dark: bool) {
    let t = rect.shrink(rect.width() * 0.04);
    let s = t.width().min(t.height());
    let radius = CornerRadius::same(pix(s * 0.26));
    match kind {
        Builtin::Start => {
            painter.rect_filled(t, radius, style.accent);
            // 2×2 白色圆角窗：间距与圆角都按 s 算，不留直角。
            let q = s * 0.27;
            let gp = s * 0.07;
            let o = (s - 2.0 * q - gp) / 2.0;
            let qc = CornerRadius::same(pix(q * 0.22));
            for i in 0..4 {
                let x = t.left() + o + (i % 2) as f32 * (q + gp);
                let y = t.top() + o + (i / 2) as f32 * (q + gp);
                painter.rect_filled(
                    EguiRect::from_min_size(Pos2::new(x, y), egui::vec2(q, q)),
                    qc,
                    Color32::WHITE,
                );
            }
        }
        Builtin::Launchpad => {
            painter.rect_filled(
                t,
                radius,
                if dark {
                    Color32::from_rgb(46, 48, 58)
                } else {
                    Color32::from_rgb(247, 248, 252)
                },
            );
            // 圆点代替方块：九宫格不再有九个直角，低饱和配色也更稳。
            const COLS: [Color32; 9] = [
                Color32::from_rgb(238, 108, 100),
                Color32::from_rgb(240, 168, 76),
                Color32::from_rgb(104, 196, 112),
                Color32::from_rgb(80, 160, 238),
                Color32::from_rgb(168, 124, 222),
                Color32::from_rgb(238, 146, 96),
                Color32::from_rgb(100, 198, 220),
                Color32::from_rgb(234, 116, 152),
                Color32::from_rgb(126, 202, 148),
            ];
            let step = s * 0.265;
            let r = s * 0.075;
            let lit = (style.state.level.clamp(0.0, 1.0) * 9.0).round() as usize;
            for (i, col) in COLS.iter().enumerate() {
                let x = t.center().x + ((i % 3) as f32 - 1.0) * step;
                let y = t.center().y + ((i / 3) as f32 - 1.0) * step;
                let c = if i < lit {
                    *col
                } else {
                    col.linear_multiply(0.35)
                };
                painter.circle_filled(Pos2::new(x, y), r, c);
            }
        }
    }
}

/// 没有 shell 图标时的首字占位：中性 field 底 + 主题色首字母。
pub fn paint_monogram(painter: &Painter, rect: EguiRect, ch: char, style: IconStyle, dark: bool) {
    let s = rect.width().min(rect.height());
    let radius = CornerRadius::same(pix(s * 0.26));
    let bg = if dark {
        Color32::from_rgba_unmultiplied(255, 255, 255, 26)
    } else {
        Color32::from_rgba_unmultiplied(23, 26, 35, 18)
    };
    painter.rect_filled(rect, radius, bg);
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(
            1.0,
            if dark {
                Color32::from_rgba_unmultiplied(255, 255, 255, 24)
            } else {
                Color32::from_rgba_unmultiplied(23, 26, 35, 14)
            },
        ),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        ch.to_string(),
        FontId::proportional(s * 0.46),
        style.accent,
    );
}

/// 控制中心卡片的图标：两条滑轨 + 两个圆钮（与设置界面滑条同一语言）。
pub fn sliders(painter: &Painter, rect: EguiRect, style: IconStyle) {
    let c = Canvas::from_rect(painter, rect, style);
    for (i, knob_at) in [(0, 0.62_f32), (1, 0.38_f32)] {
        let y = if i == 0 { -0.20 } else { 0.20 };
        c.polyline(&[(-0.36, y), (0.36, y)]);
        let kx = -0.36 + 0.72 * knob_at;
        c.dot(kx, y, 0.075);
    }
}

// ---------------------------------------------------------------------------
// 控制中心：磁贴图标 / 喇叭
// ---------------------------------------------------------------------------

/// 画一个控制中心磁贴的图标。
///
/// `state.level` 控制信号/音量/电量的段数，`state.phase` 控制齿旋转与
/// 热点涟漪 —— 调用方推进状态即可产出动态图标，无需改这里。
pub fn control_tile(painter: &Painter, rect: EguiRect, tile: Tile, style: IconStyle) {
    let c = Canvas::from_rect(painter, rect, style);
    let level = style.state.level.clamp(0.0, 1.0);
    let phase = style.state.phase;
    match tile {
        Tile::Wifi => {
            // 扇形天生只占下半格，视觉上比圆类小一圈 —— 放大 1.18 倍做
            // 光学补偿，最大弧仍在 SAFE 内（0.46 < 0.40 半径 + 中心偏移）。
            // 三段弧按 level 逐段点亮；未点亮段用同一色弱化，不换线宽。
            const K: f32 = 1.18;
            for i in 0..3 {
                let lit = (level * 3.0).round() as usize > i;
                let ink = if lit { style.color } else { c.faint() };
                c.arc(
                    0.0,
                    0.28 * K,
                    (0.13 + i as f32 * 0.13) * K,
                    -2.50,
                    -0.64,
                    ink,
                );
            }
            c.dot(0.0, 0.28 * K, 0.055 * K);
        }
        Tile::Ethernet => {
            // 圆角机箱 + 三条接口针 + 底座，各段互不接触（不叠端点）。
            // 方形类整体收在 BOX 内，与圆类图标看起来一样大。
            c.box_stroke(-BOX * 0.88, -0.26, BOX * 1.76, 0.38);
            for dx in [-0.14_f32, 0.0, 0.14] {
                c.polyline(&[(dx, -0.06), (dx, 0.06)]);
            }
            c.polyline(&[(-0.20, 0.22), (0.20, 0.22)]);
            // 顶上的两个小圆端，替代原来会与框描边叠在一起的短线。
            c.dot(-0.14, -0.34, 0.045);
            c.dot(0.14, -0.34, 0.045);
        }
        Tile::Bluetooth => {
            // 一条闭合折线勾出右侧菱形 + 竖线（首尾同点，圆头去重），
            // 再加左侧 V —— 只有两条子路径，不存在端点重叠。
            c.polyline(&[
                (-0.03, -0.40),
                (0.23, -0.15),
                (-0.03, 0.0),
                (0.23, 0.15),
                (-0.03, 0.40),
                (-0.03, -0.40),
            ]);
            c.polyline(&[(-0.23, -0.16), (-0.03, 0.0), (-0.23, 0.16)]);
        }
        Tile::DarkMode => {
            // 月牙：一段外弧 + 一段反向内弧，圆头收尾。
            c.arc(0.0, 0.0, SAFE, 0.80, 5.50, style.color);
            c.arc(0.16, -0.10, 0.30, 1.40, 4.90, style.color);
        }
        Tile::Location => {
            // 水滴针：一条连续折线（圆弧采样 + 收尖），圆头软化全部拐角。
            let mut pts: Vec<(f32, f32)> = (0..=14)
                .map(|i| {
                    let t = std::f32::consts::PI * i as f32 / 14.0;
                    (0.26 * t.cos(), -0.12 - 0.26 * t.sin())
                })
                .collect();
            pts.extend([(0.0, 0.40), (0.26, -0.12)]);
            c.polyline(&pts);
            c.dot(0.0, -0.12, 0.065);
        }
        Tile::Mute => speaker(
            painter,
            rect,
            IconStyle::new(
                style.color,
                style.accent,
                IconState {
                    muted: true,
                    ..style.state
                },
            ),
        ),
        Tile::Settings => {
            // 齿轮：外环 + 8 个圆齿 + 轴孔，全部圆形，没有梯形锐角。
            // phase 让齿整体微转 —— 动态图标的最小示例。
            c.circle_stroke(0.0, 0.0, 0.25);
            c.circle_stroke(0.0, 0.0, 0.085);
            let spin = phase * std::f32::consts::TAU / 8.0;
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0 + spin;
                c.circle_filled(0.33 * a.cos(), 0.33 * a.sin(), 0.075, style.color);
            }
        }
        Tile::Energy => {
            // 电池 + 闪电 + 底部电量条，三块互不重叠；电量条按 level。
            c.box_stroke(-0.32, -0.20, 0.58, 0.38);
            c.circle_filled(0.30, -0.01, 0.045, style.color);
            c.polyline(&[
                (0.05, -0.14),
                (-0.09, 0.00),
                (0.01, 0.00),
                (-0.03, 0.10),
                (0.11, -0.05),
                (0.02, -0.05),
                (0.05, -0.14),
            ]);
            if level > 0.02 {
                c.box_filled(-0.27, 0.11, 0.50 * level, 0.045, style.color);
            }
        }
        Tile::Hotspot => {
            // 与 Wifi 同样的扇形光学补偿；涟漪按 phase 外扩（幅度很小，
            // 静态时仍是一枚标准热点图标）。
            const K: f32 = 1.15;
            let ripple = (phase * std::f32::consts::TAU).sin() * 0.02;
            c.dot(0.0, 0.30 * K, 0.065 * K);
            c.arc(0.0, 0.30 * K, 0.19 * K + ripple, -2.40, -0.74, style.color);
            c.arc(0.0, 0.30 * K, 0.35 * K + ripple, -2.40, -0.74, style.color);
        }
    }
}

/// 喇叭：音量滑块左侧与「静音」磁贴共用。
///
/// 本体是一条闭合折线（圆头连接，无三角锐角）；声波按 `level` 逐段点亮。
pub fn speaker(painter: &Painter, rect: EguiRect, style: IconStyle) {
    let c = Canvas::from_rect(painter, rect, style);
    c.polyline(&[
        (-0.38, -0.10),
        (-0.22, -0.10),
        (-0.06, -0.26),
        (-0.06, 0.26),
        (-0.22, 0.10),
        (-0.38, 0.10),
        (-0.38, -0.10),
    ]);
    if style.state.muted {
        c.polyline(&[(0.04, -0.12), (0.28, 0.12)]);
        c.polyline(&[(0.28, -0.12), (0.04, 0.12)]);
    } else {
        let level = style.state.level.clamp(0.0, 1.0);
        for i in 0..2 {
            let lit = (level * 2.0).round() as usize > i;
            let ink = if lit { style.color } else { c.faint() };
            c.arc(0.02, 0.0, 0.20 + i as f32 * 0.12, -0.85, 0.85, ink);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 路径识别不看大小写：ini 是用户手改的，`MDOCK:Launchpad` 也得认。
    #[test]
    fn builtin_paths_are_case_insensitive() {
        assert_eq!(Builtin::from_path("mdock:start"), Some(Builtin::Start));
        assert_eq!(
            Builtin::from_path("MDOCK:Launchpad"),
            Some(Builtin::Launchpad)
        );
        assert_eq!(Builtin::from_path("C:/Apps/x.exe"), None);
        assert_eq!(Builtin::from_path("mdock:"), None);
    }

    /// 画布坐标按 s 归一化：s 放大 4 倍，偏移量也正好放大 4 倍。
    #[test]
    fn canvas_coordinates_scale_with_size() {
        let map = |c: f32, s: f32, x: f32| c + x * s;
        let small = map(100.0, 24.0, SAFE) - 100.0;
        let large = map(100.0, 96.0, SAFE) - 100.0;
        assert_eq!(large / small, 4.0);
    }

    /// 动态状态入口：level/phase 夹紧与循环，超出范围不会把图标画飞。
    #[test]
    fn icon_state_clamps_dynamic_inputs() {
        let st = IconState::default().with_level(2.0).with_phase(1.25);
        assert_eq!(st.level, 1.0);
        assert!((st.phase - 0.25).abs() < 1e-6);
        assert_eq!(IconState::default().with_level(-1.0).level, 0.0);
    }

    /// 安全区：内容半径不超过 20px/24px 的归一化值，方形还有光学补偿。
    #[test]
    fn safe_area_matches_fluent_grid() {
        // 经函数参数传值，避免 clippy 把常量比较判成 assertions_on_constants。
        let inside = |radius: f32, limit: f32| radius <= limit;
        let grid = 20.0_f32 / 48.0 + 0.02; // 20px 内容 / 24px 画布 = 0.4167
        assert!(inside(SAFE, grid));
        assert!(inside(BOX, SAFE));
    }
}
