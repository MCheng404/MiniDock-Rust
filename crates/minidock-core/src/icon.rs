//! 图标适配：决定一块位图里**哪一部分**该画、占多大、要不要圆角。
//!
//! 逐条对应原版 `FitBitmap()`（`minidock.c` 578 行）的三档语义：
//!
//! | `fit` | 原版文案 | 做法 |
//! |---|---|---|
//! | 0 | 自动（去除透明 / 纯色边，推荐） | 先按 alpha 求内容边界；若是整块不透明，再按"与左上角同色即背景"二次收紧，并加圆角 |
//! | 1 | 完整保留（不裁切，可能有留白） | 整图不动，按 94% 缩放 |
//! | 2 | 居中裁切并填满（圆角方形） | 取中心正方形，按 100% 缩放 + 圆角 |
//!
//! 这里只做**几何与遮罩**，不重采样：绘制阶段把 `bbox` 转成 UV 子矩形交给
//! GPU 缩放，比在 CPU 上逐像素插值又快又清楚（Dock 图标要随放大动画连续变化）。
//!
//! 为什么要二次收紧（`solid_bbox`）：Win11 的计算器图标是**整块不透明的深色
//! 方块**，alpha 全 255。只按 alpha 求边界会得到整图，于是 Dock 上会出现一个
//! 深色大方块；原版会把同色的底板当背景裁掉，只剩下中间的图形。

/// 一块矩形区域，`x1` / `y1` 为**独占**上界。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BBox {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

impl BBox {
    #[must_use]
    pub fn width(self) -> u32 {
        self.x1.saturating_sub(self.x0)
    }

    #[must_use]
    pub fn height(self) -> u32 {
        self.y1.saturating_sub(self.y0)
    }

    #[must_use]
    pub fn is_full(self, w: u32, h: u32) -> bool {
        self.x0 == 0 && self.y0 == 0 && self.x1 == w && self.y1 == h
    }

    #[must_use]
    pub fn full(w: u32, h: u32) -> Self {
        Self {
            x0: 0,
            y0: 0,
            x1: w,
            y1: h,
        }
    }
}

/// 适配方式。数值即 ini 里 `fit` 字段。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum FitMode {
    /// 自动去边（含纯色底板）。
    #[default]
    Auto = 0,
    /// 完整保留。
    Full = 1,
    /// 居中裁切并填满。
    Square = 2,
}

impl FitMode {
    /// 越界按 [`FitMode::Auto`] 处理 —— 原版用 `_wtoi` 直接吃进 int，
    /// 未知值在 `FitBitmap` 里走到"不裁切"分支；对我们来说按自动更合理。
    #[must_use]
    pub const fn from_code(v: i32) -> Self {
        match v {
            1 => Self::Full,
            2 => Self::Square,
            _ => Self::Auto,
        }
    }
}

/// 适配结果。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct IconFit {
    /// 要画的源区域。
    pub bbox: BBox,
    /// `Some(半径)` = 内容是不透明整块，需要按圆角遮罩。半径以**源像素**计。
    pub rounded: Option<f32>,
    /// 内容在目标尺寸上的填充比例（原版是 0.94 或 1.0）。
    pub fill: f32,
}

/// 原版 `FitBitmap` 的 fill 参数，三档都是 0.94，只有居中裁切用 1.0。
const FILL: f32 = 0.94;
/// 圆角半径占短边的比例（原版 `* 0.22f`）。
const CORNER: f32 = 0.22;
/// alpha 边界阈值（原版 `AlphaBBox(..., 40)`）。
const ALPHA_THR: u8 = 40;

/// 计算适配区域。
#[must_use]
pub fn fit_icon(rgba: &[u8], w: u32, h: u32, mode: FitMode) -> IconFit {
    if w == 0 || h == 0 || rgba.len() < (w as usize) * (h as usize) * 4 {
        return IconFit {
            bbox: BBox::full(w, h),
            rounded: None,
            fill: FILL,
        };
    }

    match mode {
        FitMode::Full => IconFit {
            bbox: BBox::full(w, h),
            rounded: None,
            fill: FILL,
        },
        FitMode::Square => {
            // 原版：取短边做中心正方形，fill = 1.0。
            let side = w.min(h);
            let bbox = BBox {
                x0: (w - side) / 2,
                y0: (h - side) / 2,
                x1: (w - side) / 2 + side,
                y1: (h - side) / 2 + side,
            };
            IconFit {
                bbox,
                rounded: Some(side as f32 * CORNER),
                fill: 1.0,
            }
        }
        FitMode::Auto => {
            let alpha = alpha_bbox(rgba, w, h, ALPHA_THR);
            if !alpha.is_full(w, h) {
                // 有透明边 → 只裁透明，不加圆角。
                return IconFit {
                    bbox: alpha,
                    rounded: None,
                    fill: FILL,
                };
            }
            // 整块不透明：试着把同色底板也裁掉，并加圆角。
            match solid_bbox(rgba, w, h) {
                Some(b) => IconFit {
                    bbox: b,
                    rounded: Some(b.width().min(b.height()) as f32 * CORNER),
                    fill: FILL,
                },
                None => IconFit {
                    bbox: alpha,
                    rounded: Some(w.min(h) as f32 * CORNER),
                    fill: FILL,
                },
            }
        }
    }
}

/// alpha 大于阈值的最小包围盒。整图全透明时返回全图（不是空盒 ——
/// 原版也这样，否则后面会算出 0 尺寸）。
#[must_use]
pub fn alpha_bbox(rgba: &[u8], w: u32, h: u32, thr: u8) -> BBox {
    let (mut minx, mut miny) = (w, h);
    // 初值 -1：没有任何命中时才能和"命中在第 0 列"区分开。
    let (mut maxx, mut maxy) = (-1i64, -1i64);
    for y in 0..h {
        let row = (y as usize) * (w as usize) * 4;
        for x in 0..w {
            if rgba[row + (x as usize) * 4 + 3] > thr {
                minx = minx.min(x);
                maxx = maxx.max(i64::from(x));
                miny = miny.min(y);
                maxy = maxy.max(i64::from(y));
            }
        }
    }
    if maxx < 0 {
        return BBox::full(w, h);
    }
    BBox {
        x0: minx,
        y0: miny,
        x1: maxx as u32 + 1,
        y1: maxy as u32 + 1,
    }
}

/// 把"与左上角同色"的底板当背景裁掉。整图就是底板本身（没有前景）时返回 `None`。
///
/// 判定规则照抄原版：alpha > 200 且与参考色三通道绝对差之和 < 70 视为背景；
/// 某一行/列里背景像素占比 > 98% 就继续向内收。
#[must_use]
pub fn solid_bbox(rgba: &[u8], w: u32, h: u32) -> Option<BBox> {
    if w < 2 || h < 2 {
        return None;
    }
    let stride = (w as usize) * 4;
    let px = |x: u32, y: u32| -> &[u8] {
        let i = (y as usize) * stride + (x as usize) * 4;
        &rgba[i..i + 4]
    };
    let (r0, g0, b0, a0) = {
        let p = px(0, 0);
        (p[0], p[1], p[2], p[3])
    };
    // 左上角本身是透明的 → 这个图标没有"纯色底板"，交给 alpha 边界处理。
    if a0 < 200 {
        return None;
    }
    let is_bg = |p: &[u8]| -> bool {
        p[3] > 200
            && (i32::from(p[0]) - i32::from(r0)).abs()
                + (i32::from(p[1]) - i32::from(g0)).abs()
                + (i32::from(p[2]) - i32::from(b0)).abs()
                < 70
    };

    let mut top = 0u32;
    let mut bot = h - 1;
    let mut lef = 0u32;
    let mut rig = w - 1;

    // 整数比较，避免浮点：bad * 100 > 总数 * 2 即"非背景占比超过 2%"。
    let mostly_bg_row = |y: u32| -> bool {
        let bad = (0..w).filter(|&x| !is_bg(px(x, y))).count() as u64;
        bad * 100 <= u64::from(w) * 2
    };
    let mostly_bg_col = |x: u32, top: u32, bot: u32| -> bool {
        let bad = (top..=bot).filter(|&y| !is_bg(px(x, y))).count() as u64;
        bad * 100 <= u64::from(bot - top + 1) * 2
    };

    while top < bot && mostly_bg_row(top) {
        top += 1;
    }
    while bot > top && mostly_bg_row(bot) {
        bot -= 1;
    }
    while lef < rig && mostly_bg_col(lef, top, bot) {
        lef += 1;
    }
    while rig > lef && mostly_bg_col(rig, top, bot) {
        rig -= 1;
    }

    // 太薄的结果多半是误判（例如整块同色），保留全图。
    if bot - top > 8 && rig - lef > 8 {
        Some(BBox {
            x0: lef,
            y0: top,
            x1: rig + 1,
            y1: bot + 1,
        })
    } else {
        None
    }
}

/// 在 `bbox` 内按圆角遮罩清零 alpha（模拟原版的 `GdipSetClipPath`）。
///
/// 只在准备纹理时做一次。半径按源像素给，缩放后自然等比 —— 所以 Dock 图标
/// 在放大动画的任意尺寸下圆角比例都一致。
pub fn apply_round_mask(rgba: &mut [u8], w: u32, bbox: BBox, radius: f32) {
    if radius <= 0.0 {
        return;
    }
    let (bw, bh) = (bbox.width() as f32, bbox.height() as f32);
    if bw <= 0.0 || bh <= 0.0 {
        return;
    }
    // 半径不能超过短边一半，否则圆角会自交。
    let r = radius.min(bw.min(bh) / 2.0);
    let stride = (w as usize) * 4;

    for y in bbox.y0..bbox.y1 {
        let fy = (y - bbox.y0) as f32 + 0.5;
        for x in bbox.x0..bbox.x1 {
            let fx = (x - bbox.x0) as f32 + 0.5;
            // 只在四个角的方形区域内判定。
            let dx = if fx < r {
                r - fx
            } else if fx > bw - r {
                fx - (bw - r)
            } else {
                0.0
            };
            let dy = if fy < r {
                r - fy
            } else if fy > bh - r {
                fy - (bh - r)
            } else {
                0.0
            };
            if dx > 0.0 && dy > 0.0 && dx * dx + dy * dy > r * r {
                let i = (y as usize) * stride + (x as usize) * 4;
                rgba[i + 3] = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一张 `w×h` 的图，`f(x,y)` 返回 RGBA。
    fn make(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                v.extend_from_slice(&f(x, y));
            }
        }
        v
    }

    #[test]
    fn alpha_bbox_finds_content() {
        // 32×32 全透明，中间 10..22 是实心方块。
        let img = make(32, 32, |x, y| {
            if (10..22).contains(&x) && (10..22).contains(&y) {
                [255, 0, 0, 255]
            } else {
                [0, 0, 0, 0]
            }
        });
        let b = alpha_bbox(&img, 32, 32, 40);
        assert_eq!(
            b,
            BBox {
                x0: 10,
                y0: 10,
                x1: 22,
                y1: 22
            }
        );
    }

    #[test]
    fn alpha_bbox_returns_full_when_fully_transparent() {
        // 全透明时返回全图：原版也这样，返回空盒会让后续算 0 尺寸。
        let img = make(8, 8, |_, _| [0, 0, 0, 0]);
        assert_eq!(alpha_bbox(&img, 8, 8, 40), BBox::full(8, 8));
    }

    #[test]
    fn solid_bbox_trims_flat_plate() {
        // 模拟 Win11 计算器：整块 (70,70,70) 不透明，中间 20..40 有白色图形。
        let img = make(64, 64, |x, y| {
            if (20..40).contains(&x) && (20..40).contains(&y) {
                [250, 250, 250, 255]
            } else {
                [70, 70, 70, 255]
            }
        });
        let b = solid_bbox(&img, 64, 64).expect("应当裁掉纯色底板");
        assert_eq!(
            b,
            BBox {
                x0: 20,
                y0: 20,
                x1: 40,
                y1: 40
            }
        );
    }

    #[test]
    fn solid_bbox_gives_up_on_uniform_image() {
        // 整块同色 = 底板即全部，裁完什么都不剩 —— 原版靠"太薄就放弃"兜住。
        let img = make(64, 64, |_, _| [70, 70, 70, 255]);
        assert_eq!(solid_bbox(&img, 64, 64), None);
    }

    #[test]
    fn solid_bbox_gives_up_when_corner_transparent() {
        // 左上角透明说明没有纯色底板，这条路径不该介入。
        let img = make(64, 64, |x, y| {
            if x < 4 && y < 4 {
                [0, 0, 0, 0]
            } else {
                [70, 70, 70, 255]
            }
        });
        assert_eq!(solid_bbox(&img, 64, 64), None);
    }

    /// 三档适配的核心区别：`Auto` 会裁、`Full` 不裁、`Square` 取中心正方形。
    #[test]
    fn fit_modes_differ_as_expected() {
        // 64×64 不透明底板 + 中心图形 → Auto 会收紧并加圆角。
        let plate = make(64, 64, |x, y| {
            if (20..40).contains(&x) && (20..40).contains(&y) {
                [250, 250, 250, 255]
            } else {
                [70, 70, 70, 255]
            }
        });
        let auto = fit_icon(&plate, 64, 64, FitMode::Auto);
        assert_eq!(auto.bbox.width(), 20);
        assert!(auto.rounded.is_some(), "纯色底板应当加圆角");
        assert!((auto.fill - 0.94).abs() < f32::EPSILON);

        let full = fit_icon(&plate, 64, 64, FitMode::Full);
        assert_eq!(full.bbox, BBox::full(64, 64));
        assert!(full.rounded.is_none());

        // 非正方形时才看得出裁切：64×32 取中心 32×32。
        let wide = make(64, 32, |_, _| [1, 2, 3, 255]);
        let sq = fit_icon(&wide, 64, 32, FitMode::Square);
        assert_eq!(sq.bbox.width(), 32);
        assert_eq!(sq.bbox.height(), 32);
        assert_eq!(sq.bbox.x0, 16);
        assert!((sq.fill - 1.0).abs() < f32::EPSILON);
        assert!(sq.rounded.is_some());
    }

    #[test]
    fn auto_keeps_transparent_icons_unrounded() {
        // 有透明边的图标（记事本那种）：只裁 alpha，不加圆角。
        let img = make(32, 32, |x, y| {
            if (8..24).contains(&x) && (8..24).contains(&y) {
                [100, 150, 200, 255]
            } else {
                [0, 0, 0, 0]
            }
        });
        let f = fit_icon(&img, 32, 32, FitMode::Auto);
        assert_eq!(f.bbox.width(), 16);
        assert!(f.rounded.is_none(), "透明边图标不该被圆角切掉");
    }

    #[test]
    fn round_mask_clears_corners_only() {
        // 32×32 全不透明，圆角半径 8：四角应被清空，中心保持不透明。
        let mut img = make(32, 32, |_, _| [255, 0, 0, 255]);
        let bbox = BBox::full(32, 32);
        apply_round_mask(&mut img, 32, bbox, 8.0);
        let a = |x: u32, y: u32| img[((y * 32 + x) * 4 + 3) as usize];
        assert_eq!(a(0, 0), 0, "左上角应被切掉");
        assert_eq!(a(31, 0), 0, "右上角应被切掉");
        assert_eq!(a(0, 31), 0, "左下角应被切掉");
        assert_eq!(a(31, 31), 0, "右下角应被切掉");
        assert_eq!(a(16, 16), 255, "中心不该动");
        assert_eq!(a(16, 0), 255, "上边中点不该动");
        assert_eq!(a(0, 16), 255, "左边中点不该动");
    }

    #[test]
    fn round_mask_clamps_radius_to_half_short_side() {
        // 半径给到离谱的值也不能把整块擦掉：夹到短边一半。
        let mut img = make(16, 16, |_, _| [255, 0, 0, 255]);
        apply_round_mask(&mut img, 16, BBox::full(16, 16), 1000.0);
        let a = |x: u32, y: u32| img[((y * 16 + x) * 4 + 3) as usize];
        assert_eq!(a(8, 8), 255, "中心必须还在");
        assert_eq!(a(0, 0), 0);
    }

    /// 空图 / 尺寸不符时不能 panic，要退回"整图不裁"。
    #[test]
    fn degenerate_inputs_fall_back_to_full() {
        assert_eq!(fit_icon(&[], 0, 0, FitMode::Auto).bbox, BBox::full(0, 0));
        let short = vec![0u8; 16];
        let f = fit_icon(&short, 32, 32, FitMode::Auto);
        assert_eq!(f.bbox, BBox::full(32, 32));
    }
}
