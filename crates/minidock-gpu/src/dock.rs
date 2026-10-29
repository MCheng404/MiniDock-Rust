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
use minidock_core::color;
use minidock_core::config::{Config, Edge, Key};
use minidock_core::item::{Item, ItemKind};
use minidock_core::layout::{self, Metrics, Rect as LRect, SlotKind};

use egui::epaint::Direction;

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

    fn slot_kind(&self) -> SlotKind {
        if self.is_sep() {
            SlotKind::Separator
        } else {
            SlotKind::Icon
        }
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

    /// 窗口的**物理**像素尺寸。egui 画布尺寸 = 这个值 / ppp。
    ///
    /// 名字里带 `physical` 是有意的：`metrics.lw/lh` 与 C 版一致都是物理像素，
    /// 早期叫 `logical_size` 让人以为可以直接当 egui 尺寸用。
    #[must_use]
    pub fn physical_size(&self) -> (f32, f32) {
        (self.metrics.lw as f32, self.metrics.lh as f32)
    }

    /// 推进一帧：重算度量、放大动画、槽位与命中。
    ///
    /// `dpi` 只在度量里用来做 DPI 缩放。几何计算全部委托给
    /// [`minidock_core::layout`]，这里只负责把结果搬进 `geom`。
    pub fn update(&mut self, dpi: i32) {
        let kinds: Vec<SlotKind> = self.items.iter().map(DockItem::slot_kind).collect();
        let n_sep = kinds.iter().filter(|k| k.is_separator()).count();
        let m = layout::calc_metrics(&self.cfg, dpi, kinds.len(), n_sep, self.work);
        self.metrics = m;

        // 基准中心：算放大权重用（必须用未放大的位置，见 `map_actual_to_base`）。
        let base_c = layout::base_centers(&m, &kinds, m.clock_extra);

        // 放大动画：目标尺寸 → 指数逼近。
        let view = layout::View {
            mouse: self.mouse,
            hover: self.hover,
            hide_cur: self.hide_cur,
        };
        let targets =
            layout::magnify_targets(&m, &kinds, &base_c, &self.prev_act_c, view, self.cfg.zoom);
        let mut anim = false;
        for (it, target) in self.items.iter_mut().zip(&targets) {
            anim |= layout::lerp_to(&mut it.cur, *target);
        }

        // 实际槽位与命中（用本帧槽位，否则气泡会比图标慢一帧）。
        let cur: Vec<f32> = self.items.iter().map(|i| i.cur).collect();
        let slots = layout::place_slots(&m, &cur, m.clock_extra);
        let bars = layout::bars(&m);
        self.hot = layout::hit_test(&m, &bars, &slots, &cur, view);

        self.geom = DockGeom {
            bar_l: slots.bar_l,
            bar_w: slots.bar_w,
            bar_top: bars.top,
            bar_bottom: bars.bottom,
            icon_bottom: bars.icon_bottom,
            slot_l: slots.slot_l,
            slot_r: slots.slot_r,
            act_c: slots.act_c.clone(),
            base_c,
        };
        // 下一帧的 `map_actual_to_base` 用本帧的实际中心（C 版同序）。
        self.prev_act_c = slots.act_c;
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
    /// 顺序即层叠顺序：栏体 → 空状态提示 → 时钟卡片 → 分割线 → 图标与
    /// 运行指示点 → 悬停气泡。每一项都在自己的方法里，改一处不影响别处。
    ///
    /// * `ppp` —— 每逻辑点对应多少物理像素。
    /// * `dark` —— 深色主题。
    /// * `clock` —— 时钟卡片的（时间, 日期）文本；`None` 表示不画时钟。
    pub fn paint(&self, ui: &egui::Ui, ppp: f32, dark: bool, clock: Option<(&str, &str)>) {
        let p = self.projection(ppp);
        let painter = ui.painter();

        let bar = self.paint_bar_area(painter, p, dark);
        if self.items.is_empty() {
            painter.text(
                bar.center(),
                Align2::CENTER_CENTER,
                "将应用 / 文件夹 / 网址拖到这里",
                FontId::proportional(13.0),
                Color32::from_rgba_unmultiplied(255, 255, 255, 190),
            );
        }
        if let Some((time, date)) = clock {
            self.paint_clock_card(painter, p, dark, time, date);
        }
        self.paint_separators(painter, p, dark);
        self.paint_icons(painter, p, dark);
        self.paint_hover_bubble(painter, p);
    }

    /// 一帧的坐标换算上下文。
    ///
    /// `k` 把物理像素换成 egui 逻辑点（= 1/ppp）；`(tx, ty)` 是隐藏动画的
    /// 位移，也以物理像素计。**先加位移再乘 k**，顺序反了在 150% 缩放下
    /// 会差半屏。
    fn projection(&self, ppp: f32) -> Proj {
        let m = &self.metrics;
        let off = self.hide_cur * (m.bar_h + m.margin + 14.0);
        let (tx, ty) = match self.cfg.edge {
            Edge::Top => (0.0, -off),
            Edge::Left => (-off, 0.0),
            Edge::Right => (off, 0.0),
            Edge::Bottom => (0.0, off),
        };
        Proj {
            k: 1.0 / ppp,
            tx,
            ty,
        }
    }

    /// 栏体。返回画布上的矩形（空状态提示要用它的中心）。
    fn paint_bar_area(&self, painter: &egui::Painter, p: Proj, dark: bool) -> EguiRect {
        let m = &self.metrics;
        let bar = self.map(
            self.geom.bar_l,
            self.geom.bar_top,
            self.geom.bar_w,
            m.bar_h,
            p,
        );
        self.paint_bar(painter, bar, self.bar_radius(p), dark);
        bar
    }

    /// 栏体圆角：配置值不得大于半个栏高，否则 egui 的圆角会把矩形画成椭圆。
    fn bar_radius(&self, p: Proj) -> f32 {
        let r = (self.cfg.radius as f32).min(self.metrics.bar_h / 2.0);
        r * p.k
    }

    /// 时钟卡片：栏体 + 时间 + 日期（横向/纵向都由 `clock_w` 决定）。
    fn paint_clock_card(
        &self,
        painter: &egui::Painter,
        p: Proj,
        dark: bool,
        time: &str,
        date: &str,
    ) {
        let m = &self.metrics;
        let c = self.map(
            self.geom.bar_l + self.geom.bar_w + m.gap + 6.0,
            self.geom.bar_top,
            m.clock_w as f32,
            m.bar_h,
            p,
        );
        self.paint_bar(painter, c, self.bar_radius(p), dark);

        let (tc, dc) = if dark {
            (
                Color32::from_rgba_unmultiplied(245, 240, 240, 245),
                Color32::from_rgba_unmultiplied(190, 220, 220, 228),
            )
        } else {
            (
                Color32::from_rgba_unmultiplied(245, 30, 30, 36),
                Color32::from_rgba_unmultiplied(190, 60, 60, 70),
            )
        };
        // 上下分栏：时间占 12%–56%，日期占 54%–86%（中间重叠 2% 让两行更紧）。
        for (top, bottom, text, size, color) in
            [(0.12, 0.56, time, 14.0, tc), (0.54, 0.86, date, 11.0, dc)]
        {
            painter.text(
                EguiRect::from_min_max(
                    Pos2::new(c.left(), c.top() + c.height() * top),
                    Pos2::new(c.right(), c.top() + c.height() * bottom),
                )
                .center(),
                Align2::CENTER_CENTER,
                text,
                FontId::proportional(size),
                color,
            );
        }
    }

    /// 分割线：斜线，从图标底部往上 0.84·S。
    fn paint_separators(&self, painter: &egui::Painter, p: Proj, dark: bool) {
        let m = &self.metrics;
        let color = if dark {
            Color32::from_rgba_unmultiplied(110, 255, 255, 255)
        } else {
            Color32::from_rgba_unmultiplied(120, 60, 60, 70)
        };
        for (i, it) in self.items.iter().enumerate() {
            // `cur < 2` 表示这条分割线还在长出/消失，跳过以免画出 1 px 的毛刺。
            if !it.is_sep() || it.cur < 2.0 {
                continue;
            }
            let (lx, ly, lw, lh) = layout::map_rect(
                self.cfg.edge,
                self.geom.act_c[i] + it.slide,
                self.geom.icon_bottom - m.s * 0.88,
                0.0,
                m.s * 0.84,
                m.lh as f32,
            );
            let a = self.to_egui(lx, ly, p);
            let b = self.to_egui(lx + lw, ly + lh, p);
            painter.line_segment([a, b], Stroke::new(1.4 * p.k, color));
        }
    }

    /// 图标（M1 用"圆角底盘 + 首字"占位）与运行指示点。
    fn paint_icons(&self, painter: &egui::Painter, p: Proj, dark: bool) {
        for (i, it) in self.items.iter().enumerate() {
            if it.is_sep() || it.cur < 1.0 {
                continue;
            }
            let sz = it.cur;
            let lx = self.geom.slot_l[i] + it.slide;
            self.paint_icon(painter, p, dark, it, lx, sz);
            if self.cfg.dots {
                self.paint_running_dot(painter, p, dark, lx, sz);
            }
        }
    }

    /// 单个图标：可选底盘 + 首字占位。
    fn paint_icon(
        &self,
        painter: &egui::Painter,
        p: Proj,
        dark: bool,
        it: &DockItem,
        lx: f32,
        sz: f32,
    ) {
        let m = &self.metrics;
        let (ix, iy, iw, ih) = layout::map_rect(
            self.cfg.edge,
            lx,
            self.geom.icon_bottom - sz,
            sz,
            sz,
            m.lh as f32,
        );
        let rect = EguiRect::from_min_size(self.to_egui(ix, iy, p), egui::vec2(iw * p.k, ih * p.k));

        // 底盘（macOS 风格的圆角方块）。0 = 不画，1 = 纯色，2 = 半透明 + 描边。
        let plate = self.cfg.get_i32(Key::PLATE_MODE);
        let mut inner = rect;
        if plate > 0 {
            let pr = (sz * self.cfg.get_i32(Key::PlateRadius) as f32 / 100.0).min(sz / 2.0);
            let radius = CornerRadius::same(r8(pr * p.k));
            let color = match plate {
                1 => Color32::from_rgba_unmultiplied(255, 244, 245, 249),
                2 => Color32::from_rgba_unmultiplied(120, 255, 255, 255),
                _ => Color32::from_rgba_unmultiplied(130, 18, 19, 24),
            };
            painter.rect_filled(rect, radius, color);
            if plate >= 2 {
                painter.rect_stroke(
                    rect,
                    radius,
                    Stroke::new(p.k, Color32::from_rgba_unmultiplied(60, 255, 255, 255)),
                    StrokeKind::Middle,
                );
            }
            // 图标缩进 13%，让底盘露出来一圈。
            let inset = sz * 0.13;
            inner = EguiRect::from_min_size(
                rect.min + egui::vec2(inset * p.k, inset * p.k),
                egui::vec2((iw - 2.0 * inset) * p.k, (ih - 2.0 * inset) * p.k),
            );
        }

        let fill = if dark {
            Color32::from_rgba_unmultiplied(90, 120, 130, 160)
        } else {
            Color32::from_rgba_unmultiplied(160, 90, 100, 140)
        };
        painter.rect_filled(inner, CornerRadius::same(r8(4.0 * p.k)), fill);
        if let Some(ch) = it.name.chars().next() {
            let color = if dark {
                Color32::from_rgba_unmultiplied(230, 240, 240, 245)
            } else {
                Color32::from_rgba_unmultiplied(240, 30, 30, 36)
            };
            painter.text(
                inner.center(),
                Align2::CENTER_CENTER,
                ch.to_string(),
                FontId::proportional((sz * 0.4).max(10.0) * p.k),
                color,
            );
        }
    }

    /// 运行指示点：横条或圆点，贴在栏底。
    fn paint_running_dot(&self, painter: &egui::Painter, p: Proj, dark: bool, lx: f32, sz: f32) {
        let m = &self.metrics;
        let round = self.cfg.get_i32(Key::DotStyle) != 0;
        // 横条：宽 0.32·S、高 S/13（下限 3.4）；圆点：直径 S/11（下限 6）。
        let mut pw = if round { m.s / 11.0 } else { m.s * 0.32 };
        let mut ph = m.s / 13.0;
        if ph < 3.4 {
            ph = 3.4;
        }
        if round {
            ph = pw;
        }
        if pw < 6.0 {
            pw = 6.0;
        }

        let (dx, dy, dw, dh) = layout::map_rect(
            self.cfg.edge,
            lx + sz / 2.0 - pw / 2.0,
            self.geom.bar_bottom - 2.5 - ph,
            pw,
            ph,
            m.lh as f32,
        );
        let dot = EguiRect::from_min_size(self.to_egui(dx, dy, p), egui::vec2(dw * p.k, dh * p.k));
        let radius = CornerRadius::same(r8((dot.width().min(dot.height())) / 2.0));
        let (fill, stroke) = if dark {
            (
                Color32::from_rgba_unmultiplied(255, 255, 255, 255),
                Color32::from_rgba_unmultiplied(120, 0, 0, 0),
            )
        } else {
            (
                Color32::from_rgba_unmultiplied(255, 24, 26, 34),
                Color32::from_rgba_unmultiplied(150, 255, 255, 255),
            )
        };
        painter.rect_filled(dot, radius, fill);
        painter.rect_stroke(dot, radius, Stroke::new(p.k, stroke), StrokeKind::Middle);
    }

    /// 悬停气泡。只在标签开启且图标已长出来时画。
    fn paint_hover_bubble(&self, painter: &egui::Painter, p: Proj) {
        let Some(hot) = self.hot else { return };
        if !self.cfg.labels || !it_visible(self, hot) {
            return;
        }
        self.paint_bubble(painter, hot, p);
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

    /// 栏体的两色。优先级：强调色 → 自定义色 → 主题默认。
    ///
    /// 强调色要向 DWM 取真实色相，M1 还没接，所以它暂时与默认深浅色同值 ——
    /// 两个分支合并成一条，避免"改了一处忘了另一处"。
    fn bar_colors(&self, dark: bool) -> ((f32, f32, f32), (f32, f32, f32)) {
        if !self.cfg.accent_tone && self.cfg.bg_color1 >= 0 {
            let c1 = color::from_colorref(self.cfg.bg_color1 as u32);
            let c2 = if self.cfg.bg_color2 >= 0 {
                color::from_colorref(self.cfg.bg_color2 as u32)
            } else {
                // 未指定终点：把起点压暗 25%，保持"渐变"的观感。
                color::mix(c1, color::rgb(0, 0, 0), 0.25)
            };
            return (to_rgb(c1), to_rgb(c2));
        }
        if dark {
            ((58.0, 60.0, 68.0), (30.0, 31.0, 36.0))
        } else {
            ((255.0, 255.0, 255.0), (232.0, 235.0, 243.0))
        }
    }

    /// 悬停气泡（C 版 `DrawBubbleAt`）：宽 = 文本宽 + 24，高 = 28，圆角 9。
    ///
    /// 位置由气泡**自己**按图标槽位算：气泡要贴在被悬停图标的哪一侧、
    /// 以及夹进窗口内的规则，都与停靠边有关，放在这里比让调用方拼参数清楚。
    fn paint_bubble(&self, painter: &egui::Painter, index: usize, p: Proj) {
        let k = p.k;
        let m = &self.metrics;
        let it = &self.items[index];
        let sz = it.cur;
        let text = it.name.as_str();
        let (ix, iy, iw, ih) = layout::map_rect(
            self.cfg.edge,
            self.geom.slot_l[index],
            self.geom.icon_bottom - sz,
            sz,
            sz,
            m.lh as f32,
        );
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
        let win_w = m.lw as f32;
        let win_h = m.lh as f32;
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
        let p0 = self.to_egui(x, y, p);
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
    fn map(&self, u: f32, v: f32, w: f32, h: f32, p: Proj) -> EguiRect {
        let (x, y, ww, hh) = layout::map_rect(self.cfg.edge, u, v, w, h, self.metrics.lh as f32);
        EguiRect::from_min_size(self.to_egui(x, y, p), egui::vec2(ww * p.k, hh * p.k))
    }

    /// 物理坐标 → 画布坐标（逻辑点），并加上隐藏位移。
    fn to_egui(&self, x: f32, y: f32, p: Proj) -> Pos2 {
        Pos2::new((x + p.tx) * p.k, (y + p.ty) * p.k)
    }
}

/// 一帧的坐标换算上下文（见 [`Dock::projection`]）。
#[derive(Copy, Clone, Debug)]
struct Proj {
    /// 物理像素 → egui 逻辑点。
    k: f32,
    /// 隐藏动画的横向位移（物理像素）。
    tx: f32,
    /// 隐藏动画的纵向位移（物理像素）。
    ty: f32,
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

/// `0xAARRGGBB` → 各通道 `f32`（绘制时要用浮点做混合）。
fn to_rgb(c: color::Argb) -> (f32, f32, f32) {
    (
        color::red(c) as f32,
        color::green(c) as f32,
        color::blue(c) as f32,
    )
}
