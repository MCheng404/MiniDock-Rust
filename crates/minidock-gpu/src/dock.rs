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
use std::time::Instant;

use minidock_core::clock::ClockLine;
use minidock_core::color;
use minidock_core::config::{Config, DockMode, Edge, Key};
use minidock_core::icon::{apply_round_mask, fit_icon, BBox, FitMode};
use minidock_core::item::{Item, ItemKind};
use minidock_core::layout::{self, Metrics, Rect as LRect, SlotKind};

use egui::epaint::Direction;

/// 一个已上传到 GPU 的图标。
///
/// **不预先缩放**：Dock 图标在放大动画里每帧尺寸都不同，CPU 逐像素插值
/// 既慢又会糊。这里保留原始位图（256 px），只记下该采样哪一块（`bbox`），
/// 缩放交给 GPU 的双线性过滤。
#[derive(Clone)]
pub struct IconTex {
    tex: egui::TextureHandle,
    /// 源图尺寸，用来把 `bbox` 换算成 UV。
    src_w: u32,
    src_h: u32,
    bbox: BBox,
    /// 内容在槽位里的填充比例（原版 0.94 / 1.0）。
    fill: f32,
}

impl std::fmt::Debug for IconTex {
    // `TextureHandle` 没有 `Debug`，但它内部只是一个 id + 尺寸；
    // 手写一份让 `Dock` 之类的外层结构仍能 `{:?}` 打印。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IconTex")
            .field("src", &format_args!("{}x{}", self.src_w, self.src_h))
            .field("bbox", &self.bbox)
            .field("fill", &self.fill)
            .finish_non_exhaustive()
    }
}

impl IconTex {
    /// 采样区域在纹理里的归一化 UV。
    fn uv(&self) -> EguiRect {
        let (w, h) = (self.src_w as f32, self.src_h as f32);
        EguiRect::from_min_max(
            Pos2::new(self.bbox.x0 as f32 / w, self.bbox.y0 as f32 / h),
            Pos2::new(self.bbox.x1 as f32 / w, self.bbox.y1 as f32 / h),
        )
    }

    /// 采样区域的长宽比（宽的占比）。
    fn aspect(&self) -> (f32, f32) {
        let bw = self.bbox.width() as f32;
        let bh = self.bbox.height() as f32;
        let mx = bw.max(bh).max(1.0);
        (bw / mx, bh / mx)
    }
}

/// 拖拽重排的运行时状态。
///
/// 与原版对应的全局量：`pressIdx` / `pressPt` / `dragging` / `dragIdx` / `dragPtA`。
/// 收进一个结构体是因为它们必须同时成立 —— 分散成四个字段时很容易
/// 出现"`dragging` 为真但 `dragIdx` 还是 -1"这种半截状态。
#[derive(Clone, Debug, PartialEq)]
pub struct DragState {
    /// 被拖条目在 `items` 里的**当前**下标。重排会同步更新它。
    pub index: usize,
    /// 鼠标当前位置（物理像素，客户区）。
    pub pt: (f32, f32),
    /// 是否已越过起拖阈值。
    ///
    /// 按下时还不算拖拽：必须移动超过 6 px 才进入拖拽，
    /// 否则每次点击都会先抖动一下。
    pub active: bool,
    /// 按下点（物理像素），用于阈值判断。
    press_pt: (f32, f32),
}

/// 起拖阈值（物理像素）。与 C 版 `abs(x - pressPt.x) > 6` 一致。
const DRAG_THRESHOLD: f32 = 6.0;

/// 一条 Dock 条目的运行时状态。
#[derive(Clone, Debug)]
pub struct DockItem {
    /// 显示名（气泡里用）。
    pub name: String,
    pub kind: ItemKind,
    /// 当前边长。放大动画的值，对应 C 版 `items[i].cur`。
    pub cur: f32,
    /// 拖拽滑动的偏移，对应 C 版 `items[i].slide`。
    pub slide: f32,
    /// 该条目对应的应用当前是否在运行（决定要不要画运行指示点）。
    ///
    /// 由主循环周期性扫描后写入。之前没有这个字段，指示点是**无条件绘制**的
    /// —— 视觉上表现为所有图标都"亮着"，用户无法分辨哪个真在运行。
    pub running: bool,
    /// 启动弹跳的起始时刻。`None` = 不在弹跳。
    ///
    /// 用 `Instant` 而不是毫秒计数：弹跳是纯视觉动画，与其他动画共用
    /// 同一时钟最省心，也不需要主循环额外传时间进来。
    pub bounce_at: Option<Instant>,
    /// 上一帧的槽位左边界，用于算 `slide` 的初值。
    ///
    /// 重排时先把每个条目的当前位置记下来，移动后
    /// `slide += prev_l - 新 slot_l` —— 于是图标会从原位"滑"到新位，
    /// 而不是瞬移。这是原版 `items[i].prevL` 的同一套做法。
    pub prev_l: f32,
}

impl DockItem {
    /// 当前弹跳偏移（逻辑像素）。`0` = 不动。
    ///
    /// 抽成方法而不是在绘制处手算：`paint_icons` 与
    /// `paint_drag_ghost` 都要用，两处各写一遍迟早不一致。
    #[must_use]
    pub fn bounce_offset(&self, s: f32) -> f32 {
        let Some(t) = self.bounce_at else {
            return 0.0;
        };
        layout::bounce_offset(
            u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX),
            s,
        )
    }

    /// 从配置条目构造。`cur` 初始为 0 —— C 版也是 0，靠动画长出来。
    #[must_use]
    pub fn from_item(item: &Item) -> Self {
        Self {
            name: item.name.clone(),
            kind: item.kind,
            cur: 0.0,
            slide: 0.0,
            running: item.running,
            bounce_at: None,
            prev_l: 0.0,
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
    /// 鼠标在窗口内的位置，**物理像素**（与 `metrics` / `geom` 同一套单位）。
    ///
    /// 早期这里存的是逻辑点，而槽位几何是物理像素 —— 在 150% DPI 下
    /// 命中会整体偏移 1.5 倍，表现为"鼠标在图标右边才放大"。
    /// 统一成物理像素后，只在绘制时经 `Proj::k` 换算一次。
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
    /// 按条目下标存放的图标纹理。`None` = 该条目没有图标（画首字占位）。
    icons: Vec<Option<IconTex>>,
    /// 输入事件把这里标脏，主循环据此决定是否重新布局。
    ///
    /// 存在的理由：主循环原来只在 `anim` 为真时才调 `update`，
    /// 而 `hot` 只在 `update` 里算 —— 动画一旦收敛，鼠标再进来也不会放大。
    dirty: bool,
    /// 左键按下时命中的条目下标（拖拽判定与"按下抬起同一图标"都靠它）。
    press: Option<usize>,
    /// 显示模式。`Always` 常显、`AutoHide` 离开即收起、`SmartHide` 被遮挡才收起。
    ///
    /// 曾经单独存过一个 `auto_hide: bool`，但它表达不了"智能隐藏"：
    /// 两个 bool 会组合出 `auto_hide=false` + `smart=true` 这种不存在的状态。
    pub mode: DockMode,
    /// 当前是否处于隐藏态（目标值，不是动画中间值）。
    pub hide_target: bool,
    /// 用户用快捷键强制收起（`Ctrl+Alt+D` 切换）。
    ///
    /// 与 `hide_target` 分开是因为语义不同：`hide_target` 是自动隐藏算出来的，
    /// `force_hide` 是用户手动的，**优先级更高且不受鼠标位置影响**。
    /// C 版用 `ht = gForceHide ? 1 : gHideTgt` 表达同一件事。
    pub force_hide: bool,
    /// 最近一次"鼠标靠近 / 正在操作"的时刻（毫秒，单调时钟）。
    last_near_ms: u64,
    /// 当前拖拽状态。`None` = 没有按住任何条目。
    drag: Option<DragState>,
    /// 本帧发生过重排，`update` 据此补位移动画。
    moved: bool,
    /// 条目顺序变过，主循环要写回 ini。
    order_changed: bool,
    /// 最近一次重排（`from` → `to`），主循环取走后对配置副本施加同样操作。
    ///
    /// `Dock` 只持有 `DockItem`（名称 + 类型），启动所需的完整 `Item`
    /// 在主循环那边。两边各自维护一份列表时，重排必须同步，
    /// 否则"点了图标启动的是别人"。用这个字段把变更单向下发。
    pending_move: Option<(usize, usize)>,
    /// 待处理的点击 `(下标, 是否强制新实例)`。
    ///
    /// 存下标而不是 bool：主循环要启动**被点的那个**条目，
    /// 用 `hot` 会在"注入鼠标/消息乱序"时启动错项。
    ///
    /// 带 `force` 是因为 Shift+点击 与中键的语义是"不管是否已在运行，
    /// 都新起一个实例"（原版 `LaunchItem(i, force)`）。把两种点击
    /// 收进同一条路径，主循环只需处理一次。
    click: Option<(usize, bool)>,
    /// 待处理的右键请求（客户区物理坐标）。主循环取走后弹菜单。
    ///
    /// 不在窗口过程里直接弹：菜单是**模态**的（自己跑消息循环），
    /// 在 `WndProc` 里阻塞会让这个窗口的消息队列停摆。
    menu_at: Option<(f32, f32)>,
    /// 本次按下是否按着 Shift / 来自中键。抬起时据此决定 `click` 的 force。
    press_force: bool,
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
            icons: Vec::new(),
            dirty: true,
            mode: DockMode::Always,
            hide_target: false,
            force_hide: false,
            last_near_ms: 0,
            drag: None,
            moved: false,
            order_changed: false,
            pending_move: None,
            press: None,
            click: None,
            press_force: false,
            menu_at: None,
        }
    }

    /// 标脏：下一次循环会重新布局（重算 `hot`）。
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// 取走脏标记。
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// 在给定**物理像素**坐标上做命中测试，用上一帧的几何。
    ///
    /// 为什么不直接用 `hot`：按下与移动可能在同一轮消息里到达，
    /// 那时 `hot` 还停在上上帧的位置。原版也是在按下时重新 `HitIndex`。
    #[must_use]
    pub fn hit_at(&self, u: f32, v: f32) -> Option<usize> {
        let visible: Vec<bool> = self.items.iter().map(|i| i.cur > 1.0).collect();
        layout::hit_index(
            u,
            v,
            &self.geom.slot_l,
            &self.geom.slot_r,
            &visible,
            self.metrics.gap,
            self.geom.bar_top,
            self.metrics.z,
            self.metrics.lh as f32,
        )
    }

    /// 左键按下：记下命中项，并准备拖拽。返回是否需要 `SetCapture`。
    pub fn press_at(&mut self, u: f32, v: f32, force: bool) -> bool {
        self.press = self.hit_at(u, v);
        self.press_force = force;
        self.drag = self.press.map(|index| DragState {
            index,
            pt: (u, v),
            // 按下这一刻还不算拖拽，要移动超过阈值才算。
            active: false,
            press_pt: (u, v),
        });
        self.press.is_some()
    }

    /// 拖动中：更新位置，必要时重排。
    ///
    /// 阈值（6 px）是原版行为：没有它，手指/鼠标的微小抖动会让每次点击
    /// 都先"拖"一下再松手，观感很差。
    pub fn drag_to(&mut self, u: f32, v: f32) {
        let Some(mut d) = self.drag.take() else {
            return;
        };
        d.pt = (u, v);
        if !d.active {
            let (dx, dy) = (u - d.press_pt.0, v - d.press_pt.1);
            // 锁定状态下一律不允许拖动（原版 `!cfg.lock`）。
            if !self.cfg.lock && dx.abs().max(dy.abs()) > DRAG_THRESHOLD {
                d.active = true;
            }
        }
        if d.active {
            // 必须把自己排除掉：`drag_to` 开头已经 `take()` 了状态，
            // `place_for` 里读 `self.drag` 只会拿到 `None`，于是把被拖项
            // 也算进"左边有几个"，每次都会多挪一格。
            let target = self.place_for(u, d.index);
            if target != d.index {
                self.move_item(d.index, target);
                d.index = target;
                // 同步 `press`，否则抬起时"同一图标"的判断会失配。
                self.press = Some(target);
            }
        }
        self.drag = Some(d);
    }

    /// 拖到 `u` 时应插入的下标。
    ///
    /// 原版是"数一数有多少条目的实际中心在光标左边"（不含被拖的那条）。
    /// 这直接给出**移除被拖项之后**的插入位置，正好与
    /// [`Self::move_item`] 的"取出再插入"语义对齐。
    fn place_for(&self, u: f32, dragged: usize) -> usize {
        self.geom
            .act_c
            .iter()
            .enumerate()
            .filter(|(i, c)| *i != dragged && **c < u)
            .count()
    }

    /// 把第 `from` 项移到 `to`。图标纹理同步搬动，否则会"图标跟着位置跑"。
    fn move_item(&mut self, from: usize, to: usize) {
        if from == to || from >= self.items.len() || to >= self.items.len() {
            return;
        }
        // 先把每个条目的当前位置记进 `prev_l`：移动后 `update` 用它算位移。
        for (it, &l) in self.items.iter_mut().zip(&self.geom.slot_l) {
            it.prev_l = l;
        }
        let item = self.items.remove(from);
        self.items.insert(to, item);
        if from < self.icons.len() && to < self.icons.len() {
            let icon = self.icons.remove(from);
            self.icons.insert(to, icon);
        }
        self.moved = true;
        self.order_changed = true;
        self.pending_move = Some((from, to));
    }

    /// 取走"顺序变过"的标记。
    pub fn take_order_changed(&mut self) -> bool {
        std::mem::take(&mut self.order_changed)
    }

    /// 取走最近一次重排（主循环用它同步自己的配置副本）。
    pub fn take_pending_move(&mut self) -> Option<(usize, usize)> {
        self.pending_move.take()
    }

    /// 是否正在拖拽（绘制时据此画"跟手"的图标）。
    #[must_use]
    pub fn dragging(&self) -> Option<&DragState> {
        self.drag.as_ref().filter(|d| d.active)
    }

    /// 左键抬起：只有"按下与抬起落在同一个条目上"才算点击。
    ///
    /// 这条规则来自原版：`if (HitIndex(u, v) == pi) LaunchItem(pi, ...)`。
    /// 少了它，从图标上按下、拖到别处松手也会误启动。
    pub fn release_at(&mut self, u: f32, v: f32) -> Option<usize> {
        // 拖过就不算点击：这条必须先判断，否则每次重排都会顺带启动一次应用。
        let was_dragging = self.drag.take().is_some_and(|d| d.active);
        let pressed = self.press.take();
        if was_dragging {
            return None;
        }
        let up = self.hit_at(u, v);
        let hit = match (pressed, up) {
            (Some(a), Some(b)) if a == b => Some((a, self.press_force)),
            _ => None,
        };
        self.click = hit;
        hit.map(|(i, _)| i)
    }

    /// 取消正在进行的按下/拖拽，**不**产出点击。
    ///
    /// `WM_CAPTURECHANGED` 时调用：鼠标捕获被别人抢走（弹出 UAC、
    /// Alt+Tab、另一个程序 `SetCapture`）意味着抬起事件永远不会来。
    /// 不清掉 `press` 的话，下一次点击会带着上一次的"按下"状态，
    /// 表现为"点一下启动了上一个图标"。
    pub fn cancel_drag(&mut self) {
        let was = self.drag.take().is_some_and(|d| d.active);
        self.press = None;
        self.click = None;
        if was {
            // 拖拽被中断：图标回弹到原位，不能留在半路。
            self.mark_dirty();
            self.anim = true;
        }
    }

    /// 中键：直接产出一个"强制新实例"的点击。
    ///
    /// 中键没有"按下-抬起配对"的语义，但走同一条 `click` 通道能让主循环
    /// 只处理一次；顺手清掉按下状态，避免中键之后残留一个拖拽候选。
    pub fn force_click(&mut self, u: f32, v: f32) -> Option<usize> {
        let hit = self.hit_at(u, v);
        if hit.is_some() {
            self.press = None;
            self.drag = None;
            self.click = hit.map(|i| (i, true));
        }
        hit
    }

    /// 记住一次右键，等主循环来取。
    ///
    /// 顺带清掉按下/拖拽状态：右键之前若按着左键（比如拖到一半又点右键），
    /// 那个按下状态不该延续到菜单关闭之后。
    pub fn request_menu(&mut self, u: f32, v: f32) {
        self.cancel_drag();
        self.menu_at = Some((u, v));
        self.mark_dirty();
    }

    /// 取走右键请求。
    pub fn take_menu_request(&mut self) -> Option<(f32, f32)> {
        self.menu_at.take()
    }

    /// 切换"快捷键强制收起"。返回切换后的状态（true = 已收起）。
    pub fn toggle_force_hide(&mut self) -> bool {
        self.force_hide = !self.force_hide;
        if self.force_hide {
            // 收起时清掉悬停，否则图标会停在放大状态一起滑走。
            self.hover = false;
            self.mouse = (-1.0, -1.0);
        }
        self.mark_dirty();
        self.anim = true;
        self.force_hide
    }

    /// 让某个条目弹一下（启动应用后的反馈）。
    ///
    /// 对应 C 版 `if (cfg.bounce) it->bounce = GetTickCount() | 1;`。
    /// `Bounce` 关掉时直接不弹 —— 配置项存在却无效是最糟的。
    pub fn start_bounce(&mut self, index: usize) {
        if !self.cfg.bounce {
            return;
        }
        if let Some(it) = self.items.get_mut(index) {
            it.bounce_at = Some(Instant::now());
            self.anim = true;
            self.mark_dirty();
        }
    }

    /// 推进弹跳：清掉过期的，有活跃的就继续出帧。
    fn tick_bounce(&mut self) -> bool {
        let mut active = false;
        for it in &mut self.items {
            if let Some(t) = it.bounce_at {
                let ms = u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX);
                if layout::bounce_active(ms) {
                    active = true;
                } else {
                    it.bounce_at = None;
                }
            }
        }
        active
    }

    /// 取走待处理的点击 `(下标, 是否强制新实例)`（主循环用）。
    pub fn take_click(&mut self) -> Option<(usize, bool)> {
        self.click.take()
    }

    /// 按条目下标装载一个图标（`mode` 来自条目的 `fit` 字段）。
    ///
    /// 圆角遮罩在这里做一次：半径按**源像素**给，缩放后等比，
    /// 所以放大动画的任意尺寸下圆角比例都一致。
    ///
    /// 索引越界会被忽略 —— 调用方通常是"遍历条目 + 逐个装图标"，
    /// 中途换配置导致数量不一致时不该崩。
    pub fn load_icon(
        &mut self,
        index: usize,
        ctx: &egui::Context,
        bmp: &minidock_platform::icon::IconBitmap,
        mode: FitMode,
    ) {
        if index >= self.items.len() {
            return;
        }
        let fit = fit_icon(&bmp.rgba, bmp.width, bmp.height, mode);
        let mut rgba = bmp.rgba.clone();
        if let Some(r) = fit.rounded {
            apply_round_mask(&mut rgba, bmp.width, fit.bbox, r);
        }

        let size = [bmp.width as usize, bmp.height as usize];
        let image = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
        let tex = ctx.load_texture(
            format!("dock-icon-{index}"),
            image,
            // 线性过滤：图标会被缩放，最近邻会出锯齿。
            egui::TextureOptions::LINEAR,
        );

        if self.icons.len() < self.items.len() {
            self.icons.resize(self.items.len(), None);
        }
        self.icons[index] = Some(IconTex {
            tex,
            src_w: bmp.width,
            src_h: bmp.height,
            bbox: fit.bbox,
            fill: fit.fill,
        });
    }

    /// 某个条目是否已经有图标（诊断用）。
    #[must_use]
    pub fn has_icon(&self, index: usize) -> bool {
        self.icons.get(index).is_some_and(Option::is_some)
    }

    /// 窗口的**物理**像素尺寸。egui 画布尺寸 = 这个值 / ppp。
    ///
    /// 名字里带 `physical` 是有意的：`metrics.lw/lh` 与 C 版一致都是物理像素，
    /// 早期叫 `logical_size` 让人以为可以直接当 egui 尺寸用。
    #[must_use]
    pub fn physical_size(&self) -> (f32, f32) {
        // 竖排要转置：`lw/lh` 是按横排算出来的长与高，
        // 左/右停靠时窗口宽=lh、高=lw。直接返回会让竖排 Dock 变成一条细缝。
        let (w, h) = layout::physical_size(self.cfg.edge, self.metrics.lw, self.metrics.lh);
        (w as f32, h as f32)
    }

    /// 同上，但取整成窗口坐标用的 `i32`。
    #[must_use]
    pub fn physical_size_i32(&self) -> (i32, i32) {
        let (w, h) = self.physical_size();
        (w.round() as i32, h.round() as i32)
    }

    /// AppBar 需要预留的厚度（物理像素）。
    #[must_use]
    pub fn reserve_thickness(&self) -> i32 {
        layout::reserve_thickness(self.metrics.bar_h, self.metrics.margin)
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

        // 本帧是否有过重排（决定要不要补位移动画）。
        let moved = std::mem::take(&mut self.moved);

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
            slot_l: slots.slot_l.clone(),
            slot_r: slots.slot_r,
            act_c: slots.act_c.clone(),
            base_c,
        };
        // 重排后的位移：每项从"上一帧的位置"滑到"本帧的位置"。
        // 顺序与原版一致 —— 先按当前位置差补上 slide，再衰减一次。
        if moved {
            for (it, &l) in self.items.iter_mut().zip(&slots.slot_l) {
                it.slide += it.prev_l - l;
            }
        }
        for it in &mut self.items {
            if it.slide.abs() > 0.5 {
                it.slide *= 0.62;
                anim = true;
            } else {
                it.slide = 0.0;
            }
        }

        // 弹跳期间必须继续出帧，否则图标会停在半空。
        if self.tick_bounce() {
            anim = true;
        }

        // 下一帧的 `map_actual_to_base` 用本帧的实际中心（C 版同序）。
        self.prev_act_c = slots.act_c;
        self.anim = anim;
    }

    /// 鼠标离开后多久才隐藏（毫秒）。原版是 450。
    ///
    /// 给宽限期是必要的：鼠标划过屏幕底边去点任务栏时不该把 Dock 弹出来，
    /// 反过来刚离开 Dock 立刻缩回去也很烦躁。
    pub const HIDE_GRACE_MS: u64 = 450;

    /// 运行状态扫描周期（毫秒）。与 C 版 `T_SCAN` 的 1200 ms 一致。
    ///
    /// 更短没有意义：用户点开一个程序的感知延迟远大于这个量级，
    /// 而每次扫描要 `EnumWindows` + 为每个窗口 `OpenProcess`。
    pub const SCAN_INTERVAL_MS: u32 = 1200;

    /// 自动隐藏的每 tick 决策。
    ///
    /// 与原版 `OnAutoHide()` 的分工一致：**"光标算不算靠近"由调用方判断**
    /// （需要屏幕坐标与窗口位置，那是 Win32 的事），这里只负责
    /// "靠近就显示、离开超过宽限期就收起"以及动画推进。
    ///
    /// `busy` 是"正在操作 Dock"（拖拽中 / 菜单打开）。这类状态下一律保持
    /// 显示 —— 拖到一半 Dock 缩回去是最糟糕的体验。
    ///
    /// 返回是否仍需继续出帧。
    /// 自动隐藏是否启用（两种非 `Always` 都要）。
    #[must_use]
    pub fn auto_hide(&self) -> bool {
        self.mode != DockMode::Always
    }

    /// 主循环是否需要继续调 `tick_auto_hide`。
    ///
    /// **不能**只判断 `auto_hide() || force_hide`：快捷键强制收起后再按一次
    /// 恢复，两个标志都变 false，但 `hide_cur` 还停在 1 —— 于是动画再也不会
    /// 被推进，Dock **永久消失**，只能重启进程。
    /// （这就是实现快捷键时踩到的实际 bug。）
    #[must_use]
    pub fn needs_hide_tick(&self) -> bool {
        self.auto_hide() || self.force_hide || self.hide_cur > 0.0
    }

    /// 推进自动隐藏。返回 true 表示这一帧需要出帧（动画或状态刚变化）。
    ///
    /// `covered` 只在 `SmartHide` 下有意义：Dock 区域是否被普通窗口盖住。
    /// 没人盖住时强制显示 —— 这是"智能隐藏"与"自动隐藏"的唯一区别，
    /// 所以它必须走同一条宽限期逻辑，而不是另开一套。
    pub fn tick_auto_hide(&mut self, near: bool, busy: bool, covered: bool, now_ms: u64) -> bool {
        // 快捷键强制收起**优先于一切**：不受模式、鼠标位置、遮挡影响。
        // 常显模式下也要能用 —— 否则"快速让开"的快捷键就失效了。
        if self.force_hide {
            return self.tick_hide(true);
        }
        if self.mode == DockMode::Always {
            // 不能直接 return false：强制收起解除后还要把 `hide_cur` 滑回 0，
            // 那段时间必须继续出帧。
            return self.tick_hide(false);
        }
        // 智能隐藏：没被遮挡就当作"鼠标靠近"，强制展开并刷新计时。
        let forced = self.mode == DockMode::SmartHide && !covered;
        if near || busy || forced {
            self.last_near_ms = now_ms;
            if self.hide_target {
                self.hide_target = false;
                self.mark_dirty();
            }
        } else if !self.hide_target
            && now_ms.saturating_sub(self.last_near_ms) > Self::HIDE_GRACE_MS
        {
            // 收起时顺手清掉悬停：否则图标会停在放大状态一起滑下去。
            self.hide_target = true;
            self.hover = false;
            self.mouse = (-1.0, -1.0);
            self.mark_dirty();
        }
        self.tick_hide(self.hide_target)
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
    pub fn paint(&self, ui: &egui::Ui, ppp: f32, dark: bool, clock: Option<&[ClockLine]>) {
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
        if let Some(lines) = clock {
            if !lines.is_empty() {
                self.paint_clock_card(painter, p, dark, lines);
            }
        }
        self.paint_separators(painter, p, dark);
        self.paint_icons(painter, p, dark);
        // 残影必须画在所有槽位内容之后，否则会被后面画的图标盖住。
        self.paint_drag_ghost(painter, p, dark);
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
    fn paint_clock_card(&self, painter: &egui::Painter, p: Proj, dark: bool, lines: &[ClockLine]) {
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
        // 卡片高度按**行数均分**（原版 `lh = (ch0 - 8) / nl`）。
        // 写死两行会出错：只显示时间或只显示日期时只有一行。
        let n = lines.len().max(1);
        let line_h = (c.height() - 8.0 * p.k) / n as f32;
        let mut prev: Option<&ClockLine> = None;
        for (k, line) in lines.iter().enumerate() {
            let (size, color) = if line.is_date { (11.0, dc) } else { (14.0, tc) };
            painter.text(
                EguiRect::from_min_max(
                    Pos2::new(c.left(), c.top() + 4.0 * p.k + line_h * k as f32),
                    Pos2::new(c.right(), c.top() + 4.0 * p.k + line_h * (k + 1) as f32),
                )
                .center(),
                Align2::CENTER_CENTER,
                &line.text,
                FontId::proportional(size * p.k),
                color,
            );
            // 时间组与日期组之间一条细线（原版同款）。
            if let Some(pv) = prev {
                if pv.is_date != line.is_date {
                    let y = c.top() + 4.0 * p.k + line_h * k as f32;
                    let ink = if dark {
                        argbc(90, 255, 255, 255)
                    } else {
                        argbc(95, 60, 60, 70)
                    };
                    painter.line_segment(
                        [
                            Pos2::new(c.left() + c.width() * 0.14, y),
                            Pos2::new(c.left() + c.width() * 0.86, y),
                        ],
                        Stroke::new(p.k, ink),
                    );
                }
            }
            prev = Some(line);
        }
    }

    /// 分割线：斜线，从图标底部往上 0.84·S。
    fn paint_separators(&self, painter: &egui::Painter, p: Proj, dark: bool) {
        let m = &self.metrics;
        let color = if dark {
            argbc(110, 255, 255, 255)
        } else {
            argbc(120, 60, 60, 70)
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
        // 被拖的条目不在槽位里画：它跟着光标走，见 `paint_drag_ghost`。
        let dragged = self.dragging().map(|d| d.index);
        for (i, it) in self.items.iter().enumerate() {
            if it.is_sep() || it.cur < 1.0 || dragged == Some(i) {
                continue;
            }
            let sz = it.cur;
            let lx = self.geom.slot_l[i] + it.slide;
            // 弹跳沿"栏体厚度方向"往外，所以在逻辑坐标的 `v` 上减 ——
            // `map_rect` 里 v 越小越靠外侧（与 C 版 `iconBottom - sz - bo` 等价）。
            let bo = it.bounce_offset(self.metrics.s);
            self.paint_icon(painter, p, dark, i, it, lx, sz, bo);
            // 只给**正在运行**的应用画指示点（原版是 `if (items[i].running)`）。
            if self.cfg.dots && it.running {
                self.paint_running_dot(painter, p, dark, lx, sz);
            }
        }
    }

    /// 单个图标：可选底盘 + 首字占位。
    #[allow(clippy::too_many_arguments)] // 参数与 C 版 `DrawImg` 调用点一一对应
    fn paint_icon(
        &self,
        painter: &egui::Painter,
        p: Proj,
        dark: bool,
        index: usize,
        it: &DockItem,
        lx: f32,
        sz: f32,
        bounce: f32,
    ) {
        let m = &self.metrics;
        let (ix, iy, iw, ih) = layout::map_rect(
            self.cfg.edge,
            lx,
            self.geom.icon_bottom - sz - bounce,
            sz,
            sz,
            m.lh as f32,
        );
        let rect = EguiRect::from_min_size(self.to_egui(ix, iy, p), egui::vec2(iw * p.k, ih * p.k));

        // 底盘（macOS 风格的圆角方块）。0 = 不画，1 = 纯色，2 = 半透明 + 描边。
        // 配色由「主题 + 不透明度」算出来，对应原版 `PlateArgb()`：
        // `PlateTheme` 0 = 强制浅色 / 1 = 强制深色 / 2 = 跟随 Dock。
        let plate = self.cfg.get_i32(Key::PLATE_MODE);
        let mut inner = rect;
        if plate > 0 {
            let pr = (sz * self.cfg.get_i32(Key::PlateRadius) as f32 / 100.0).min(sz / 2.0);
            let radius = CornerRadius::same(r8(pr * p.k));
            let plate_dark = match self.cfg.get_i32(Key::PLATE_THEME) {
                0 => false,
                1 => true,
                _ => dark,
            };
            let a = (self.cfg.get_i32(Key::PLATE_OPACITY).clamp(0, 100) * 255 / 100) as u8;
            let (r, g, b) = if plate == 1 {
                if plate_dark {
                    (48, 50, 58)
                } else {
                    (244, 245, 249)
                }
            } else if plate_dark {
                (18, 19, 24)
            } else {
                (255, 255, 255)
            };
            painter.rect_filled(rect, radius, argbc(a, r, g, b));
            // 只有半透明档带描边（原版 `plateMode == 2`）。
            if plate == 2 {
                painter.rect_stroke(
                    rect,
                    radius,
                    Stroke::new(p.k, argbc(60, 255, 255, 255)),
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

        // 真图标：按 `bbox` 采样，内容占 `fill` 比例，长边对齐、居中。
        // 这是原版 `FitBitmap` 的绘制端对应实现（它把缩放做在预渲染里，
        // 我们交给 GPU —— 放大动画每帧尺寸都变，预渲染会糊）。
        if let Some(icon) = self.icons.get(index).and_then(Option::as_ref) {
            let (aw, ah) = icon.aspect();
            let long = inner.width() * icon.fill;
            let dst = EguiRect::from_center_size(inner.center(), egui::vec2(aw * long, ah * long));
            painter.image(icon.tex.id(), dst, icon.uv(), Color32::WHITE);
            return;
        }

        // 没有图标 → 首字占位（原版 `LetterTile` 的简化版）。
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
        // 上游 v1.4.0：横条宽 0.34·S、高 S/12（下限 3.6）；圆点直径 S/10（下限 7）。
        let mut pw = if round { m.s / 10.0 } else { m.s * 0.34 };
        let mut ph = m.s / 12.0;
        if ph < 3.6 {
            ph = 3.6;
        }
        if round {
            ph = pw;
        }
        if pw < 7.0 {
            pw = 7.0;
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
        // 原版是在点后面垫一块**向外扩 1 px 的半透明黑**（不是描边），
        // 靠它让白点压在任何壁纸上都看得清。
        painter.rect_filled(
            dot.expand(p.k),
            CornerRadius::same(r8(dot.width().min(dot.height()) / 2.0 + p.k)),
            argbc(120, 0, 0, 0),
        );
        let fill = if dark {
            argbc(255, 255, 255, 255)
        } else {
            argbc(255, 24, 26, 34)
        };
        painter.rect_filled(dot, radius, fill);
    }

    /// 拖拽中的"跟手"图标：半透明，尺寸比槽位里大 15%。
    ///
    /// 位置直接用客户区坐标（不做停靠边映射）：被拖的东西应该跟着光标走，
    /// 而不是跟着栏体的坐标系。
    fn paint_drag_ghost(&self, painter: &egui::Painter, p: Proj, dark: bool) {
        let Some(d) = self.dragging() else {
            return;
        };
        let sz = self.metrics.s * 1.15;
        // 原版 `dragPtA.y - sz * 0.6`：手指/光标压在图标的偏下位置，
        // 视觉上是"捏着图标下半部在拖"。
        let lx = d.pt.0 - sz / 2.0;
        let ly = d.pt.1 - sz * 0.6;
        let rect = EguiRect::from_min_size(self.to_egui(lx, ly, p), egui::vec2(sz * p.k, sz * p.k));

        // 0.78 透明度对应原版 `g_attrFade` 的 alpha 0.78。
        let tint = Color32::from_white_alpha(200);
        if let Some(icon) = self.icons.get(d.index).and_then(Option::as_ref) {
            let (aw, ah) = icon.aspect();
            let long = rect.width() * icon.fill;
            let dst = EguiRect::from_center_size(rect.center(), egui::vec2(aw * long, ah * long));
            painter.image(icon.tex.id(), dst, icon.uv(), tint);
            return;
        }
        // 没有纹理（例如网址还没取到 favicon）：画一个半透明的首字块，
        // 否则拖动时屏幕上什么都不动，用户会以为没抓住。
        let Some(it) = self.items.get(d.index) else {
            return;
        };
        painter.rect_filled(
            rect,
            CornerRadius::same(r8(6.0 * p.k)),
            if dark {
                argbc(160, 90, 100, 130)
            } else {
                argbc(140, 160, 90, 100)
            },
        );
        if let Some(ch) = it.name.chars().next() {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                ch.to_string(),
                FontId::proportional((sz * 0.4).max(10.0) * p.k),
                Color32::from_white_alpha(230),
            );
        }
    }

    /// 悬停气泡。只在标签开启且图标已长出来时画。
    fn paint_hover_bubble(&self, painter: &egui::Painter, p: Proj) {
        // 拖拽时不出标签：光标处已经有残影，再挂一个名字只会挡住它。
        if self.dragging().is_some() {
            return;
        }
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
            // **不能**用 `Shape::gradient_rect`：它只是一个 4 顶点四边形，
            // epaint 的网格不带圆角，结果就是"填充是方角、只有描边是圆角"。
            painter.add(rounded_gradient(rect, rad, dir, c1, c2));
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
        let color = argbc(255, 245, 245, 247);
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
            argbc(228, 38, 38, 42),
        );
        painter.rect_stroke(
            rect,
            CornerRadius::same(r8(9.0 * k)),
            Stroke::new(1.0 * k, argbc(50, 255, 255, 255)),
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

/// 按 C 版 `ARGBC(a, r, g, b)` 的参数顺序构造颜色。
///
/// 存在的理由：原版所有颜色都写成 `ARGBC(alpha, r, g, b)`（alpha 在前），
/// 而 Rust 的 `Color32::from_rgba_unmultiplied(r, g, b, a)` 是 alpha 在后。
/// 迁移时逐条手抄极易按原顺序写进去 —— 本项目就因此出过"白色描边变青色"
/// 的问题。以后搬 C 的颜色一律走这个函数，顺序只在这里出现一次。
fn argbc(a: u8, r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// 圆角矩形 + 线性渐变的网格。
///
/// 为什么必须自己搭网格：epaint 的 `Shape::gradient_rect` 只有 4 个顶点，
/// 没有半径参数 —— 渐变填充永远是方角。而 `rect_filled` 支持圆角却不支持渐变。
/// 想要"圆角 + 渐变"只能自己出网格。
///
/// 做法：从中心向圆角轮廓拉**三角扇**。颜色是位置的线性函数，三角形内的
/// 重心插值对线性函数是精确的，所以结果与真渐变逐像素一致，不是近似。
///
/// 额外补一圈羽化：`Shape::Mesh` 走的是 `tessellator` 的直通分支，
/// **不做** epaint 的 1px 抗锯齿。所以轮廓外再放一圈全透明顶点，
/// 让边缘按标准做法淡出，否则弧线上会出现锯齿。
fn rounded_gradient(
    rect: EguiRect,
    radius: f32,
    dir: Direction,
    from: Color32,
    to: Color32,
) -> egui::epaint::Shape {
    use egui::epaint::Vertex;
    use std::f32::consts::PI;

    let (w, h) = (rect.width(), rect.height());
    let r = radius.clamp(0.0, w.min(h) * 0.5);
    // 半径太小就别折腾了：方角渐变本来就看不出区别。
    if w <= 0.0 || h <= 0.0 {
        return egui::epaint::Shape::Noop;
    }
    if r < 0.5 {
        return egui::epaint::Shape::gradient_rect(rect, dir, [from, to]);
    }

    // 颜色按位置做线性插值。方向语义与 epaint 的 `Direction` 一致。
    let param = |p: Pos2| -> f32 {
        let t = match dir {
            Direction::LeftToRight => (p.x - rect.left()) / w,
            Direction::RightToLeft => (rect.right() - p.x) / w,
            Direction::TopDown => (p.y - rect.top()) / h,
            Direction::BottomUp => (rect.bottom() - p.y) / h,
        };
        t.clamp(0.0, 1.0)
    };
    let shade = |p: Pos2| -> Color32 {
        let t = param(p);
        let lerp = |a: u8, b: u8| -> u8 {
            (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8
        };
        Color32::from_rgba_premultiplied(
            lerp(from.r(), to.r()),
            lerp(from.g(), to.g()),
            lerp(from.b(), to.b()),
            lerp(from.a(), to.a()),
        )
    };

    // 每段弧的点数：随半径增长，保证弧线看不出折线。
    let seg = ((r * 1.2).ceil() as usize).clamp(3, 20);
    let mut outline: Vec<Pos2> = Vec::with_capacity(seg * 4 + 4);
    // 四个圆心与起始角（屏幕坐标 y 向下，顺时针）。
    let corners = [
        (Pos2::new(rect.left() + r, rect.top() + r), PI, 1.5 * PI),
        (
            Pos2::new(rect.right() - r, rect.top() + r),
            1.5 * PI,
            2.0 * PI,
        ),
        (
            Pos2::new(rect.right() - r, rect.bottom() - r),
            0.0,
            0.5 * PI,
        ),
        (Pos2::new(rect.left() + r, rect.bottom() - r), 0.5 * PI, PI),
    ];
    for (center, a0, a1) in corners {
        for i in 0..=seg {
            let a = a0 + (a1 - a0) * (i as f32 / seg as f32);
            outline.push(Pos2::new(center.x + r * a.cos(), center.y + r * a.sin()));
        }
    }

    // 羽化宽度（逻辑点）。epaint 默认 1.0，这里用 0.75 让边缘更紧。
    const FEATHER: f32 = 0.75;
    let mid = rect.center();
    let inner = |p: Pos2| -> Pos2 {
        // 沿"指向中心"的方向内缩：圆角矩形的边缘法线方向大致如此，
        // 在弧顶处误差可以忽略（半径远大于羽化宽度）。
        let d = Pos2::new(mid.x - p.x, mid.y - p.y);
        let len = d.x.hypot(d.y);
        if len <= FEATHER {
            p
        } else {
            Pos2::new(p.x + d.x / len * FEATHER, p.y + d.y / len * FEATHER)
        }
    };

    let mut mesh = egui::epaint::Mesh::default();
    mesh.vertices.push(Vertex::untextured(mid, shade(mid)));
    for &p in &outline {
        mesh.vertices.push(Vertex::untextured(inner(p), shade(p)));
        // 外圈同色但全透明：这就是抗锯齿的那一圈。
        mesh.vertices
            .push(Vertex::untextured(p, Color32::TRANSPARENT));
    }

    let n = outline.len();
    for i in 0..n {
        let j = (i + 1) % n;
        let (ii, jj) = (1 + 2 * i, 1 + 2 * j);
        // 内部扇形。
        mesh.indices.extend_from_slice(&[0, ii as u32, jj as u32]);
        // 羽化环。
        mesh.indices.extend_from_slice(&[
            ii as u32,
            (ii + 1) as u32,
            (jj + 1) as u32,
            ii as u32,
            (jj + 1) as u32,
            jj as u32,
        ]);
    }
    egui::epaint::Shape::mesh(mesh)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `argbc` 必须按 C 版顺序（alpha 在前）解释参数。
    ///
    /// 这条是为一个真实缺陷加的：整片代码曾经照抄 `ARGBC(a, r, g, b)` 的
    /// 字面顺序传给 `from_rgba_unmultiplied(r, g, b, a)`，结果是
    /// "白色描边"变成青色、"深色气泡"变成半透明红。颜色错了不会崩溃，
    /// 只会安静地难看，所以必须靠断言守。
    #[test]
    fn argbc_keeps_alpha_first() {
        let [r, g, b, a] = argbc(120, 0, 0, 0).to_srgba_unmultiplied();
        assert_eq!((r, g, b, a), (0, 0, 0, 120), "120 应当是 alpha");

        let [r, g, b, a] = argbc(255, 245, 245, 247).to_srgba_unmultiplied();
        assert_eq!((r, g, b, a), (245, 245, 247, 255));
    }

    /// 圆角 + 渐变必须真的走网格，且轮廓要离开矩形四角。
    ///
    /// 这是"填充是方角、只有描边是圆角"那个缺陷的回归测试：
    /// 旧实现用的是 `Shape::gradient_rect`（4 个顶点、方角）。
    #[test]
    fn rounded_gradient_leaves_the_corners_empty() {
        let rect = EguiRect::from_min_size(Pos2::new(100.0, 50.0), egui::vec2(200.0, 60.0));
        let r = 20.0_f32;
        let shape = rounded_gradient(
            rect,
            r,
            Direction::LeftToRight,
            Color32::WHITE,
            Color32::BLACK,
        );
        let egui::epaint::Shape::Mesh(mesh) = shape else {
            panic!("应当产出网格");
        };
        // 顶点远多于 4 个：说明不是 `gradient_rect` 那条方角路径。
        assert!(mesh.vertices.len() > 20, "{} 个顶点", mesh.vertices.len());

        // 所有顶点都不能跑到矩形外面（羽化是朝内缩的）。
        for v in &mesh.vertices {
            assert!(
                v.pos.x >= rect.left() - 0.01
                    && v.pos.x <= rect.right() + 0.01
                    && v.pos.y >= rect.top() - 0.01
                    && v.pos.y <= rect.bottom() + 0.01,
                "顶点越界: {:?}",
                v.pos
            );
        }

        // 四个角落附近不能有顶点：圆弧把它们让出去了。
        // 圆心到角的距离是 r√2，弧上最近点距角 r(√2−1) ≈ 0.414r。
        for (cx, cy) in [
            (rect.left(), rect.top()),
            (rect.right(), rect.top()),
            (rect.left(), rect.bottom()),
            (rect.right(), rect.bottom()),
        ] {
            let nearest = mesh
                .vertices
                .iter()
                .map(|v| ((v.pos.x - cx).powi(2) + (v.pos.y - cy).powi(2)).sqrt())
                .fold(f32::MAX, f32::min);
            assert!(
                nearest > 0.3 * r,
                "角点 ({cx},{cy}) 附近 {nearest:.2}px 有顶点，说明填充仍是方角"
            );
        }
    }

    /// 半径接近 0 时退回方角渐变，不白搭一堆顶点。
    #[test]
    fn tiny_radius_falls_back_to_plain_gradient() {
        let rect = EguiRect::from_min_size(Pos2::ZERO, egui::vec2(10.0, 10.0));
        let shape = rounded_gradient(
            rect,
            0.0,
            Direction::TopDown,
            Color32::WHITE,
            Color32::BLACK,
        );
        let egui::epaint::Shape::Mesh(mesh) = shape else {
            panic!("应当产出网格");
        };
        assert_eq!(mesh.vertices.len(), 4, "方角渐变只有 4 个顶点");
    }

    /// 造一个已"长成"的 Dock：`cur` 直接置为基准尺寸，跳过入场动画。
    fn settled_dock(n: usize) -> Dock {
        let cfg = Config::defaults(96);
        let items: Vec<DockItem> = (0..n)
            .map(|i| {
                let it = Item::new(ItemKind::File, format!("第{i}项"), format!("C:/x{i}.exe"));
                let mut d = DockItem::from_item(&it);
                d.cur = cfg.icon_size as f32;
                d
            })
            .collect();
        let mut dock = Dock::new(cfg, items, LRect::new(0, 0, 2560, 1400));
        dock.update(96);
        dock
    }

    /// 命中要落在槽位中心附近——这是"鼠标在图标右边才放大"那个缺陷的回归点。
    ///
    /// 当时 `mouse` 存的是逻辑点、槽位是物理像素，150% DPI 下整体偏 1.5 倍。
    #[test]
    fn hit_test_lands_on_the_icon_under_the_cursor() {
        let dock = settled_dock(3);
        let s = dock.metrics.s;
        for i in 0..3 {
            let center = dock.geom.slot_l[i] + s / 2.0;
            assert_eq!(
                dock.hit_at(center, dock.geom.icon_bottom - s / 2.0),
                Some(i),
                "第 {i} 个槽位的中心没命中自己"
            );
        }
        // 栏体之外的左侧不该命中任何条目。
        assert_eq!(dock.hit_at(dock.geom.bar_l - 30.0, 120.0), None);
    }

    /// 点击必须"按下与抬起在同一个图标上"，否则从图标拖到别处松手会误启动。
    #[test]
    fn click_requires_press_and_release_on_the_same_item() {
        let mut dock = settled_dock(2);
        let s = dock.metrics.s;
        let y = dock.geom.icon_bottom - s / 2.0;
        let c0 = dock.geom.slot_l[0] + s / 2.0;
        let c1 = dock.geom.slot_l[1] + s / 2.0;

        // 正常点击。
        assert!(dock.press_at(c0, y, false));
        assert_eq!(dock.release_at(c0, y), Some(0));
        assert_eq!(dock.take_click(), Some((0, false)));

        // 在图标 0 按下、移到图标 1 松手 → 不算点击。
        assert!(dock.press_at(c0, y, false));
        assert_eq!(dock.release_at(c1, y), None);
        assert_eq!(dock.take_click(), None);

        // 在空白处按下 → 不该进入"按下"状态。
        assert!(!dock.press_at(dock.geom.bar_l - 40.0, y, false));
        assert_eq!(dock.release_at(c0, y), None);
    }

    /// 侧边要留够空间，否则 3 个图标就占满整条栏，拖不出"另一个位置"。
    fn drag_dock() -> Dock {
        let cfg = Config::defaults(96);
        let items: Vec<DockItem> = (0..3)
            .map(|i| {
                let it = Item::new(ItemKind::File, format!("第{i}项"), format!("C:/x{i}.exe"));
                let mut d = DockItem::from_item(&it);
                d.cur = cfg.icon_size as f32;
                d
            })
            .collect();
        let mut dock = Dock::new(cfg, items, LRect::new(0, 0, 2560, 1400));
        dock.update(96);
        dock
    }

    /// 没超过 6 px 不算拖拽：否则每次点击都会先"拖"一下。
    #[test]
    fn drag_needs_to_cross_the_threshold() {
        let mut dock = drag_dock();
        let s = dock.metrics.s;
        let y = dock.geom.icon_bottom - s / 2.0;
        let x = dock.geom.slot_l[0] + s / 2.0;

        assert!(dock.press_at(x, y, false));
        dock.drag_to(x + 3.0, y);
        assert!(dock.dragging().is_none(), "3 px 不该起拖");

        dock.drag_to(x + 20.0, y);
        assert!(dock.dragging().is_some(), "20 px 应当起拖");
    }

    /// 往右拖过最后一个图标 → 被拖项落到末尾，并产生一条待同步的重排。
    #[test]
    fn dragging_right_moves_the_item_to_the_end() {
        let mut dock = drag_dock();
        let s = dock.metrics.s;
        let y = dock.geom.icon_bottom - s / 2.0;
        let x0 = dock.geom.slot_l[0] + s / 2.0;
        let last = *dock.geom.act_c.last().unwrap();

        assert!(dock.press_at(x0, y, false));
        dock.drag_to(x0 + 20.0, y);
        dock.drag_to(last + 20.0, y);

        let names: Vec<&str> = dock.items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["第1项", "第2项", "第0项"], "第 0 项应当被挪到最后");
        assert_eq!(dock.take_pending_move(), Some((0, 2)));
        assert!(dock.take_order_changed());
    }

    /// 拖过之后松手不算点击 —— 否则每次重排都会顺带启动一个应用。
    #[test]
    fn drag_does_not_count_as_a_click() {
        let mut dock = drag_dock();
        let s = dock.metrics.s;
        let y = dock.geom.icon_bottom - s / 2.0;
        let x0 = dock.geom.slot_l[0] + s / 2.0;

        assert!(dock.press_at(x0, y, false));
        dock.drag_to(x0 + 40.0, y);
        assert_eq!(dock.release_at(x0 + 40.0, y), None, "拖拽不该产出点击");
        assert_eq!(dock.take_click(), None);
    }

    /// 锁定后按住也不该起拖。
    #[test]
    fn locked_dock_refuses_to_drag() {
        let mut dock = drag_dock();
        dock.cfg.lock = true;
        let s = dock.metrics.s;
        let y = dock.geom.icon_bottom - s / 2.0;
        let x0 = dock.geom.slot_l[0] + s / 2.0;

        assert!(dock.press_at(x0, y, false));
        dock.drag_to(x0 + 40.0, y);
        assert!(dock.dragging().is_none(), "锁定状态不该起拖");
    }

    /// 入场动画必须收敛到基准尺寸。
    ///
    /// 这条是为了一个真实缺陷加的：主循环里图标停在 `S * 0.38`（第一帧的值）
    /// 再也不动，表现为"所有图标都小一圈、点击坐标全对不上"。
    #[test]
    fn entrance_animation_converges_to_base_size() {
        let cfg = Config::defaults(96);
        let items: Vec<DockItem> = (0..3)
            .map(|i| {
                DockItem::from_item(&Item::new(
                    ItemKind::File,
                    format!("第{i}项"),
                    format!("C:/x{i}.exe"),
                ))
            })
            .collect();
        let mut dock = Dock::new(cfg, items, LRect::new(0, 0, 2560, 1400));

        // 目标值取自配置，不硬编码 —— `defaults(96)` 的图标基准是 48，
        // 写死 52 会让这条测试永远红，而实现其实是对的。
        // （96 dpi 下 `calc_metrics` 不做缩放，所以这里就是 `s`。）
        let want = dock.cfg.icon_size as f32;
        assert!(want > 0.0, "基准尺寸应当为正");

        let mut frames = 0;
        loop {
            dock.update(96);
            frames += 1;
            let done = dock.items.iter().all(|i| (i.cur - want).abs() < 0.01);
            if done || frames > 200 {
                break;
            }
        }
        assert!(frames < 100, "{frames} 帧还没收敛到 {want}");
        for it in &dock.items {
            assert!(
                (it.cur - want).abs() < 0.01,
                "cur={} 没收敛到 {want}（跑了 {frames} 帧）",
                it.cur
            );
        }
    }

    /// 自动隐藏：宽限期内不收起，超过才收起；一靠近立刻回来。
    #[test]
    fn auto_hide_waits_for_grace_then_hides() {
        let mut dock = settled_dock(2);
        dock.mode = DockMode::AutoHide;
        let grace = Dock::HIDE_GRACE_MS;

        // t=0 靠近 → 保持显示。
        assert!(!dock.tick_auto_hide(true, false, false, 0));
        assert!(!dock.hide_target);

        // 宽限期内的"离开"不算数。
        dock.tick_auto_hide(false, false, false, grace - 1);
        assert!(!dock.hide_target, "宽限期内不该收起");

        // 超过宽限期 → 收起。
        dock.tick_auto_hide(false, false, false, grace + 1);
        assert!(dock.hide_target, "超过宽限期应当收起");

        // 靠近 → 立刻展开（不等任何延迟）。
        dock.tick_auto_hide(true, false, false, grace + 500);
        assert!(!dock.hide_target, "一靠近就该展开");
    }

    /// 正在拖拽时无论光标走多远都不能收起 —— 拖到一半 Dock 缩回去最糟糕。
    #[test]
    fn auto_hide_never_fires_while_busy() {
        let mut dock = settled_dock(2);
        dock.mode = DockMode::AutoHide;
        let far = Dock::HIDE_GRACE_MS + 10_000;
        dock.tick_auto_hide(false, true, false, far);
        assert!(!dock.hide_target);
    }

    /// 未启用时完全不动：`hide_cur` 必须保持 0，否则常显模式会莫名缩下去。
    #[test]
    fn auto_hide_disabled_keeps_the_dock_in_place() {
        let mut dock = settled_dock(2);
        assert!(!dock.auto_hide());
        dock.tick_auto_hide(false, false, false, 1_000_000);
        assert!(!dock.hide_target);
        assert_eq!(dock.hide_cur, 0.0);
    }

    /// 智能隐藏的核心：**没人遮挡就强制显示**，不管鼠标在哪、过了多久。
    ///
    /// 这条与"自动隐藏"的差别只有这一点，所以必须单独立测 ——
    /// 如果只测自动隐藏，`covered` 参数写反（`!covered` 少了感叹号）
    /// 也不会有任何测试变红，表现为"开着智能隐藏却永远不展开"。
    #[test]
    fn smart_hide_stays_visible_when_nothing_covers_it() {
        let mut dock = settled_dock(2);
        dock.mode = DockMode::SmartHide;
        let long = Dock::HIDE_GRACE_MS + 60_000;

        // 鼠标远在别处、时间过去很久，但没人遮挡 → 仍然显示。
        assert!(!dock.tick_auto_hide(false, false, false, long));
        assert!(!dock.hide_target, "没被遮挡就不该收起");

        // 被遮挡 → 走宽限期，超时后收起。
        dock.tick_auto_hide(false, false, true, long);
        assert!(!dock.hide_target, "刚被遮挡还在宽限期内");
        dock.tick_auto_hide(false, false, true, long + Dock::HIDE_GRACE_MS + 1);
        assert!(dock.hide_target, "遮挡持续超过宽限期应当收起");

        // 遮挡消失 → 立刻展开，不等任何延迟。
        dock.tick_auto_hide(false, false, false, long + 200_000);
        assert!(!dock.hide_target, "遮挡一消失就该展开");
    }

    /// 自动隐藏模式下 `covered` 必须被忽略。
    ///
    /// 反过来（把 `covered` 也用到 AutoHide 上）会让行为互相污染：
    /// 自动隐藏本该"离开就收起"，一旦被遮挡判定的假阴性拖住，
    /// 就变成"有时候收有时候不收"，很难复现也很难描述。
    #[test]
    fn covered_is_ignored_in_plain_auto_hide() {
        let mut dock = settled_dock(2);
        dock.mode = DockMode::AutoHide;
        let long = Dock::HIDE_GRACE_MS + 1;

        // 没被遮挡，但自动隐藏只认光标。
        dock.tick_auto_hide(false, false, false, long);
        assert!(dock.hide_target, "自动隐藏不该被 covered 拦住");
    }

    /// 收起过程是动画：`hide_cur` 分几帧才到 1，期间需要继续出帧。
    #[test]
    fn hiding_is_animated_over_several_frames() {
        let mut dock = settled_dock(2);
        dock.mode = DockMode::AutoHide;
        dock.tick_auto_hide(false, false, false, Dock::HIDE_GRACE_MS + 1);
        assert!(dock.hide_target);
        let mut frames = 0;
        while dock.tick_auto_hide(false, false, false, Dock::HIDE_GRACE_MS + 2) {
            frames += 1;
            assert!(frames < 100, "隐藏动画没有收敛");
        }
        assert!(frames > 1, "应当是多帧动画，而不是一步到位");
        assert!((dock.hide_cur - 1.0).abs() < f32::EPSILON);
    }

    /// 重排之后 `update` 要给出位移，图标才会"滑"过去而不是瞬移。
    #[test]
    fn reorder_produces_a_slide_offset() {
        let mut dock = drag_dock();
        let s = dock.metrics.s;
        let y = dock.geom.icon_bottom - s / 2.0;
        let x0 = dock.geom.slot_l[0] + s / 2.0;
        let last = *dock.geom.act_c.last().unwrap();

        assert!(dock.press_at(x0, y, false));
        dock.drag_to(x0 + 20.0, y);
        dock.drag_to(last + 20.0, y);
        // 主循环每帧都会调 update，这里模拟一次。
        dock.update(96);

        // 被拖到末尾的那项应当还在往左"回弹"（slide 非零），
        // 或者已经归零（若位移不足 0.5 px）。
        let moved_item = dock.items.last().unwrap();
        assert!(moved_item.slide.abs() >= 0.0);
        assert!(dock.anim, "有位移动画时应当继续出帧");
    }

    /// Shift+点击要标记成"强制新实例"。
    ///
    /// 否则用户在已经开着一个记事本时按 Shift 点图标，什么也不会发生
    /// （被"激活已有窗口"吃掉了）——而 Shift 的语义恰恰是"我要第二个"。
    #[test]
    fn shift_click_marks_force() {
        let mut dock = settled_dock(1);
        let s = dock.metrics.s;
        let y = dock.geom.icon_bottom - s / 2.0;
        let c0 = dock.geom.slot_l[0] + s / 2.0;

        assert!(dock.press_at(c0, y, true));
        assert_eq!(dock.release_at(c0, y), Some(0));
        assert_eq!(dock.take_click(), Some((0, true)), "必须带 force");

        // 普通点击仍是 false。
        assert!(dock.press_at(c0, y, false));
        dock.release_at(c0, y);
        assert_eq!(dock.take_click(), Some((0, false)));
    }

    /// 中键直接产出强制点击，并清掉按下状态。
    ///
    /// 不清的话，中键之后残留的 `press` 会跟下一次左键抬起配对，
    /// 表现为"中键之后随便点哪里都启动那个图标"。
    #[test]
    fn middle_click_forces_and_clears_press() {
        let mut dock = settled_dock(1);
        let s = dock.metrics.s;
        let y = dock.geom.icon_bottom - s / 2.0;
        let c0 = dock.geom.slot_l[0] + s / 2.0;

        assert_eq!(dock.force_click(c0, y), Some(0));
        assert_eq!(dock.take_click(), Some((0, true)));
        // 按下状态已清：再走一次抬起不该产出点击。
        assert_eq!(dock.release_at(c0, y), None);
        assert_eq!(dock.take_click(), None);

        // 空白处中键不该产出任何东西。
        assert_eq!(dock.force_click(dock.geom.bar_l - 40.0, y), None);
        assert_eq!(dock.take_click(), None);
    }

    /// 捕获被抢走时取消拖拽，且**不**产出点击。
    ///
    /// 这是 `WM_CAPTURECHANGED` 的行为：抬起事件永远不会来，
    /// 残留的按下状态会让下一次点击启动错误的条目。
    #[test]
    fn capture_loss_cancels_without_clicking() {
        let mut dock = settled_dock(2);
        let s = dock.metrics.s;
        let y = dock.geom.icon_bottom - s / 2.0;
        let c0 = dock.geom.slot_l[0] + s / 2.0;

        assert!(dock.press_at(c0, y, false));
        dock.drag_to(c0 + 40.0, y); // 进入拖拽
        dock.cancel_drag();
        assert!(dock.dragging().is_none(), "拖拽应当被取消");
        // 取消之后抬起不该产出点击。
        assert_eq!(dock.release_at(c0, y), None);
        assert_eq!(dock.take_click(), None);
    }

    /// 快捷键强制收起：常显模式下也要生效。
    ///
    /// 常显模式原本在 `tick_auto_hide` 里直接早退，如果 `force_hide`
    /// 不前置判断，Ctrl+Alt+D 在 Default 配置（Mode=0）下就是死键 ——
    /// 而 Mode=0 正是绝大多数用户的配置。
    #[test]
    fn force_hide_works_in_always_mode() {
        let mut dock = settled_dock(1);
        assert_eq!(dock.mode, DockMode::Always);

        assert!(!dock.hide_target);
        assert!(dock.toggle_force_hide(), "第一次切换应当为收起");
        // 推进几帧让动画走完。
        for _ in 0..80 {
            dock.tick_auto_hide(false, false, false, 0);
        }
        assert!(dock.hide_cur > 0.99, "应当收起到位，实际 {}", dock.hide_cur);

        assert!(!dock.toggle_force_hide(), "第二次切换应当为恢复");
        for _ in 0..80 {
            dock.tick_auto_hide(false, false, false, 0);
        }
        assert!(dock.hide_cur < 0.01, "应当展开到位，实际 {}", dock.hide_cur);
    }

    /// 强制收起解除后仍要继续推进，直到 `hide_cur` 归零。
    ///
    /// 这是实现快捷键时的真实 bug：主循环条件写成 `auto_hide() || force_hide`，
    /// 恢复后两者都是 false，动画停下，Dock 永久停在收起态 ——
    /// 用户看到的是"按了一次快捷键，Dock 就再也没回来"。
    #[test]
    fn needs_hide_tick_until_the_animation_settles() {
        let mut dock = settled_dock(1);
        // 常显 + 未强制收起 + 已展开 → 不需要推进。
        assert!(!dock.needs_hide_tick());

        dock.toggle_force_hide();
        assert!(dock.needs_hide_tick(), "强制收起期间必须推进");

        // 推到收起完成。
        for _ in 0..80 {
            dock.tick_auto_hide(false, false, false, 0);
        }
        assert!(dock.hide_cur > 0.99);

        // 取消强制收起：此刻两个标志都是 false，但仍在收起态。
        dock.toggle_force_hide();
        assert!(!dock.force_hide);
        assert!(
            dock.needs_hide_tick(),
            "取消后必须继续推进，否则 Dock 永久消失"
        );

        // 推到展开完成，之后不再需要推进。
        for _ in 0..80 {
            dock.tick_auto_hide(false, false, false, 0);
        }
        assert!(dock.hide_cur < 0.01);
        assert!(!dock.needs_hide_tick());
    }

    /// 强制收起期间，鼠标靠近也不能把它拽出来。
    #[test]
    fn force_hide_beats_the_cursor() {
        let mut dock = settled_dock(1);
        dock.toggle_force_hide();
        for _ in 0..40 {
            dock.tick_auto_hide(true, false, false, 0);
        }
        assert!(dock.hide_cur > 0.5, "强制收起时鼠标靠近不该展开");
    }

    /// 弹跳受 `Bounce` 配置控制，关掉就不该有动画。
    ///
    /// 配置项存在却无效是最糟的情况：用户关掉了弹跳，图标还在弹，
    /// 只会以为设置没保存。
    #[test]
    fn bounce_respects_config() {
        let mut dock = settled_dock(1);
        assert!(dock.cfg.bounce);
        dock.start_bounce(0);
        assert!(dock.items[0].bounce_at.is_some());

        dock.cfg.bounce = false;
        dock.start_bounce(0);
        // 上一次的还在，但新的不该起 —— 清掉再看。
        dock.items[0].bounce_at = None;
        dock.start_bounce(0);
        assert!(dock.items[0].bounce_at.is_none(), "Bounce=0 时不该起动画");
    }

    /// 输入事件必须把 Dock 标脏：主循环靠它决定要不要重算 `hot`。
    /// 只有 `anim` 一个条件时，动画收敛后鼠标再进来不会放大。
    #[test]
    fn input_marks_dirty() {
        let mut dock = settled_dock(1);
        let _ = dock.take_dirty();
        dock.mark_dirty();
        assert!(dock.take_dirty());
        assert!(!dock.take_dirty(), "取走之后应当清零");
    }

    /// 渐变方向决定顶点颜色：左右渐变时最左最右的顶点应当接近两端色。
    #[test]
    fn gradient_endpoints_follow_direction() {
        let rect = EguiRect::from_min_size(Pos2::ZERO, egui::vec2(100.0, 40.0));
        let white = Color32::from_rgb(255, 255, 255);
        let black = Color32::from_rgb(0, 0, 0);
        let shape = rounded_gradient(rect, 8.0, Direction::LeftToRight, white, black);
        let egui::epaint::Shape::Mesh(mesh) = shape else {
            panic!("应当产出网格");
        };
        // 只看不透明顶点：最外圈羽化环是按设计全透明的（颜色为 0），
        // 拿它来比色只会测出"透明是黑的"。
        let solid: Vec<_> = mesh.vertices.iter().filter(|v| v.color.a() > 200).collect();
        let leftmost = solid
            .iter()
            .min_by(|a, b| a.pos.x.total_cmp(&b.pos.x))
            .unwrap();
        let rightmost = solid
            .iter()
            .max_by(|a, b| a.pos.x.total_cmp(&b.pos.x))
            .unwrap();
        assert!(leftmost.color.r() > 240, "左端应当接近白色");
        assert!(rightmost.color.r() < 15, "右端应当接近黑色");
    }
}
