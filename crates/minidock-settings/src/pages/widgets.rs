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

use egui::{Align, Align2, Color32, CornerRadius, FontId, Layout, RichText, Ui};
use minidock_core::color;
use minidock_core::config::{Config, Key};

use super::{Command, Outcome, Page, SettingsState};

/// 左侧导航栏宽度（逻辑像素）。**写死**，不随标签内容变化。
pub(crate) const NAV_W: f32 = 193.0;

/// 导航项高度。
const NAV_ITEM_H: f32 = 40.0;

/// 滑杆的固定宽度。egui 的滑杆默认会吃掉整行剩余宽度，在"标题在左、
/// 控件在右"的排版里会把标题挤扁。
const SLIDER_W: f32 = 200.0;

/// 右列控件预留的宽度。左列最大宽度 = 可用宽度 - 它。
///
/// 宁可让标题换一行，也不让控件被挤出可视区。
const CTL_RESERVE: f32 = 232.0;

/// 行与行之间的留白。
const ROW_GAP: f32 = 7.0;

/// 开关占位。不写死的话复选框会贴着文字内边距缩进，右边缘比下拉框少 9 px，
/// 一列开关与一列下拉摆在同一张卡片里能看出来。
const CTL_TOGGLE_W: f32 = 22.0;
const CTL_TOGGLE_H: f32 = 18.0;

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

    pub(super) const LIGHT_SIDEBAR: Color32 = Color32::from_rgb(251, 251, 253);
    pub(super) const LIGHT_CARD: Color32 = Color32::from_rgb(255, 255, 255);
    pub(super) const LIGHT_MENU: Color32 = Color32::from_rgb(255, 255, 255);
    pub(super) const DARK_SIDEBAR: Color32 = Color32::from_rgb(28, 29, 34);
    pub(super) const DARK_CARD: Color32 = Color32::from_rgb(33, 34, 40);
    pub(super) const DARK_MENU: Color32 = Color32::from_rgb(38, 39, 47);
}

/// 悬停底色（雏形的 `--hover`：浅色下一层黑、深色下一层白）。
fn hover(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgba_unmultiplied(255, 255, 255, 14)
    } else {
        Color32::from_rgba_unmultiplied(23, 26, 35, 12)
    }
}

/// 侧栏底色（外壳只画窗口底色，侧栏这块由这里铺）。
pub(crate) fn sidebar_fill(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        palette::DARK_SIDEBAR
    } else {
        palette::LIGHT_SIDEBAR
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

/// 卡片外框：无描边、大圆角、柔和投影 —— "无直线"靠这个成立。
///
/// 颜色不用 `visuals.panel_fill`（浅色下与窗口底色同值，卡片会消失），
/// 投影代替 1px 描边把卡片从底色上托起来。
fn card_frame(ui: &Ui) -> egui::Frame {
    let dark = ui.visuals().dark_mode;
    egui::Frame::new()
        .fill(if dark {
            palette::DARK_CARD
        } else {
            palette::LIGHT_CARD
        })
        .corner_radius(egui::CornerRadius::same(18))
        .inner_margin(egui::Margin::symmetric(16, 12))
        .shadow(egui::Shadow {
            offset: [0, 10],
            blur: 28,
            spread: 2,
            color: Color32::from_black_alpha(if dark { 110 } else { 26 }),
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
    card_frame(ui)
        .show(ui, |ui| {
            // 卡片必须**一律撑满**可用宽度。
            //
            // `Frame` 的宽度由内容决定：一行 `label` 只有几十像素，那个卡片
            // 就缩成一小块（性能页的「当前占用」、添加页的「重置」都这样，
            // 摆在一列同宽卡片里像排版坏了）。这里直接把内容的最小宽度
            // 顶到可用宽度。
            ui.set_min_width(ui.available_width());
            ui.strong(title);
            if !note.is_empty() {
                ui.label(RichText::new(note).small().weak());
            }
            ui.add_space(6.0);
            add(ctx, ui)
        })
        .inner
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
    let r = ui
        .horizontal(|ui| {
            // 给控件留出固定宽度，剩下的才归左列。不这么做的话，一句长标题会
            // 把控件挤出可视区 —— `horizontal` 不会自动换行。
            let max = (ui.available_width() - CTL_RESERVE).max(120.0);
            let id = ui
                .vertical(|ui| {
                    ui.set_max_width(max);
                    #[expect(deprecated, reason = "占位 id，真实 id 由 label 的 resp 提供")]
                    let mut id = egui::Id::new("row-label");
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
        })
        // `horizontal` 返回 `InnerResponse<InnerResponse<R>>` —— 两层壳都要拆。
        .inner
        .inner;
    // 行与行之间留白。不能靠 `item_spacing`：那会连带把"标题 / ⓘ"的行距
    // 也撑开。
    ui.add_space(ROW_GAP);
    r
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
    let vis = ui.visuals();
    let open = state.tip.as_ref().is_some_and(|t| t.id == resp.id);

    let (bg, fg) = if open || resp.hovered() {
        (
            vis.selection.bg_fill.linear_multiply(0.35),
            vis.selection.stroke.color,
        )
    } else {
        (hover(ui), vis.weak_text_color())
    };
    // 半径取 15：15px 的方块正好是圆。
    ui.painter().rect_filled(rect, CornerRadius::same(15), bg);
    ui.painter().text(
        rect.center(),
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
pub(crate) fn render_tip(ui: &mut Ui, tip: &super::Tip) {
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

    #[expect(deprecated, reason = "整个应用只有一个气泡，固定 id 即可")]
    let area_id = egui::Id::new("settings-tip");
    egui::Area::new(area_id)
        .order(egui::Order::Tooltip)
        .pivot(pivot)
        .fixed_pos(pos)
        .show(ui.ctx(), |ui| {
            let dark = ui.visuals().dark_mode;
            egui::Frame::new()
                .fill(if dark {
                    palette::DARK_MENU
                } else {
                    palette::LIGHT_MENU
                })
                .corner_radius(egui::CornerRadius::same(12))
                .inner_margin(egui::Margin::symmetric(12, 10))
                .shadow(egui::Shadow {
                    offset: [0, 12],
                    blur: 24,
                    spread: 2,
                    color: Color32::from_black_alpha(if dark { 120 } else { 40 }),
                })
                .show(ui, |ui| {
                    ui.set_max_width(240.0);
                    ui.label(
                        RichText::new(&tip.text)
                            .size(11.5)
                            .color(ui.visuals().weak_text_color()),
                    );
                });
        });
}

/// 开关行。
///
/// 开关画在右边、标题在左边（Windows 11 设置的惯例）。标题本身不响应点击
/// —— 与系统一致：开关在哪就点哪。
pub(crate) fn toggle(ctx: &mut Ctx<'_>, ui: &mut Ui, key: Key, label: &str, note: &str) {
    let before = ctx.cfg.get_i32(key);
    let mut v = before != 0;
    row(ui, ctx.state, label, note, |ui, id| {
        ui.add_sized(
            [CTL_TOGGLE_W, CTL_TOGGLE_H],
            egui::Checkbox::new(&mut v, ""),
        )
        .labelled_by(id);
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
        ui.add_sized(
            [CTL_TOGGLE_W, CTL_TOGGLE_H],
            egui::Checkbox::new(&mut v, ""),
        )
        .labelled_by(id);
    });
    if v != value {
        apply(ctx.cfg, v);
        ctx.out.changed = true;
    }
}

/// 滑杆行。取值范围来自 `core::config`（`slider_range` 优先）。
pub(crate) fn slider(ctx: &mut Ctx<'_>, ui: &mut Ui, key: Key, label: &str, suffix: &str) {
    let (lo, hi) = key.slider_range().unwrap_or_else(|| key.range());
    let before = ctx.cfg.get_i32(key);
    let mut v = before;
    row(ui, ctx.state, label, "", |ui, id| {
        ui.add_sized(
            [SLIDER_W, 20.0],
            egui::Slider::new(&mut v, lo..=hi).suffix(suffix),
        )
        .labelled_by(id);
    });
    if v != before {
        ctx.cfg.apply_i32(key, v);
        ctx.out.changed = true;
    }
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
    if ui.button(label).clicked() {
        ctx.out.command = Some(cmd);
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
    let mut first_visible: Option<Page> = None;
    let mut current_visible = false;

    for p in Page::ALL {
        if !filter.is_empty() && !p.title().to_lowercase().contains(&filter) {
            continue;
        }
        if first_visible.is_none() {
            first_visible = Some(p);
        }
        if *page_ref(state) == p {
            current_visible = true;
        }
        if nav_item(ui, p.title(), *page_ref(state) == p).clicked() {
            state.page = p;
            // 切页后气泡的锚点属于旧页面，留着会飘在半空
            state.tip = None;
            state.tip_hover = None;
        }
    }

    if !current_visible {
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

    if ui.is_rect_visible(rect) {
        let vis = ui.visuals();
        let (bg, fg) = if selected {
            (
                vis.selection.bg_fill.linear_multiply(0.30),
                vis.selection.stroke.color,
            )
        } else if resp.hovered() {
            (hover(ui), vis.strong_text_color())
        } else {
            (Color32::TRANSPARENT, vis.text_color())
        };
        if bg != Color32::TRANSPARENT {
            ui.painter()
                .rect_filled(rect, egui::CornerRadius::same(13), bg);
        }
        ui.painter().text(
            egui::pos2(rect.left() + 14.0, rect.center().y),
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
