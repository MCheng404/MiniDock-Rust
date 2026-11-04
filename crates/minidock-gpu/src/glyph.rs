//! 内置条目的图标：全部用 `painter` 现画，不依赖字体与 emoji。
//!
//! Dock 上有两个"不是文件"的条目：`mdock:start`（开始菜单）与
//! `mdock:launchpad`（起动台）。它们没有可取的 shell 图标，走图标线程只会
//! 拿到"首字占位"—— 用户看到的是一个写着「起」的方块。
//!
//! 这里按图形画：圆角底 + 几何元素。三条硬约束：
//!
//! 1. **不用 emoji**：字体里不一定有，缺字时是一块豆腐，而且彩色字形跟
//!    其余图标的观感完全不搭；
//! 2. **不用渐变**：项目的视觉基调是纯平，渐变在这个尺寸上只会显脏；
//! 3. **全部按矩形比例算**：图标尺寸每帧都在变（悬停放大动画），
//!    写死像素会在放大到一半时错位。
//!
//! 图形比例沿用原版（四格窗 / 九宫格点阵）。

use egui::{Color32, CornerRadius, Painter, Pos2, Rect as EguiRect, Stroke};

/// 需要现画的内置条目。
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

/// 画一块内置图标，铺满 `rect`（逻辑点）。
pub fn paint_tile(painter: &Painter, rect: EguiRect, kind: Builtin) {
    // 与 shell 图标一样留一点边：别的图标本体也不占满整格。
    let t = rect.shrink(rect.width() * 0.04);
    let r = CornerRadius::same(round(t.width() * 0.22));
    match kind {
        Builtin::Start => {
            painter.rect_filled(t, r, Color32::from_rgb(46, 123, 246));
            let q = t.width() * 0.27;
            let gp = t.width() * 0.04;
            let o = (t.width() - 2.0 * q - gp) / 2.0;
            for i in 0..4 {
                let x = t.left() + o + (i % 2) as f32 * (q + gp);
                let y = t.top() + o + (i / 2) as f32 * (q + gp);
                painter.rect_filled(
                    EguiRect::from_min_size(Pos2::new(x, y), egui::vec2(q, q)),
                    CornerRadius::same(round(q * 0.08)),
                    Color32::WHITE,
                );
            }
        }
        Builtin::Launchpad => {
            painter.rect_filled(t, r, Color32::from_rgb(58, 62, 78));
            // 九宫格配色沿用原版：第一行红黄绿，第二行蓝紫橙，第三行青粉绿。
            const COLS: [Color32; 9] = [
                Color32::from_rgb(255, 95, 87),
                Color32::from_rgb(254, 188, 46),
                Color32::from_rgb(40, 200, 64),
                Color32::from_rgb(10, 132, 255),
                Color32::from_rgb(191, 90, 242),
                Color32::from_rgb(255, 159, 10),
                Color32::from_rgb(100, 210, 255),
                Color32::from_rgb(255, 55, 95),
                Color32::from_rgb(48, 209, 88),
            ];
            let q = t.width() * 0.2;
            let gp = t.width() * 0.075;
            let o = (t.width() - 3.0 * q - 2.0 * gp) / 2.0;
            for (i, c) in COLS.iter().enumerate() {
                let x = t.left() + o + (i % 3) as f32 * (q + gp);
                let y = t.top() + o + (i / 3) as f32 * (q + gp);
                painter.rect_filled(
                    EguiRect::from_min_size(Pos2::new(x, y), egui::vec2(q, q)),
                    CornerRadius::same(round(q * 0.28)),
                    *c,
                );
            }
        }
    }
}

/// 控制中心卡片的图标：两条滑轨 + 两个滑块。
///
/// 这个卡片此前**什么都不画**：条带给它留了 0.72·栏高 的宽度，点得到、
/// 看不见，像胶囊上缺了一块。
pub fn sliders(painter: &Painter, rect: EguiRect, color: Color32) {
    let w = rect.width();
    let h = rect.height();
    let x0 = rect.left() + w * 0.1;
    let x1 = rect.right() - w * 0.1;
    let stroke = Stroke::new((w * 0.1).max(1.0), color);
    // 两条轨，滑块一左一右，像"开 / 关"各一条。
    for (i, knob_at) in [(0, 0.68_f32), (1, 0.32_f32)] {
        let y = rect.top() + h * if i == 0 { 0.34 } else { 0.66 };
        painter.line_segment([Pos2::new(x0, y), Pos2::new(x1, y)], stroke);
        let kx = x0 + (x1 - x0) * knob_at;
        painter.circle_filled(Pos2::new(kx, y), w * 0.14, color);
    }
}

/// 半径取整到整数像素，避免半点像素造成的糊边。
fn round(v: f32) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
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
}
