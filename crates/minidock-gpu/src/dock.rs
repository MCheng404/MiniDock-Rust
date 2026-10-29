//! M1 的 Dock 本体：布局状态机 + egui 绘制。
//!
//! 几何**逐条照抄 C 版 v1.4.0 的 `Paint()`**（`minidock.c` 的 `DrawBar` /
//! `DrawBubbleAt` / `HitIndex` / `MapActualToBase`），验收标准是"与 C 版
//! 同配置并排截图，栏体几何误差 ≤ 1 px"。
//!
//! ## 单位
//!
//! `core::layout` 的度量是**物理像素**（与 C 版一致：`cfg` 里存的就是
//! `Sc()` 缩放后的物理值，C 版的 `LW/LH` 也是物理像素）。
//! egui 的画布是**逻辑点**，所以绘制时统一乘 `1 / ppp` 换算。
//! 搞混这两套单位是 M0 已经踩过的坑（表面尺寸与客户区错位）。
//!
//! ## 本轮范围
//!
//! 含：栏体、时钟卡片、分割线、图标占位、运行指示点、悬停气泡、悬停放大。
//! 不含：图标加载（M2）、拖拽重排（M2）、倒影、磨砂（需要桌面捕获）。

use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect as EguiRect, Stroke, StrokeKind};
use minidock_core::config::{Config, Edge, Key};
use minidock_core::item::{Item, ItemKind};
use minidock_core::layout::{self, Metrics, Rect as LRect};

use egui::epaint::Direction;

/// 放大动画的每帧收敛系数（C 版 `it->cur += d * 0.38f`）。
const SIZE_LERP: f32 = 0.38;
/// 动画收敛阈值：差值小于它就直接吸附（C 版 `fabsf(d) > 0.3f`）。
const SNAP: f32 = 0.3;
/// 放大影响的半径系数（C 版 `R = (S + g) * 2.6f`）。
const MAG_RADIUS: f32 = 2.6;

/// 一条 Dock 条目的运行时状态。
#[derive(Clone, Debug)]
pub struct DockItem {
    /// 显示名（气泡里用）。
    pub name: String,
    pub kind: ItemKind,
    /// 当前边长。放大动画的值，对应 C 版 `items[i].cur`。
    pub cur: f32,
    /// 拖拽滑动的偏移，对应 C 版 `items[i].slide`（M1 不用，保留字段）。
    pub slide: f32,
}

impl DockItem {
    /// 从配置条目构造。`cur` 初始为 0 —— C 版也是 0，靠动画长出来。
    #[must_use]
    pub fn from_item(item: &Item) -> Self {
        Self {
            name: item.name.clone(),
            kind: item.kind,
            cur: 0.0,
            slide: 0.0,
        }
    }

    fn is_sep(&self) -> bool {
        self.kind == ItemKind::Separator
    }
}

/// 派生几何。全部是物理像素。
#[derive(Clone, Debug, Default)]
pub struct DockGeom {
    /// 栏体左边界。
    pub bar_l: f32,
    /// 栏体宽度。
    pub bar_w: f32,
    /// 栏体上边界。
    pub bar_top: f32,
    /// 栏体下边界。
    pub bar_bottom: f32,
    /// 图标底边（图标从这里往上长）。
    pub icon_bottom: f32,
    /// 每个槽位的左边界。
    pub slot_l: Vec<f32>,
    /// 每个槽位的右边界。
    pub slot_r: Vec<f32>,
    /// 每个槽位的**实际**中心（放大后）。
    pub act_c: Vec<f32>,
    /// 每个槽位的**基准**中心（未放大）。
    pub base_c: Vec<f32>,
}

/// Dock 本体。
#[derive(Debug)]
pub struct Dock {
    pub cfg: Config,
    pub items: Vec<DockItem>,
    /// 工作区（物理像素），用于算可用边长。
    pub work: LRect,
    /// 鼠标在窗口内的**逻辑**坐标（未命中时为负）。
    pub mouse: (f32, f32),
    /// 鼠标是否在窗口内。
    pub hover: bool,
    /// 当前悬停的条目下标。
    pub hot: Option<usize>,
    /// 隐藏进度：0 = 完全显示，1 = 完全隐藏。
    pub hide_cur: f32,
    /// 本帧是否有动画（决定要不要继续出帧）。
    pub anim: bool,
    pub metrics: Metrics,
    pub geom: DockGeom,
    /// 上一帧的实际中心，供 `map_actual_to_base` 用（与 C 版同序）。
    prev_act_c: Vec<f32>,
}

impl Dock {
    /// 建一个 Dock。`work` 是工作区矩形（物理像素）。
    #[must_use]
    pub fn new(cfg: Config, items: Vec<DockItem>, work: LRect) -> Self {
        Self {
            cfg,
            items,
            work,
            mouse: (-1.0, -1.0),
            hover: false,
            hot: None,
            hide_cur: 0.0,
            anim: true,
            metrics: Metrics::default(),
            geom: DockGeom::default(),
            prev_act_c: Vec::new(),
        }
    }

    /// 逻辑窗口尺寸（egui 画布用）。物理尺寸 = 逻辑 × ppp。
    #[must_use]
    pub fn logical_size(&self) -> (f32, f32) {
        // `metrics.lw/lh` 是物理像素；调用方按 ppp 换算，这里给出物理值，
        // 由 `Overlay` 决定窗口的物理尺寸。
        (self.metrics.lw as f32, self.metrics.lh as f32)
    }

    /// 推进一帧：重算度量、放大动画、槽位与命中。
    ///
    /// `dpi` 只在度量里用来做 DPI 缩放。
    pub fn update(&mut self, dpi: i32) {
        let n = self.items.len();
        let n_sep = self.items.iter().filter(|i| i.is_sep()).count();
        let m = layout::calc_metrics(&self.cfg, dpi, n, n_sep, self.work);
        self.metrics = m;

        let s = m.s;
        let g = m.gap;
        let sep_w = m.sep_w;

        // ---- 基准中心（未放大时的槽位）----
        let base_w = {
            let mut w = 2.0 * m.pad_x;
            for it in &self.items {
                w += if it.is_sep() { sep_w } else { s };
            }
            if n > 1 {
                w += g * (n - 1) as f32;
            }
            if n == 0 {
                w = 2.0 * m.pad_x + s * 4.0;
            }
            w
        };
        let bl = (m.lw as f32 - base_w - m.clock_extra) / 2.0;
        let mut x = bl + m.pad_x;
        let mut base_c = Vec::with_capacity(n);
        for it in &self.items {
            let w = if it.is_sep() { sep_w } else { s };
            base_c.push(x + w / 2.0);
            x += w + g;
        }
        self.geom.base_c = base_c;

        // ---- 放大动画 ----
        // 权重用**基准**位置算：图标放大后槽位会移动，直接用实际位置会让
        // "鼠标属于哪个图标"随放大程度漂移（C 版 `MapActualToBase`）。
        let mag =
            self.cfg.zoom && self.hover && n > 0 && self.mouse.0 >= 0.0 && self.hide_cur < 0.5;
        let u = if mag {
            layout::map_actual_to_base(self.mouse.0, &self.geom.base_c, &self.prev_act_c)
        } else {
            0.0
        };
        let r = (s + g) * MAG_RADIUS;
        let mut anim = false;
        for (i, it) in self.items.iter_mut().enumerate() {
            let target = if it.is_sep() {
                sep_w
            } else if mag {
                let w = layout::magnify_weight(self.geom.base_c[i] - u, r);
                s + (m.z - s) * w
            } else {
                s
            };
            let d = target - it.cur;
            if d.abs() > SNAP {
                it.cur += d * SIZE_LERP;
                anim = true;
            } else {
                it.cur = target;
            }
        }

        // ---- 实际槽位 ----
        let mut tot = 2.0 * m.pad_x;
        for it in &self.items {
            tot += it.cur;
        }
        if n > 1 {
            tot += g * (n - 1) as f32;
        }
        if n == 0 {
            tot += s * 4.0;
        }
        let mut bar_w = tot;
        let max_w = m.lw as f32 - 4.0 - m.clock_extra;
        if bar_w > max_w {
            bar_w = max_w;
        }
        let bar_l = (m.lw as f32 - bar_w - m.clock_extra) / 2.0;
        let mut x = bar_l + m.pad_x;
        let mut slot_l = Vec::with_capacity(n);
        let mut slot_r = Vec::with_capacity(n);
        let mut act_c = Vec::with_capacity(n);
        for it in &self.items {
            slot_l.push(x);
            slot_r.push(x + it.cur);
            act_c.push((x + (x + it.cur)) / 2.0);
            x += it.cur + g;
        }

        let bar_bottom = m.lh as f32 - m.margin - 2.0;
        let bar_top = bar_bottom - m.bar_h;
        let icon_bottom = bar_bottom - m.pad - 2.0;

        self.geom = DockGeom {
            bar_l,
            bar_w,
            bar_top,
            bar_bottom,
            icon_bottom,
            slot_l,
            slot_r,
            act_c: act_c.clone(),
            base_c: self.geom.base_c.clone(),
        };
        // 下一帧的 `map_actual_to_base` 用本帧的实际中心（C 版同序）。
        self.prev_act_c = act_c;

        // ---- 命中 ----
        // 必须用**本帧**的槽位，否则气泡会比图标慢一帧。
        let visible: Vec<bool> = self.items.iter().map(|i| i.cur > 1.0).collect();
        self.hot = if self.hover && self.hide_cur < 0.5 {
            layout::hit_index(
                self.mouse.0,
                self.mouse.1,
                &self.geom.slot_l,
                &self.geom.slot_r,
                &visible,
                g,
                bar_top,
                m.z,
                m.lh as f32,
            )
        } else {
            None
        };

        self.anim = anim;
    }

    /// 隐藏动画推进。返回是否仍在动画中。
    pub fn tick_hide(&mut self, target_hidden: bool) -> bool {
        let target = if target_hidden { 1.0 } else { 0.0 };
        let d = target - self.hide_cur;
        if d.abs() > 0.01 {
            self.hide_cur += d * 0.25;
            self.anim = true;
            true
        } else {
            self.hide_cur = target;
            false
        }
    }

    /// 绘制整个 Dock。
    ///
    /// * `ppp` —— 每逻辑点对应多少物理像素。
    /// * `dark` —— 深色主题。
    /// * `clock` —— 时钟卡片的（时间, 日期）文本；`None` 表示不画时钟。
    pub fn paint(&self, ui: &egui::Ui, ppp: f32, dark: bool, clock: Option<(&str, &str)>) {
        let k = 1.0 / ppp;
        let m = &self.metrics;
        let g = &self.geom;
        let painter = ui.painter();

        // 隐藏位移：整条 Dock 平移出屏幕边缘。
        let off = self.hide_cur * (m.bar_h + m.margin + 14.0);
        let (tx, ty) = match self.cfg.edge {
            Edge::Top => (0.0, -off),
            Edge::Left => (-off, 0.0),
            Edge::Right => (off, 0.0),
            Edge::Bottom => (0.0, off),
        };

        // 栏体。
        let rad = self.cfg.radius as f32;
        let rad = rad.min(m.bar_h / 2.0) * k;
        let bar = self.map(g.bar_l, g.bar_top, g.bar_w, m.bar_h, k, tx, ty);
        self.paint_bar(painter, bar, rad, dark);

        if self.items.is_empty() {
            painter.text(
                bar.center(),
                Align2::CENTER_CENTER,
                "将应用 / 文件夹 / 网址拖到这里",
                FontId::proportional(13.0),
                Color32::from_rgba_unmultiplied(255, 255, 255, 190),
            );
        }

        // 时钟卡片：栏体 + 时间 + 日期（横向/纵向都由 clock_w 决定）。
        if let Some((time, date)) = clock {
            let c = self.map(
                g.bar_l + g.bar_w + m.gap + 6.0,
                g.bar_top,
                m.clock_w as f32,
                m.bar_h,
                k,
                tx,
                ty,
            );
            self.paint_bar(painter, c, rad, dark);
            let tc = if dark {
                Color32::from_rgba_unmultiplied(245, 240, 240, 245)
            } else {
                Color32::from_rgba_unmultiplied(245, 30, 30, 36)
            };
            let dc = if dark {
                Color32::from_rgba_unmultiplied(190, 220, 220, 228)
            } else {
                Color32::from_rgba_unmultiplied(190, 60, 60, 70)
            };
            painter.text(
                EguiRect::from_min_max(
                    Pos2::new(c.left(), c.top() + c.height() * 0.12),
                    Pos2::new(c.right(), c.top() + c.height() * 0.56),
                )
                .center(),
                Align2::CENTER_CENTER,
                time,
                FontId::proportional(14.0),
                tc,
            );
            painter.text(
                EguiRect::from_min_max(
                    Pos2::new(c.left(), c.top() + c.height() * 0.54),
                    Pos2::new(c.right(), c.top() + c.height() * 0.86),
                )
                .center(),
                Align2::CENTER_CENTER,
                date,
                FontId::proportional(11.0),
                dc,
            );
        }

        // 分割线：斜线，从图标底部往上 0.84·S。
        let sep_color = if dark {
            Color32::from_rgba_unmultiplied(110, 255, 255, 255)
        } else {
            Color32::from_rgba_unmultiplied(120, 60, 60, 70)
        };
        for (i, it) in self.items.iter().enumerate() {
            if !it.is_sep() || it.cur < 2.0 {
                continue;
            }
            let cx = g.act_c[i] + it.slide;
            let (lx, ly, lw, lh) = layout::map_rect(
                self.cfg.edge,
                cx,
                g.icon_bottom - m.s * 0.88,
                0.0,
                m.s * 0.84,
                m.lh as f32,
            );
            let a = self.to_egui(lx, ly, k, tx, ty);
            let b = self.to_egui(lx + lw, ly + lh, k, tx, ty);
            painter.line_segment([a, b], Stroke::new(1.4 * k, sep_color));
        }

        // 图标（占位）+ 运行指示点。
        for (i, it) in self.items.iter().enumerate() {
            if it.is_sep() || it.cur < 1.0 {
                continue;
            }
            let sz = it.cur;
            let lx = g.slot_l[i] + it.slide;
            let (ix, iy, iw, ih) =
                layout::map_rect(self.cfg.edge, lx, g.icon_bottom - sz, sz, sz, m.lh as f32);
            let r = self.to_egui(ix, iy, k, tx, ty);
            let rect = EguiRect::from_min_size(r, egui::vec2(iw * k, ih * k));
            // 占位：圆角方块 + 首字。真正的图标是 M2 的事。
            let plate = self.cfg.get_i32(Key::PLATE_MODE);
            let mut inner = rect;
            if plate > 0 {
                let mut pr = sz * self.cfg.get_i32(Key::PlateRadius) as f32 / 100.0;
                if pr > sz / 2.0 {
                    pr = sz / 2.0;
                }
                let pc = match plate {
                    1 => Color32::from_rgba_unmultiplied(255, 244, 245, 249),
                    2 => Color32::from_rgba_unmultiplied(120, 255, 255, 255),
                    _ => Color32::from_rgba_unmultiplied(130, 18, 19, 24),
                };
                painter.rect_filled(rect, CornerRadius::same(r8(pr * k)), pc);
                if plate >= 2 {
                    painter.rect_stroke(
                        rect,
                        CornerRadius::same(r8(pr * k)),
                        Stroke::new(1.0 * k, Color32::from_rgba_unmultiplied(60, 255, 255, 255)),
                        StrokeKind::Middle,
                    );
                }
                let inset = sz * 0.13;
                inner = EguiRect::from_min_size(
                    rect.min + egui::vec2(inset * k, inset * k),
                    egui::vec2((iw - 2.0 * inset) * k, (ih - 2.0 * inset) * k),
                );
            }
            let fill = if dark {
                Color32::from_rgba_unmultiplied(90, 120, 130, 160)
            } else {
                Color32::from_rgba_unmultiplied(160, 90, 100, 140)
            };
            painter.rect_filled(inner, CornerRadius::same(r8(4.0 * k)), fill);
            if let Some(ch) = it.name.chars().next() {
                painter.text(
                    inner.center(),
                    Align2::CENTER_CENTER,
                    ch.to_string(),
                    FontId::proportional((sz * 0.4).max(10.0) * k),
                    if dark {
                        Color32::from_rgba_unmultiplied(230, 240, 240, 245)
                    } else {
                        Color32::from_rgba_unmultiplied(240, 30, 30, 36)
                    },
                );
            }

            // 运行指示点：横条或圆点，贴在栏底。
            if self.cfg.dots {
                let mut ph = m.s / 13.0;
                let mut pw = if self.cfg.get_i32(Key::DotStyle) == 0 {
                    m.s * 0.32
                } else {
                    m.s / 11.0
                };
                if ph < 3.4 {
                    ph = 3.4;
                }
                if self.cfg.get_i32(Key::DotStyle) != 0 {
                    ph = pw;
                }
                if pw < 6.0 {
                    pw = 6.0;
                }
                let (dx, dy, dw, dh) = layout::map_rect(
                    self.cfg.edge,
                    lx + sz / 2.0 - pw / 2.0,
                    g.bar_bottom - 2.5 - ph,
                    pw,
                    ph,
                    m.lh as f32,
                );
                let p0 = self.to_egui(dx, dy, k, tx, ty);
                let dot = EguiRect::from_min_size(p0, egui::vec2(dw * k, dh * k));
                let rr = (dot.width().min(dot.height())) / 2.0;
                painter.rect_filled(
                    dot,
                    CornerRadius::same(r8(rr)),
                    if dark {
                        Color32::from_rgba_unmultiplied(255, 255, 255, 255)
                    } else {
                        Color32::from_rgba_unmultiplied(255, 24, 26, 34)
                    },
                );
                painter.rect_stroke(
                    dot,
                    CornerRadius::same(r8(rr)),
                    Stroke::new(
                        1.0 * k,
                        if dark {
                            Color32::from_rgba_unmultiplied(120, 0, 0, 0)
                        } else {
                            Color32::from_rgba_unmultiplied(150, 255, 255, 255)
                        },
                    ),
                    StrokeKind::Middle,
                );
            }
        }

        // 悬停气泡。
        if let Some(h) = self.hot {
            if self.cfg.labels && it_visible(self, h) {
                let sz = self.items[h].cur;
                let (ix, iy, iw, ih) = layout::map_rect(
                    self.cfg.edge,
                    g.slot_l[h],
                    g.icon_bottom - sz,
                    sz,
                    sz,
                    m.lh as f32,
                );
                self.paint_bubble(painter, &self.items[h].name, ix, iy, iw, ih, k, tx, ty);
            }
        }
    }

    /// 栏体：纯色或渐变 + 可选描边。
    fn paint_bar(&self, painter: &egui::Painter, rect: EguiRect, rad: f32, dark: bool) {
        let op = self.cfg.opacity as f32 / 100.0;
        if op <= 0.0 {
            return;
        }
        let (c1, c2) = self.bar_colors(dark);
        let (c1, c2) = (
            Color32::from_rgba_unmultiplied(c1.0 as u8, c1.1 as u8, c1.2 as u8, (op * 255.0) as u8),
            Color32::from_rgba_unmultiplied(c2.0 as u8, c2.1 as u8, c2.2 as u8, (op * 255.0) as u8),
        );
        if self.cfg.get_i32(Key::BgMode) == 0 {
            painter.rect_filled(rect, CornerRadius::same(r8(rad)), c1);
        } else {
            let dir = if self.cfg.edge.is_horizontal() {
                Direction::LeftToRight
            } else {
                Direction::TopDown
            };
            painter.add(egui::epaint::Shape::gradient_rect(rect, dir, [c1, c2]));
        }
        if self.cfg.border {
            let base = if dark { 90.0 } else { 150.0 };
            let a = (base * if op < 0.35 { op / 0.35 } else { 1.0 }) as u8;
            painter.rect_stroke(
                rect,
                CornerRadius::same(r8(rad)),
                Stroke::new(1.2, Color32::from_rgba_unmultiplied(255, 255, 255, a)),
                StrokeKind::Middle,
            );
        }
    }

    /// 栏体的两色（上游：强调色 / 自定义色 / 默认深浅色）。
    #[allow(clippy::type_complexity)]
    fn bar_colors(&self, dark: bool) -> ((f32, f32, f32), (f32, f32, f32)) {
        if self.cfg.accent_tone {
            // 强调色要向系统取，这里退回到中性色；M1 不接 DWM 取色。
            if dark {
                ((58.0, 60.0, 68.0), (30.0, 31.0, 36.0))
            } else {
                ((255.0, 255.0, 255.0), (232.0, 235.0, 243.0))
            }
        } else if self.cfg.bg_color1 >= 0 {
            let c1 = colorref_to_rgb(self.cfg.bg_color1);
            let c2 = if self.cfg.bg_color2 >= 0 {
                colorref_to_rgb(self.cfg.bg_color2)
            } else {
                (c1.0 * 0.75, c1.1 * 0.75, c1.2 * 0.75)
            };
            (c1, c2)
        } else if dark {
            ((58.0, 60.0, 68.0), (30.0, 31.0, 36.0))
        } else {
            ((255.0, 255.0, 255.0), (232.0, 235.0, 243.0))
        }
    }

    /// 悬停气泡（C 版 `DrawBubbleAt`）：宽 = 文本宽 + 24，高 = 28，圆角 9。
    #[allow(clippy::too_many_arguments)]
    fn paint_bubble(
        &self,
        painter: &egui::Painter,
        text: &str,
        ix: f32,
        iy: f32,
        iw: f32,
        ih: f32,
        k: f32,
        tx: f32,
        ty: f32,
    ) {
        let font = FontId::proportional(12.0);
        let color = Color32::from_rgba_unmultiplied(255, 245, 245, 247);
        let text_w = painter
            .layout_no_wrap(text.to_owned(), font.clone(), color)
            .size()
            .x;
        let w = text_w + 24.0 * k;
        let h = 28.0 * k;
        let (mut x, mut y) = match self.cfg.edge {
            Edge::Top => (ix + iw / 2.0 - w / 2.0, iy + ih + 8.0 * k),
            Edge::Left => (ix + iw + 10.0 * k, iy + ih / 2.0 - h / 2.0),
            Edge::Right => (ix - 10.0 * k - w, iy + ih / 2.0 - h / 2.0),
            Edge::Bottom => (ix + iw / 2.0 - w / 2.0, iy - 8.0 * k - h),
        };
        // 夹进窗口内 4 px。
        let win_w = self.metrics.lw as f32;
        let win_h = self.metrics.lh as f32;
        if x < 4.0 * k {
            x = 4.0 * k;
        }
        if x + w > win_w - 4.0 * k {
            x = win_w - 4.0 * k - w;
        }
        if y < 4.0 * k {
            y = 4.0 * k;
        }
        if y + h > win_h - 4.0 * k {
            y = win_h - 4.0 * k - h;
        }
        let p0 = self.to_egui(x, y, k, tx, ty);
        let rect = EguiRect::from_min_size(p0, egui::vec2(w, h));
        painter.rect_filled(
            rect,
            CornerRadius::same(r8(9.0 * k)),
            Color32::from_rgba_unmultiplied(228, 38, 38, 42),
        );
        painter.rect_stroke(
            rect,
            CornerRadius::same(r8(9.0 * k)),
            Stroke::new(1.0 * k, Color32::from_rgba_unmultiplied(50, 255, 255, 255)),
            StrokeKind::Middle,
        );
        painter.text(rect.center(), Align2::CENTER_CENTER, text, font, color);
    }

    /// 逻辑坐标 → 物理坐标 → 画布坐标（含隐藏位移）。
    #[allow(clippy::too_many_arguments)] // 与 C 版 MapR 的参数一一对应
    fn map(&self, u: f32, v: f32, w: f32, h: f32, k: f32, tx: f32, ty: f32) -> EguiRect {
        let (x, y, ww, hh) = layout::map_rect(self.cfg.edge, u, v, w, h, self.metrics.lh as f32);
        let p0 = self.to_egui(x, y, k, tx, ty);
        EguiRect::from_min_size(p0, egui::vec2(ww * k, hh * k))
    }

    /// 物理坐标 → 画布坐标（逻辑点），并加上隐藏位移。
    fn to_egui(&self, x: f32, y: f32, k: f32, tx: f32, ty: f32) -> Pos2 {
        Pos2::new((x + tx) * k, (y + ty) * k)
    }
}

/// egui 0.37 的 `CornerRadius` 用 `u8` 表示半径，**亚像素会丢失**。
///
/// C 版的圆角是浮点物理像素；换算到画布后不足 1 逻辑点的差异在这里被抹平，
/// 属于可接受误差（验收线是 1 px）。
fn r8(v: f32) -> u8 {
    v.round().clamp(0.0, u8::MAX as f32) as u8
}

fn it_visible(dock: &Dock, i: usize) -> bool {
    dock.items.get(i).is_some_and(|it| it.cur > 1.0)
}

/// COLORREF（`0x00BBGGRR`）→ RGB。
fn colorref_to_rgb(c: i32) -> (f32, f32, f32) {
    let c = c as u32;
    (
        (c & 0xFF) as f32,
        ((c >> 8) & 0xFF) as f32,
        ((c >> 16) & 0xFF) as f32,
    )
}
