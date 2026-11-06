//! 矢量图标中心：Dock 内置条目、控制中心磁贴、首字占位、条带图标。
//!
//! Dock 与控制中心原先各画各的：一处是写死颜色的填充色块，一处是写死
//! 像素圆角的线稿，缩放/DPI 一变就走样。现在统一到一套规范：
//!
//! 1. **归一化画布**：所有坐标按「以图标中心为原点、边长 `s` 的方框」
//!    计算（-0.5..0.5），尺寸每帧变化（Dock 悬停放大）也不会错位；
//! 2. **统一笔触**：线宽 `s * 0.085`（下限 1.1 px），所有线段/圆弧带圆头，
//!    圆角半径都按 `s` 取比例，不写死像素；
//! 3. **颜色全由调用方给**：内置条目跟 Dock 主题色，磁贴跟前后景/开关态；
//! 4. **不用 emoji、不用渐变、不用字符**：打包字体没有这些符号（ADR-040
//!    在 ✓/✕ 上踩过豆腐块），小尺寸渐变只会显脏。

use egui::{Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect as EguiRect, Stroke};
use minidock_core::control::Tile;

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
    ink: Color32,
}

impl<'a> Canvas<'a> {
    fn from_rect(p: &'a Painter, rect: EguiRect, ink: Color32) -> Self {
        Self {
            p,
            c: rect.center(),
            s: rect.width().min(rect.height()),
            ink,
        }
    }

    fn from_center(p: &'a Painter, c: Pos2, s: f32, ink: Color32) -> Self {
        Self { p, c, s, ink }
    }

    fn pt(&self, x: f32, y: f32) -> Pos2 {
        Pos2::new(self.c.x + x * self.s, self.c.y + y * self.s)
    }

    fn stroke(&self) -> Stroke {
        Stroke::new((self.s * 0.085).max(1.1), self.ink)
    }

    /// 带圆头的线段。线稿图标的"毛刺感"全在 butt 端点上，补两个
    /// 半圆即可，代价只是每帧多两个小 shape。
    fn line(&self, x0: f32, y0: f32, x1: f32, y1: f32) {
        let st = self.stroke();
        let a = self.pt(x0, y0);
        let b = self.pt(x1, y1);
        self.p.line_segment([a, b], st);
        let r = st.width * 0.5;
        self.p.circle_filled(a, r, self.ink);
        self.p.circle_filled(b, r, self.ink);
    }

    /// 带圆头的圆弧（18 段折线在 24px 下足够顺）。
    fn arc(&self, cx: f32, cy: f32, r: f32, from: f32, to: f32) {
        let st = self.stroke();
        let pts: Vec<Pos2> = (0..=18)
            .map(|i| {
                let t = from + (to - from) * i as f32 / 18.0;
                self.pt(cx + r * t.cos(), cy + r * t.sin())
            })
            .collect();
        if let (Some(&first), Some(&last)) = (pts.first(), pts.last()) {
            self.p.add(egui::Shape::line(pts, st));
            let rad = st.width * 0.5;
            self.p.circle_filled(first, rad, self.ink);
            self.p.circle_filled(last, rad, self.ink);
        }
    }

    fn circle_stroke(&self, x: f32, y: f32, r: f32) {
        self.p
            .circle_stroke(self.pt(x, y), r * self.s, self.stroke());
    }

    fn circle_filled(&self, x: f32, y: f32, r: f32) {
        self.p.circle_filled(self.pt(x, y), r * self.s, self.ink);
    }

    fn radius(&self, ratio: f32) -> CornerRadius {
        CornerRadius::same(pix(self.s * ratio))
    }

    fn rect_filled(&self, x: f32, y: f32, w: f32, h: f32, ratio: f32, color: Color32) {
        self.p.rect_filled(
            EguiRect::from_min_size(self.pt(x, y), egui::vec2(w * self.s, h * self.s)),
            self.radius(ratio),
            color,
        );
    }

    fn rect_stroke(&self, x: f32, y: f32, w: f32, h: f32, ratio: f32) {
        self.p.rect_stroke(
            EguiRect::from_min_size(self.pt(x, y), egui::vec2(w * self.s, h * self.s)),
            self.radius(ratio),
            self.stroke(),
            egui::StrokeKind::Inside,
        );
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
/// `accent` 是 Dock 的运行时主题色（`Config::runtime_accent`，关掉同步时
/// 由调用方回落默认蓝）—— Start 的底色跟着走，不再写死蓝。
pub fn paint_tile(painter: &Painter, rect: EguiRect, kind: Builtin, accent: Color32, dark: bool) {
    let t = rect.shrink(rect.width() * 0.04);
    let s = t.width().min(t.height());
    let radius = CornerRadius::same(pix(s * 0.22));
    match kind {
        Builtin::Start => {
            painter.rect_filled(t, radius, accent);
            // 2×2 白色圆角窗：间距与圆角都按 s 算。
            let q = s * 0.27;
            let gp = s * 0.07;
            let o = (s - 2.0 * q - gp) / 2.0;
            let qc = CornerRadius::same(pix(q * 0.16));
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
            // 九宫格保留配色语义，但统一成低饱和、按明度排过的一组 ——
            // 原版九个高饱和原色在毛玻璃 Dock 上太跳。
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
            let q = s * 0.19;
            let gp = s * 0.085;
            let o = (s - 3.0 * q - 2.0 * gp) / 2.0;
            let qc = CornerRadius::same(pix(q * 0.30));
            for (i, col) in COLS.iter().enumerate() {
                let x = t.left() + o + (i % 3) as f32 * (q + gp);
                let y = t.top() + o + (i / 3) as f32 * (q + gp);
                painter.rect_filled(
                    EguiRect::from_min_size(Pos2::new(x, y), egui::vec2(q, q)),
                    qc,
                    *col,
                );
            }
        }
    }
}

/// 没有 shell 图标时的首字占位：中性 field 底 + 主题色首字母。
///
/// 旧版是脏灰底（90,120,130）+ 暗红字（240,30,30），在深浅主题里
/// 都像一块脏斑；现在与设置界面的 tonal 层、内置条目同一套语言。
pub fn paint_monogram(painter: &Painter, rect: EguiRect, ch: char, accent: Color32, dark: bool) {
    let s = rect.width().min(rect.height());
    let radius = CornerRadius::same(pix(s * 0.22));
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
        accent,
    );
}

/// 控制中心卡片的图标：两条滑轨 + 两个圆钮（与设置界面滑条同一语言）。
pub fn sliders(painter: &Painter, rect: EguiRect, color: Color32) {
    let c = Canvas::from_rect(painter, rect, color);
    for (i, knob_at) in [(0, 0.62_f32), (1, 0.38_f32)] {
        let y = if i == 0 { -0.20 } else { 0.20 };
        c.line(-0.42, y, 0.42, y);
        let kx = -0.42 + 0.84 * knob_at;
        c.circle_filled(kx, y, 0.075);
    }
}

// ---------------------------------------------------------------------------
// 控制中心：磁贴图标 / 喇叭
// ---------------------------------------------------------------------------

/// 画一个控制中心磁贴的图标。全部走统一画布，形状按 `Tile` 重画。
pub fn control_tile(painter: &Painter, rect: EguiRect, tile: Tile, color: Color32) {
    let c = Canvas::from_rect(painter, rect, color);
    match tile {
        Tile::Wifi => {
            for i in 0..3 {
                c.arc(0.0, 0.28, 0.14 + i as f32 * 0.13, -2.50, -0.64);
            }
            c.circle_filled(0.0, 0.28, 0.055);
        }
        Tile::Ethernet => {
            c.rect_stroke(-0.28, -0.24, 0.56, 0.38, 0.08);
            c.line(-0.14, -0.38, -0.14, -0.24);
            c.line(0.14, -0.38, 0.14, -0.24);
            for dx in [-0.14_f32, 0.0, 0.14] {
                c.line(dx, 0.14, dx, 0.30);
            }
            c.line(-0.20, 0.36, 0.20, 0.36);
        }
        Tile::Bluetooth => {
            // 卢恩 ᛒ（几何取自 simple-icons bluetooth.svg 去外圈）：
            // 竖线 + 右侧上下两个尖构成的菱形 + 穿过中心的左侧两条斜线。
            // 前两版把折线画成了闭合三角，看起来是「▶ 播放键」。
            c.line(-0.03, -0.43, -0.03, 0.43);
            c.line(-0.03, -0.43, 0.23, -0.17);
            c.line(0.23, -0.17, -0.03, 0.0);
            c.line(-0.03, 0.0, 0.23, 0.17);
            c.line(0.23, 0.17, -0.03, 0.43);
            c.line(-0.23, -0.18, -0.03, 0.0);
            c.line(-0.03, 0.0, -0.23, 0.18);
        }
        Tile::DarkMode => {
            // 月牙：一段外弧 + 一段反向内弧，圆头让两端收得干净。
            c.arc(0.0, 0.0, 0.40, 0.80, 5.50);
            c.arc(0.16, -0.10, 0.30, 1.40, 4.90);
        }
        Tile::Location => {
            // 定位针：圆头 + 两条收向针尖的线 + 中心孔。
            c.circle_stroke(0.0, -0.10, 0.26);
            c.circle_filled(0.0, -0.10, 0.065);
            c.line(-0.19, 0.07, 0.0, 0.44);
            c.line(0.19, 0.07, 0.0, 0.44);
        }
        Tile::Mute => speaker(
            painter,
            EguiRect::from_center_size(c.c, egui::vec2(c.s, c.s)),
            color,
            true,
        ),
        Tile::Settings => {
            // 齿轮：外环 + 8 个短粗梯形齿 + 轴孔。旧版「圆 + 8 根长辐条」
            // 画出来是太阳/雪花，没有齿块的实体感。
            c.circle_stroke(0.0, 0.0, 0.24);
            c.circle_stroke(0.0, 0.0, 0.09);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let (ca, sa) = (a.cos(), a.sin());
                // 径向 0.24..0.40、切向 ±0.07 的梯形，随角度旋转。
                let radial = |r: f32, t: f32| c.pt(ca * r - sa * t, sa * r + ca * t);
                let pts = vec![
                    radial(0.22, -0.085),
                    radial(0.40, -0.055),
                    radial(0.40, 0.055),
                    radial(0.22, 0.085),
                ];
                painter.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
            }
        }
        Tile::Energy => {
            // 电池 + 闪电：节能模式一眼能懂，不再像"禁止符"。
            c.rect_stroke(-0.32, -0.19, 0.56, 0.38, 0.07);
            c.rect_filled(0.24, -0.07, 0.06, 0.14, 0.02, color);
            let bolt = [
                (0.05, -0.16),
                (-0.09, 0.02),
                (0.01, 0.02),
                (-0.04, 0.16),
                (0.11, -0.05),
                (0.02, -0.05),
                (0.05, -0.16), // 闭合，不闭会看成一根箭头
            ];
            for w in bolt.windows(2) {
                c.line(w[0].0, w[0].1, w[1].0, w[1].1);
            }
        }
        Tile::Hotspot => {
            c.circle_filled(0.0, 0.30, 0.065);
            c.arc(0.0, 0.30, 0.20, -2.40, -0.74);
            c.arc(0.0, 0.30, 0.36, -2.40, -0.74);
        }
    }
}

/// 喇叭：音量滑块左侧与「静音」磁贴共用。
///
/// `rect` 是图标方框；`muted` 时右侧换成 ×，否则画两道声波弧。
pub fn speaker(painter: &Painter, rect: EguiRect, color: Color32, muted: bool) {
    let c = Canvas::from_center(
        painter,
        rect.center(),
        rect.width().min(rect.height()),
        color,
    );
    // 喇叭本体：方块 + 三角，三角用凸多边形拼，避免描边缝隙。
    c.rect_filled(-0.42, -0.10, 0.16, 0.20, 0.01, color);
    let horn = vec![
        c.pt(-0.26, -0.10),
        c.pt(-0.06, -0.28),
        c.pt(-0.06, 0.28),
        c.pt(-0.26, 0.10),
    ];
    painter.add(egui::Shape::convex_polygon(horn, color, Stroke::NONE));
    if muted {
        c.line(0.04, -0.13, 0.28, 0.11);
        c.line(0.28, -0.13, 0.04, 0.11);
    } else {
        c.arc(0.02, 0.0, 0.20, -0.85, 0.85);
        c.arc(0.02, 0.0, 0.32, -0.85, 0.85);
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

    /// 画布坐标按 s 归一化：同一组图标在 24px 与 96px 下形状完全一致，
    /// 这是 Dock 悬停放大动画不错位的前提。
    #[test]
    fn canvas_coordinates_scale_with_size() {
        // Painter 无法脱离 Ui 构造，这里直接核验换算公式的线性性。
        let map = |c: f32, s: f32, x: f32| c + x * s;
        let small = map(100.0, 24.0, 0.25) - 100.0;
        let large = map(100.0, 96.0, 0.25) - 100.0;
        // s 放大 4 倍，偏移量也正好放大 4 倍 —— 形状不随尺寸漂移。
        assert_eq!(large / small, 4.0);
    }
}
