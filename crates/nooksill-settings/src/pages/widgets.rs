//! 设置界面的共享控件与卡片骨架。
//!
//! 页面文件只回答"这一页有哪些设置"，控件长什么样、怎么排版，全在这里。
//! 加一个设置项 = `core::config` 加一个键 + 对应页面加一行，两处。
//!
//! # 排版约定
//!
//! 每行都是「左：标题（+ ⓘ）」+「右：控件」。控件的 accessible name 挂在
//! 左边的标题上（[`egui::Response::labelled_by`]）——egui 不会把相邻的
//! label 当成控件的名字，读屏软件和 kittest 都会找不到它。
//!
//! 行的说明文字**不再占版面**：收进标题后的圆形提示（[`hint_button`]），
//! 点击或悬停 3 秒展开成浮层。视觉基准是 `docs/settings-prototype.html`。

use std::time::Instant;

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Layout, Response, RichText, Sense, Stroke, Ui,
    WidgetInfo,
};
use nooksill_core::color;
use nooksill_core::config::{Config, Key};

use super::{Command, Outcome, Page, SettingsState};

/// 见 crate 根的 `NAV_W`（窗口层也要用，所以定义在那里）。
pub(crate) use crate::NAV_W;

/// 导航项高度。
pub(crate) const NAV_ITEM_H: f32 = 40.0;

/// 导航标签数量上限（雏形的容量约定；超出部分侧栏滚动）。
pub(crate) const MAX_LABELS: usize = 12;

/// 滑杆的固定宽度。egui 的滑杆默认会吃掉整行剩余宽度，在"标题在左、
/// 控件在右"的排版里会把标题挤扁。
const SLIDER_W: f32 = 200.0;

/// 右列控件预留的宽度。左列最大宽度 = 可用宽度 - 它。
///
/// 宁可让标题换一行，也不让控件被挤出可视区。
const CTL_RESERVE: f32 = 232.0;

/// 行与行之间的留白。
const ROW_GAP: f32 = 7.0;

/// 胶囊开关的尺寸（Windows 11 Fluent 比例 40×22，圆钮直径 14）。
const SWITCH_W: f32 = 40.0;
const SWITCH_H: f32 = 22.0;

/// 控件底色：按钮 / 滑条轨道 / 开关关态共用的"浅灰面板"，与
/// [`crate::text_field`] 的 field 同源 —— 半透明叠在卡片白上，浅色
/// 显灰、深色显亮，正好是 Fluent 的tonal 层。
fn field_bg(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgba_unmultiplied(255, 255, 255, 16)
    } else {
        Color32::from_rgba_unmultiplied(23, 26, 35, 14)
    }
}

/// 两色按 t 线性混合（t=0 取 a）。
fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

/// 卡片与卡片之间的留白（雏形 12–18px，取中值）。
const CARD_GAP: f32 = 14.0;

/// 悬停多久后自动展开说明气泡（雏形定为 3 秒）。
const TIP_HOVER_MS: u128 = 3000;

/// 表面色，逐字取自雏形（`docs/settings-prototype.html` 的
/// `--sidebar` / `--card` / `--menu`）。
///
/// 为什么不直接用 `visuals.panel_fill`：egui 的 `panel_fill` 在浅色主题下
/// 与窗口底色几乎同值，卡片会"消失"（第一版渲染出来就是这样）。窗口底色
/// 由 `settings_win::shell` 画，卡片要压在它上面，就得比它亮一档。
/// 深色下 `--menu` 比 `--card` 亮一档，浮层才有"浮起来"的层次。
mod palette {
    use egui::Color32;

    // 侧栏与卡片都是**半透明**的：窗口底在毛玻璃开启时 alpha=140，模糊
    // 桌面从底下透上来；侧栏/卡片用不透明纯色会把毛玻璃整块挡死，看起来
    // 像贴上去的实心板（与透毛玻璃的内容缝隙严重割裂）。透明度按 Fluent
    // acrylic 的 tonal 层次排：底最透 < 侧栏 < 卡片（可读性优先）。
    // 毛玻璃关闭时窗口底 alpha=250 近实色，叠加后整体 ≈ 不透明，同一套
    // 配色两态通用，不需要按 frost 切换。
    //
    // `from_rgba_unmultiplied` 不是 const，所以是函数而不是常量。
    pub(super) fn light_sidebar() -> Color32 {
        Color32::from_rgba_unmultiplied(251, 251, 253, 120)
    }
    pub(super) fn light_card() -> Color32 {
        Color32::from_rgba_unmultiplied(255, 255, 255, 170)
    }
    pub(super) const LIGHT_MENU: Color32 = Color32::from_rgb(255, 255, 255);
    pub(super) fn dark_sidebar() -> Color32 {
        Color32::from_rgba_unmultiplied(28, 29, 34, 130)
    }
    pub(super) fn dark_card() -> Color32 {
        Color32::from_rgba_unmultiplied(33, 34, 40, 150)
    }
    pub(super) const DARK_MENU: Color32 = Color32::from_rgb(38, 39, 47);
}

/// 行悬停底色：比中性 hover 多带一点主题色，扫过时能看出是哪一套配色。
fn row_hover(ui: &Ui) -> Color32 {
    ui.visuals().selection.bg_fill.linear_multiply(0.40)
}

/// 悬停底色（雏形的 `--hover`：浅色下一层黑、深色下一层白）。
fn hover(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgba_unmultiplied(255, 255, 255, 14)
    } else {
        Color32::from_rgba_unmultiplied(23, 26, 35, 12)
    }
}

/// 侧栏底色（外壳画窗口底色，侧栏这块半透明叠上去 —— 毛玻璃能透）。
pub(crate) fn sidebar_fill(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        palette::dark_sidebar()
    } else {
        palette::light_sidebar()
    }
}

/// 一页绘制过程中的可变状态。
///
/// 三个字段都是**借用**：页面函数拿到 `&mut Ctx`，改 `cfg` 就是改配置，
/// 产出命令就写 `out`。UI 层永远不碰副作用（写注册表、开对话框）——
/// 那是主程序的事（见 [`Command`]）。
pub(crate) struct Ctx<'a> {
    pub(crate) cfg: &'a mut Config,
    pub(crate) state: &'a mut SettingsState,
    pub(crate) out: &'a mut Outcome,
}

/// 卡片外框：半透明填充、大圆角、柔和投影 + 1px 微描边。
///
/// 填充改半透明后卡片能透毛玻璃，但边界也会跟着融进底色 —— 补一条
/// 极淡的描边（Fluent 卡片的 subtle border）负责"边界在哪"，
/// 投影继续负责"浮起来"。
fn card_frame(ui: &Ui) -> egui::Frame {
    let dark = ui.visuals().dark_mode;
    egui::Frame::new()
        .fill(if dark {
            palette::dark_card()
        } else {
            palette::light_card()
        })
        .stroke(egui::Stroke::new(
            1.0,
            if dark {
                Color32::from_rgba_unmultiplied(255, 255, 255, 20)
            } else {
                Color32::from_rgba_unmultiplied(23, 26, 35, 16)
            },
        ))
        .corner_radius(egui::CornerRadius::same(18))
        .inner_margin(egui::Margin::symmetric(16, 12))
        // 投影参数是试出来的：`spread` 一旦 > 0，投影矩形外扩但圆角半径
        // 不跟着扩，暗边就会从卡片圆角的半透明 AA 边缘里透出来
        // （实测圆角弧段上出现 214/222 的暗像素，直边却干净）。
        // 所以 spread 必须是 0，密度靠 alpha 与 blur 控制。
        .shadow(egui::Shadow {
            offset: [0, 8],
            blur: 22,
            spread: 0,
            color: Color32::from_black_alpha(if dark { 90 } else { 16 }),
        })
}

/// 一个卡片：标题（可选说明）+ 内容。
///
/// 雏形见 `docs/settings-prototype.html` —— 那里每个卡片对应这里的一次
/// `card()` 调用。
pub(crate) fn card<R>(
    ctx: &mut Ctx<'_>,
    ui: &mut Ui,
    title: &str,
    note: &str,
    add: impl FnOnce(&mut Ctx<'_>, &mut Ui) -> R,
) -> R {
    // 磁吸：卡片**整体**（连同内部控件）朝光标偏移。
    //
    // 内部控件在平移后的子 Ui 里布局，所以跟着一起动 —— 与雏形的 transform
    // 行为一致。衰减用的矩形高度取上一帧的实测值（首帧用估计值 300）。
    let avail_w = ui.available_width();
    let base_min = ui.cursor().min;
    let prev_h = ctx
        .state
        .card_h_prev
        .iter()
        .find(|(p, _)| (*p - base_min).length() < 1.0)
        .map(|(_, h)| *h);
    let base = egui::Rect::from_min_size(base_min, egui::vec2(avail_w, prev_h.unwrap_or(300.0)));
    let off = magnet_offset(ui, base, 3.5, 90.0);

    // `new_child` 不推进父光标：卡片画完之后用它的实际高度手动推进
    //（往下 3px 的磁吸偏移不能带进布局，否则会逐卡片累积）。
    let mut card_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(egui::Rect::from_min_size(
                base_min + off,
                egui::vec2(avail_w, ui.available_height()),
            ))
            .layout(Layout::top_down(Align::Min)),
    );
    // 卡片必须**一律撑满**可用宽度。
    //
    // `Frame` 的宽度由内容决定：一行 `label` 只有几十像素，那个卡片
    // 就缩成一小块（性能页的「当前占用」、添加页的「重置」都这样，
    // 摆在一列同宽卡片里像排版坏了）。这里直接把内容的最小宽度
    // 顶到可用宽度。
    let frame = card_frame(&card_ui);
    let inner = frame
        .show(&mut card_ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.strong(title);
            if !note.is_empty() {
                ui.label(RichText::new(note).small().weak());
            }
            ui.add_space(6.0);
            add(ctx, ui)
        })
        .inner;

    ctx.state
        .card_h
        .push((base_min, card_ui.min_rect().height()));
    ui.advance_cursor_after_rect(egui::Rect::from_min_size(
        base_min,
        egui::vec2(avail_w, card_ui.min_rect().height()),
    ));
    // 卡片间距：不能靠 item_spacing（那会同时影响卡片内部的行距）
    ui.add_space(CARD_GAP);
    inner
}

/// 一行：左边「标题（+ ⓘ）」，右边是控件。
///
/// `note` 不再排版 —— 收进标题后的圆形提示（[`hint_button`]），点击或
/// 悬停 3 秒展开。`add` 拿到标题的 `Id`，控件要用它做 `labelled_by`。
pub(crate) fn row<R>(
    ui: &mut Ui,
    state: &mut SettingsState,
    label: &str,
    note: &str,
    add: impl FnOnce(&mut Ui, egui::Id) -> R,
) -> R {
    let note = note.trim();
    let base_min = ui.cursor().min;

    // 悬停底色（两遍法）：上一帧的行矩形还在，先垫一层再画本行内容 ——
    // 层叠顺序正好落在卡片底色之上、行内容之下。布局稳定时矩形逐帧一致，
    // 只有滚动/翻页后的第一帧会错位一瞬。
    let prev_band = state
        .row_bands_prev
        .iter()
        .find(|(p, _)| (*p - base_min).length() < 1.0)
        .map(|(_, r)| *r);
    if let Some(band) = prev_band {
        let hovered = ui
            .ctx()
            .input(|i| i.pointer.hover_pos().is_some_and(|p| band.contains(p)));
        if hovered {
            ui.painter()
                .rect_filled(band, egui::CornerRadius::same(12), row_hover(ui));
        }
    }

    let outer = ui.horizontal(|ui| {
        // 给控件留出固定宽度，剩下的才归左列。不这么做的话，一句长标题会
        // 把控件挤出可视区 —— `horizontal` 不会自动换行。
        let max = (ui.available_width() - CTL_RESERVE).max(120.0);
        let id = ui
            .vertical(|ui| {
                ui.set_max_width(max);
                let mut id = egui::Id::unique("row-label");
                ui.horizontal(|ui| {
                    let resp = ui.label(label);
                    id = resp.id;
                    if !note.is_empty() {
                        hint_button(ui, state, note);
                    }
                });
                id
            })
            .inner;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| add(ui, id))
    });
    // `horizontal` 返回 `InnerResponse<InnerResponse<R>>` —— 两层壳都要拆。
    let r = outer.inner.inner;
    // 行与行之间留白。不能靠 `item_spacing`：那会连带把"标题 / ⓘ"的行距
    // 也撑开。
    ui.add_space(ROW_GAP);
    state.row_bands.push((base_min, outer.response.rect));
    r
}

/// 磁吸：光标靠近时，元素朝它偏移（最多 `max` 像素）。
///
/// `base` 是元素**不偏移**时的布局矩形。偏移量按"光标到矩形边缘的距离"
/// 衰减：光标在元素内 → 满偏；离开 `radius` 后归零。
///
/// 只偏移**绘制**，命中测试留在原位 —— 3 px 的差值对手感没有影响，
/// 却省掉一套"视觉与点击目标分离"的补偿逻辑（HTML 的 transform 同样
/// 只挪视觉，行为一致）。
fn magnet_offset(ui: &Ui, base: egui::Rect, max: f32, radius: f32) -> egui::Vec2 {
    let pointer = ui.ctx().input(|i| i.pointer.hover_pos());
    magnet_pull(base, pointer, max, radius)
}

/// 纯几何：给定指针位置，返回元素的磁吸位移。单独拆出来才有单测。
fn magnet_pull(base: egui::Rect, pointer: Option<egui::Pos2>, max: f32, radius: f32) -> egui::Vec2 {
    let Some(p) = pointer else {
        return egui::Vec2::ZERO;
    };
    // 光标到矩形边缘的最近距离（在矩形内则为 0）
    let nx = p.x.clamp(base.left(), base.right());
    let ny = p.y.clamp(base.top(), base.bottom());
    let dist = ((p.x - nx).powi(2) + (p.y - ny).powi(2)).sqrt();
    if dist > radius {
        return egui::Vec2::ZERO;
    }
    let pull = 1.0 - dist / radius;
    let c = base.center();
    let dx = p.x - c.x;
    let dy = p.y - c.y;
    let len = (dx * dx + dy * dy).sqrt().max(0.001);
    egui::vec2(dx / len * max * pull, dy / len * max * pull)
}

/// 圆形提示点：15px 的圆，灰底感叹号，悬停变强调色。
///
/// - **点击**：展开 / 收起气泡（钉住，移开光标不消失）；
/// - **悬停满 3 秒**：自动展开（雏形的交互定义）；
/// - 移开光标且未钉住：收起。
///
/// 气泡本体在 [`super::settings_ui`] 的末尾统一渲染 —— 这里只改状态。
fn hint_button(ui: &mut Ui, state: &mut SettingsState, note: &str) {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(15.0, 15.0), egui::Sense::click());
    let draw = rect.translate(magnet_offset(ui, rect, 1.2, 30.0));
    let vis = ui.visuals();
    let open = state.tip.as_ref().is_some_and(|t| t.id == resp.id);

    let (bg, fg) = if open || resp.hovered() {
        (
            vis.selection.bg_fill.linear_multiply(0.70),
            vis.selection.stroke.color,
        )
    } else {
        (hover(ui), vis.weak_text_color())
    };
    // 半径取 15：15px 的方块正好是圆。
    ui.painter().rect_filled(draw, CornerRadius::same(15), bg);
    ui.painter().text(
        draw.center(),
        Align2::CENTER_CENTER,
        "!",
        FontId::proportional(10.0),
        fg,
    );

    // ---- 悬停计时：满 3 秒自动展开 ----
    if resp.hovered() {
        if state
            .tip_hover
            .as_ref()
            .is_none_or(|(id, _)| *id != resp.id)
        {
            state.tip_hover = Some((resp.id, Instant::now()));
        } else if let Some((_, at)) = state.tip_hover {
            let elapsed = at.elapsed().as_millis();
            let already = state.tip.as_ref().is_some_and(|t| t.id == resp.id);
            if elapsed >= TIP_HOVER_MS && !already {
                state.tip = Some(super::Tip {
                    id: resp.id,
                    text: note.to_owned(),
                    anchor: rect,
                    pinned: false,
                });
            }
        }
    } else {
        if state
            .tip_hover
            .as_ref()
            .is_some_and(|(id, _)| *id == resp.id)
        {
            state.tip_hover = None;
        }
        // 未钉住的气泡在移开光标时收起
        if state
            .tip
            .as_ref()
            .is_some_and(|t| t.id == resp.id && !t.pinned)
        {
            state.tip = None;
        }
    }

    // ---- 点击：钉住 / 收起 ----
    if resp.clicked() {
        if state
            .tip
            .as_ref()
            .is_some_and(|t| t.id == resp.id && t.pinned)
        {
            state.tip = None;
        } else {
            state.tip = Some(super::Tip {
                id: resp.id,
                text: note.to_owned(),
                anchor: rect,
                pinned: true,
            });
        }
    }

    // 没有这句，读屏软件与 kittest 都查不到这个节点（自绘控件必须自报）。
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, true, format!("说明：{note}"))
    });
}

/// 说明气泡本体。在 [`super::settings_ui`] 的**末尾**渲染 —— 那时所有行的
/// 状态都已结算完，不会出现"这一帧气泡开着、下一帧状态才更新"的抖动。
pub(crate) fn render_tip(ui: &mut Ui, state: &mut SettingsState) {
    let Some(tip) = state.tip.clone() else { return };
    let mag = magnet_offset(ui, tip.anchor, 1.2, 30.0);
    // 气泡默认放在圆点下方；下方放不下（距窗口底不足一屏）就翻到上面。
    // 用 `pivot` 翻转，不需要提前知道气泡的高度。
    let below = tip.anchor.bottom() + 100.0 <= ui.max_rect().bottom();
    let (pivot, pos) = if below {
        (
            Align2::LEFT_TOP,
            egui::pos2(
                (tip.anchor.left() - 6.0).max(8.0),
                tip.anchor.bottom() + 8.0,
            ),
        )
    } else {
        (
            Align2::LEFT_BOTTOM,
            egui::pos2((tip.anchor.left() - 6.0).max(8.0), tip.anchor.top() - 8.0),
        )
    };
    // 气泡也磁吸：朝光标轻微偏移
    let pos = pos + mag;

    let rect = std::cell::Cell::new(egui::Rect::NOTHING);
    let area_id = egui::Id::unique("settings-tip");
    egui::Area::new(area_id)
        .order(egui::Order::Tooltip)
        .pivot(pivot)
        .fixed_pos(pos)
        .show(ui.ctx(), |ui| {
            let dark = ui.visuals().dark_mode;
            let frame = egui::Frame::new()
                .fill(if dark {
                    palette::DARK_MENU
                } else {
                    palette::LIGHT_MENU
                })
                .corner_radius(egui::CornerRadius::same(12))
                .inner_margin(egui::Margin::symmetric(12, 10))
                // spread 必须 = 0（与卡片同一条规矩）：spread 会外扩投影矩形
                // 但不扩圆角半径，暗边从气泡 AA 边缘里透出来。密度靠
                // alpha 与 blur 控制。
                .shadow(egui::Shadow {
                    offset: [0, 8],
                    blur: 20,
                    spread: 0,
                    color: Color32::from_black_alpha(if dark { 90 } else { 24 }),
                });
            let resp = frame.show(ui, |ui| {
                ui.set_max_width(240.0);
                ui.label(
                    RichText::new(&tip.text)
                        .size(11.5)
                        .color(ui.visuals().weak_text_color()),
                );
            });
            rect.set(resp.response.rect);
        });

    // 点气泡之外 → 收起（钉住也不例外；雏形是"点别处就关"）。
    // 点击圆点的那一帧不会误关：光标还落在锚点上。
    let clicked_outside = ui.ctx().input(|i| {
        i.pointer.any_click()
            && i.pointer
                .interact_pos()
                .is_some_and(|p| !tip.anchor.expand(6.0).contains(p) && !rect.get().contains(p))
    });
    if clicked_outside {
        state.tip = None;
        state.tip_hover = None;
    }
}

/// 开关行。
///
/// 开关画在右边、标题在左边（Windows 11 设置的惯例）。标题本身不响应点击
/// —— 与系统一致：开关在哪就点哪。
pub(crate) fn toggle(ctx: &mut Ctx<'_>, ui: &mut Ui, key: Key, label: &str, note: &str) {
    let before = ctx.cfg.get_i32(key);
    let mut v = before != 0;
    row(ui, ctx.state, label, note, |ui, id| {
        switch_widget(ui, id, &mut v);
    });
    if v != (before != 0) {
        ctx.cfg.apply_i32(key, i32::from(v));
        ctx.out.changed = true;
    }
}

/// 派生开关：界面上的布尔值不是某个键，而是从配置算出来的。
///
/// 两个现成的例子：磨砂（`bg_mode == Frost`）与"扫描运行中应用"（同时决定
/// `dots` 与 `show_running`）。这类开关原先直接写 `ui.checkbox(...)`，结果
/// 是复选框贴在文字左边，和同一张卡片里的其它行完全不同 —— 排版上一眼就
/// 能看出是两套写法。
pub(crate) fn toggle_derived(
    ctx: &mut Ctx<'_>,
    ui: &mut Ui,
    label: &str,
    note: &str,
    value: bool,
    apply: impl FnOnce(&mut Config, bool),
) {
    let mut v = value;
    row(ui, ctx.state, label, note, |ui, id| {
        switch_widget(ui, id, &mut v);
    });
    if v != value {
        apply(ctx.cfg, v);
        ctx.out.changed = true;
    }
}

/// 胶囊开关（Windows 11 Fluent 风格）：开启 = 主题色填充 + 白色圆钮
/// 靠右；关闭 = 灰底圆钮靠左。圆钮位置走 egui 布尔动画（0.12s 滑动），
/// 悬停浮一层 tonal，聚焦画主题色外圈。
///
/// 之前是裸 `Checkbox` 空方框 —— 勾选框表达"模式"而非"开关"，也是
/// 整个设置页里最工具味的一个元素。
fn switch_widget(ui: &mut Ui, label_id: egui::Id, checked: &mut bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(SWITCH_W, SWITCH_H),
        Sense::CLICK | Sense::focusable_noninteractive(),
    );
    let mut resp = resp.labelled_by(label_id);
    resp.widget_info(|| WidgetInfo::selected(egui::Role::Switch, true, *checked, ""));

    if resp.clicked() {
        *checked = !*checked;
        resp.request_focus();
    }
    if resp.has_focus()
        && ui.input(|i| i.key_pressed(egui::Key::Space) || i.key_pressed(egui::Key::Enter))
    {
        *checked = !*checked;
        resp.mark_changed();
    }

    let vis = ui.visuals();
    let accent = vis.selection.stroke.color;
    let t = ui
        .ctx()
        .animate_bool_with_time(label_id.with("switch-on"), *checked, 0.12);

    // 底：关态灰、开态主题色，中间随动画过渡；悬停叠 tonal。
    let bg = blend(field_bg(ui), accent, t);
    ui.painter()
        .rect_filled(rect, CornerRadius::same(SWITCH_H as u8 / 2), bg);
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(SWITCH_H as u8 / 2), hover(ui));
    }
    // 关态给一圈描边，灰底在白卡片上不至于没有边界。
    if t < 0.5 {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(SWITCH_H as u8 / 2),
            Stroke::new(1.0, vis.weak_text_color().linear_multiply(0.6)),
            egui::StrokeKind::Inside,
        );
    }
    // 圆钮：白色，从左滑到右。
    let r = SWITCH_H / 2.0 - 3.0;
    let cx = egui::emath::lerp((rect.left() + r + 3.0)..=(rect.right() - r - 3.0), t);
    ui.painter()
        .circle_filled(egui::pos2(cx, rect.center().y), r, Color32::WHITE);
    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            CornerRadius::same(SWITCH_H as u8 / 2 + 2),
            Stroke::new(1.5, accent.linear_multiply(0.5)),
            egui::StrokeKind::Inside,
        );
    }
    resp
}

/// 滑杆行。取值范围来自 `core::config`（`slider_range` 优先）。
pub(crate) fn slider(ctx: &mut Ctx<'_>, ui: &mut Ui, key: Key, label: &str, suffix: &str) {
    let (lo, hi) = key.slider_range().unwrap_or_else(|| key.range());
    let before = ctx.cfg.get_i32(key);
    let mut v = before;
    row(ui, ctx.state, label, "", |ui, id| {
        slider_line(ui, id, &mut v, lo, hi, suffix);
    });
    if v != before {
        ctx.cfg.apply_i32(key, v);
        ctx.out.changed = true;
    }
}

/// Fluent 风格滑条：5px 粗圆角轨道、实心主题色已填充段、白色圆钮带
/// 主题色描边（拖动 / 悬停放大），拖动时圆钮上方浮出数值气泡。
///
/// 不用 egui 原生 Slider：它的轨道和圆钮共用 `widgets.inactive.bg_fill`，
/// 配不出"灰轨道 + 白圆钮"，圆钮描边也改不了。自绘交互并不复杂 ——
/// 指针映射 + ←/→/Home/End，都显式写在下面。
fn slider_line(ui: &mut Ui, label_id: egui::Id, value: &mut i32, lo: i32, hi: i32, suffix: &str) {
    let value_w = 48.0;
    let rail_w = SLIDER_W - value_w;
    let (area, _) = ui.allocate_exact_size(egui::vec2(SLIDER_W, 24.0), Sense::hover());
    let rail = egui::Rect::from_center_size(
        egui::pos2(area.left() + rail_w / 2.0, area.center().y),
        egui::vec2(rail_w, 5.0),
    );
    // 命中区比轨道大一圈，窄轨道直接点很难命中。
    let resp = ui.interact(
        rail.expand(9.0),
        egui::Id::unique("slider").with(label_id),
        Sense::click_and_drag(),
    );
    resp.widget_info(|| WidgetInfo::selected(egui::Role::Slider, true, false, ""));
    let mut resp = resp.labelled_by(label_id);

    // 指针设值：点击 / 拖动都把指针 x 映射到轨道区间。
    if let Some(pos) = resp.interact_pointer_pos() {
        if resp.dragged() || resp.clicked() {
            let p = ((pos.x - rail.left()) / rail.width()).clamp(0.0, 1.0);
            let nv = (lo as f32 + p * (hi - lo) as f32).round() as i32;
            if nv != *value {
                *value = nv;
                resp.mark_changed();
            }
        }
    }
    // 键盘：全程 20 步为基准步长，Home/End 归零打满。
    if resp.has_focus() {
        let step = ((hi - lo) / 20).clamp(1, 10);
        let mut nv = *value;
        for (key, d) in [(egui::Key::ArrowLeft, -step), (egui::Key::ArrowRight, step)] {
            if ui.input(|i| i.key_pressed(key)) {
                nv = (*value + d).clamp(lo, hi);
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Home)) {
            nv = lo;
        }
        if ui.input(|i| i.key_pressed(egui::Key::End)) {
            nv = hi;
        }
        if nv != *value {
            *value = nv;
            resp.mark_changed();
        }
    }
    if resp.clicked() {
        resp.request_focus();
    }

    let vis = ui.visuals();
    let accent = vis.selection.stroke.color;
    let p = (*value - lo) as f32 / (hi - lo) as f32;
    let active = resp.dragged() || resp.is_pointer_button_down_on();

    // 轨道 + 已填充段（实心主题色）。
    ui.painter()
        .rect_filled(rail, CornerRadius::same(2), field_bg(ui));
    if p > 0.0 {
        let fill = egui::Rect::from_min_max(
            rail.min,
            egui::pos2(rail.left() + rail.width() * p, rail.max.y),
        );
        ui.painter()
            .rect_filled(fill, CornerRadius::same(2), accent);
    }
    // 圆钮：白底 + 主题色描边；拖动 / 悬停放大，同时浮出数值气泡。
    let r = if active || resp.hovered() { 9.0 } else { 7.5 };
    let cx = rail.left() + rail.width() * p.clamp(0.0, 1.0);
    let knob = egui::pos2(cx, rail.center().y);
    ui.painter().circle_filled(knob, r, Color32::WHITE);
    ui.painter()
        .circle_stroke(knob, r, Stroke::new(if active { 2.5 } else { 2.0 }, accent));
    if resp.has_focus() {
        ui.painter()
            .circle_stroke(knob, r + 2.5, Stroke::new(1.5, accent.linear_multiply(0.5)));
    }
    if active {
        let text = format!("{}{}", value, suffix);
        let font = FontId::proportional(11.0);
        let galley = ui.painter().layout_no_wrap(text, font, Color32::WHITE);
        let tip_w = galley.size().x + 14.0;
        let tip = egui::Rect::from_center_size(
            egui::pos2(knob.x, rail.top() - 16.0),
            egui::vec2(tip_w, 17.0),
        );
        ui.painter().rect_filled(tip, CornerRadius::same(5), accent);
        ui.painter().galley(
            egui::Align2::CENTER_CENTER
                .align_size_within_rect(galley.size(), tip)
                .min,
            galley,
            Color32::WHITE,
        );
    }
    // 右侧常驻数值。
    let vtext = format!("{}{}", value, suffix);
    ui.painter().text(
        egui::pos2(area.right(), area.center().y),
        Align2::RIGHT_CENTER,
        vtext,
        FontId::proportional(12.0),
        if active {
            accent
        } else {
            vis.weak_text_color()
        },
    );
}

/// 下拉行。选项表是 `(整数码, 显示文本)`。
pub(crate) fn combo(ctx: &mut Ctx<'_>, ui: &mut Ui, key: Key, label: &str, opts: &[(i32, &str)]) {
    let before = ctx.cfg.get_i32(key);
    let mut cur = before;
    row(ui, ctx.state, label, "", |ui, id| {
        let text = opts
            .iter()
            .find(|(v, _)| *v == cur)
            .map_or("—", |(_, t)| *t);
        egui::ComboBox::from_id_salt(key.name())
            .selected_text(text)
            .width(180.0)
            .show_ui(ui, |ui| {
                for (v, t) in opts {
                    ui.selectable_value(&mut cur, *v, *t);
                }
            })
            .response
            .labelled_by(id);
    });
    if cur != before {
        ctx.cfg.apply_i32(key, cur);
        ctx.out.changed = true;
    }
}

/// 按钮。点了就产出一条命令，UI 层不自己执行。
pub(crate) fn button(ctx: &mut Ctx<'_>, ui: &mut Ui, label: &str, cmd: Command) {
    let font = FontId::new(
        ui.style().text_styles[&egui::TextStyle::Body].size,
        egui::FontFamily::Proportional,
    );
    // 用 PLACEHOLDER 排版：实际颜色由下面 `galley(..., vis.text_color())`
    // 决定。写死白色会让浅色主题的按钮变成"白字浅灰底"，等于看不见。
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, Color32::PLACEHOLDER);
    let size = egui::vec2((galley.size().x + 44.0).max(72.0), 30.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::CLICK);
    resp.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, label));

    // 点击或（聚焦后）空格 / 回车都触发；`Command` 不是 Copy，触发
    // 收敛到一个分支才能 move。
    let activated = resp.clicked()
        || (resp.has_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Space) || i.key_pressed(egui::Key::Enter)));
    if activated {
        ctx.out.command = Some(cmd);
        resp.mark_changed();
        resp.request_focus();
    }

    // 三态：底色上叠 hover 层（按下叠两层更深）。半透明叠层在卡片白上
    // 正好形成渐进的 tonal 台阶，不用为深浅两套各配实色。
    let vis = ui.visuals();
    ui.painter()
        .rect_filled(rect, CornerRadius::same(10), field_bg(ui));
    if resp.is_pointer_button_down_on() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(10), hover(ui));
        ui.painter()
            .rect_filled(rect, CornerRadius::same(10), hover(ui));
    } else if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(10), hover(ui));
    }
    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(10),
            Stroke::new(1.5, vis.selection.stroke.color),
            egui::StrokeKind::Inside,
        );
    }
    ui.painter().galley(
        egui::Align2::CENTER_CENTER
            .align_size_within_rect(galley.size(), rect)
            .min,
        galley,
        vis.text_color(),
    );
}

/// 主题色：来源下拉（跟随系统 / 自定义）+ 自定义时的取色器。
///
/// 与 `color_row` 的区别：这里"跟随系统"是一个**有效取值**（-1），
/// 不是"没启用"；所以不用复选框，用下拉表达两种来源。
pub(crate) fn accent_row(ctx: &mut Ctx<'_>, ui: &mut Ui) {
    let cur = ctx.cfg.get_i32(Key::UI_ACCENT);
    let mut custom = cur >= 0;
    let mut rgb = {
        let c = color::from_colorref(cur.max(0) as u32);
        [color::red(c), color::green(c), color::blue(c)]
    };
    let before = custom;
    row(
        ui,
        ctx.state,
        "来源",
        "跟随 Windows 强调色，或指定自己的颜色",
        |ui, id| {
            egui::ComboBox::from_id_salt("ui-accent-mode")
                .selected_text(if custom { "自定义" } else { "跟随系统" })
                .width(112.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut custom, false, "跟随系统")
                        .labelled_by(id);
                    ui.selectable_value(&mut custom, true, "自定义")
                        .labelled_by(id);
                })
                .response
                .labelled_by(id);
            if custom {
                ui.color_edit_button_srgb(&mut rgb).labelled_by(id);
            }
        },
    );
    if custom != before {
        // 切到自定义时给一个可读的蓝色起点，别让用户从黑色开始调。
        ctx.cfg
            .apply_i32(Key::UI_ACCENT, if custom { 0x0099_6633 } else { -1 });
        ctx.out.changed = true;
    } else if custom {
        let v = color::to_colorref(color::rgb(rgb[0], rgb[1], rgb[2])) as i32;
        if v != cur {
            ctx.cfg.apply_i32(Key::UI_ACCENT, v);
            ctx.out.changed = true;
        }
    }
}

/// 自定义颜色：`-1` = 未启用，否则是 COLORREF。
pub(crate) fn color_row(ctx: &mut Ctx<'_>, ui: &mut Ui, key: Key, label: &str) {
    let cur = ctx.cfg.get_i32(key);
    let mut enabled = cur >= 0;
    let mut arr = {
        let c = color::from_colorref(cur.max(0) as u32);
        [color::red(c), color::green(c), color::blue(c)]
    };
    row(ui, ctx.state, label, "", |ui, id| {
        let resp = ui.checkbox(&mut enabled, "启用");
        resp.labelled_by(id);
        if enabled {
            ui.color_edit_button_srgb(&mut arr).labelled_by(id);
        }
    });
    if enabled != (cur >= 0) {
        let v = if enabled { 0x0033_6699 } else { -1 };
        ctx.cfg.apply_i32(key, v);
        ctx.out.changed = true;
    } else if enabled {
        let v = color::to_colorref(color::rgb(arr[0], arr[1], arr[2])) as i32;
        if v != cur {
            ctx.cfg.apply_i32(key, v);
            ctx.out.changed = true;
        }
    }
}

/// 左侧导航栏：不分组、整行可点、只显示标签。
///
/// `state.nav_filter` 非空时只显示命中的标签；当前页被过滤掉时自动跳到
/// 第一个命中页 —— 否则左栏和右栏会对不上。
pub(crate) fn sidebar(ui: &mut Ui, state: &mut SettingsState) {
    let filter = state.nav_filter.trim().to_lowercase();
    // 当前页必须在**进循环前**快照：点击会在循环中途改 `state.page`，
    // 边遍历边读会让"旧页是否可见"的判断依赖遍历顺序 —— 点当前项
    // 之上的一项时，旧页排在点击项之后、轮到它时页已被改成新页，
    // 匹配不上 → 循环后被自动跳页覆盖回第一页（常规）。
    let current = *page_ref(state);
    let mut first_visible: Option<Page> = None;
    let mut current_visible = false;
    let mut clicked: Option<Page> = None;

    for p in Page::ALL {
        if !filter.is_empty() && !p.title().to_lowercase().contains(&filter) {
            continue;
        }
        if first_visible.is_none() {
            first_visible = Some(p);
        }
        if current == p {
            current_visible = true;
        }
        if nav_item(ui, p.title(), current == p).clicked() {
            // 只记录，循环外统一落账：自动跳页不能盖掉本帧的点击。
            clicked = Some(p);
            // 切页后气泡的锚点属于旧页面，留着会飘在半空
            state.tip = None;
            state.tip_hover = None;
        }
    }

    if let Some(p) = clicked {
        state.page = p;
    } else if !current_visible {
        if let Some(p) = first_visible {
            state.page = p;
        }
    }
}

/// 借用拆分的小工具：`nav_item` 借 `ui`，页码从 `state` 里读。
fn page_ref(state: &SettingsState) -> &Page {
    &state.page
}

/// 一个导航项：整行可点的圆角块，选中时整块变成强调色底。
///
/// 不用 `selectable_label`：它按文字宽度收缩，导航栏会参差不齐，而且
/// 点击热区只有文字那么大。这里自己 `allocate_exact_size` 撑满整行。
fn nav_item(ui: &mut Ui, text: &str, selected: bool) -> egui::Response {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, NAV_ITEM_H), egui::Sense::click());

    // 磁吸：画布朝光标偏移，命中区留在布局位置
    let draw = rect.translate(magnet_offset(ui, rect, 2.5, 56.0));

    if ui.is_rect_visible(rect) {
        let vis = ui.visuals();
        let (bg, fg) = if selected {
            (
                vis.selection.bg_fill.linear_multiply(0.70),
                vis.selection.stroke.color,
            )
        } else if resp.hovered() {
            (hover(ui), vis.strong_text_color())
        } else {
            (Color32::TRANSPARENT, vis.text_color())
        };
        if bg != Color32::TRANSPARENT {
            ui.painter()
                .rect_filled(draw, egui::CornerRadius::same(13), bg);
        }
        ui.painter().text(
            egui::pos2(draw.left() + 14.0, draw.center().y),
            egui::Align2::LEFT_CENTER,
            text,
            FontId::proportional(13.0),
            fg,
        );
    }

    // 没有这句，读屏软件与 kittest 都查不到这个节点（自绘控件必须自报）。
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, selected, text));
    resp
}

/// 页头：标题 + 一句话说明。
pub(crate) fn page_header(ui: &mut Ui, page: Page) {
    ui.add_space(2.0);
    ui.heading(page.title());
    ui.label(RichText::new(page.subtitle()).small().weak());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 光标在元素内 = 满偏，方向朝光标；离得远 = 不动；幅度永不超上限。
    #[test]
    fn 磁吸位移的分档与方向() {
        let base = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(160.0, 40.0));
        let c = base.center();

        // 元素内：满偏，方向 = 元素中心 → 光标
        let inside = magnet_pull(base, Some(egui::pos2(c.x - 30.0, c.y + 5.0)), 2.5, 56.0);
        assert!(
            inside.x < -2.0 && inside.y > 0.0,
            "应朝左下的光标偏移: {inside:?}"
        );
        assert!(
            inside.length() <= 2.5 + 1e-6,
            "幅度不得超过上限: {inside:?}"
        );

        // 刚好在半径边缘：为零
        let edge = magnet_pull(base, Some(egui::pos2(base.left() - 56.0, c.y)), 2.5, 56.0);
        assert_eq!(edge, egui::Vec2::ZERO, "半径之外不应偏移");

        // 半径之内但不在元素上：按距离衰减
        let near = magnet_pull(base, Some(egui::pos2(base.left() - 28.0, c.y)), 2.5, 56.0);
        assert!(
            near.x < -1.0 && near.x > -2.5,
            "一半距离应约为一半幅度: {near:?}"
        );

        // 没有指针（窗口外）：不动
        assert_eq!(magnet_pull(base, None, 2.5, 56.0), egui::Vec2::ZERO);
    }
}
