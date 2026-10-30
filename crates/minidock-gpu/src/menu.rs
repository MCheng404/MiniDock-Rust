//! 右键菜单：项模型、按条目类型构建、布局与绘制。
//!
//! # 为什么要独立窗口
//!
//! 菜单向上（或向屏幕内侧）弹出到栏体之外，而 Dock 窗口只有栏体那么大。
//! 分层窗口不会因为内容画到矩形外就被裁剪 —— 画出去的部分**直接不可见**。
//! 所以菜单必须有自己的窗口。
//!
//! # 分工
//!
//! 这个模块只管"菜单长什么样、有哪些项、多大"（纯逻辑，可测）。
//! 建窗、模态消息循环、命令执行放在 `bin/dock.rs` —— 那些必须碰 Win32，
//! 而这里的每一条都能用单元测试钉住。
//!
//! 上游对应 `ShowMenuAt()` + `ShowMenu()`：C 版是阻塞式自绘菜单窗口，
//! 这里保持同样的模型（打开 → 选一项或取消 → 返回命令）。

use minidock_core::item::{Item, ItemKind};

use egui::epaint::{CornerRadius, StrokeKind};
use egui::Rect as EguiRect;
use egui::{Align2, Color32, FontId, Stroke};

/// 菜单命令。执行逻辑在调用方（`bin/dock.rs`）。
///
/// 只放**当前真能执行**的命令。
///
/// 改变条目数量的三项（移除 / 加分割线 / 删分割线）依赖配置热更新：
/// 条目数变 → 栏体宽度变 → 窗口尺寸变 → 离屏目标与 DIB 要跟着重建。
/// 这条链在 `bin/dock.rs` 的 ②.1 里，菜单只负责发出命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuCmd {
    /// 打开 / 启动；已运行时由调用方决定是激活还是新起。
    Open,
    /// 在资源管理器中定位（仅文件夹）。
    OpenInExplorer,
    /// 请求应用退出（仅正在运行的普通应用）。
    QuitApp,
    /// 在当前项右侧插入一条分割线。
    AddSepAfter,
    /// 从 Dock 移除（普通条目）。
    RemoveItem,
    /// 删除这条分割线。
    RemoveSep,
    /// 打开设置窗口。
    Settings,
    /// 退出 Dock 进程。
    ExitDock,
}

/// 一个菜单项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuEntry {
    pub label: String,
    pub cmd: MenuCmd,
    /// 红色文字（移除类）。警告色是"这一步不容易撤销"的唯一提示。
    pub danger: bool,
    /// 此项之后画一条分隔线。
    pub sep_after: bool,
}

impl MenuEntry {
    fn new(label: &str, cmd: MenuCmd, danger: bool, sep_after: bool) -> Self {
        Self {
            label: label.to_owned(),
            cmd,
            danger,
            sep_after,
        }
    }
}

/// 菜单布局参数（逻辑像素）。集中在这里，绘制与命中判定共用同一份。
const ITEM_H: f32 = 31.0;
const SEP_H: f32 = 9.0;
const PAD_Y: f32 = 6.0;
const PAD_X: f32 = 12.0;
/// 菜单最小宽度。太窄会让"在资源管理器中打开"这种长标签挤在一起。
const MIN_W: f32 = 190.0;
/// 菜单背景圆角（逻辑像素）。
const RADIUS: f32 = 10.0;

/// 一次打开的菜单。
#[derive(Debug, Clone)]
pub struct Menu {
    pub entries: Vec<MenuEntry>,
    /// 当前高亮项（是 `entries` 的下标）。
    pub hover: Option<usize>,
}

impl Menu {
    /// 按被点的条目构建菜单。`item` 为 `None` 表示点在空白处（整条 Dock）。
    ///
    /// `running` 决定是否出现"退出应用"：对没在跑的应用显示这一项，
    /// 用户点了只会困惑。
    ///
    /// 返回 `None` 表示**不该弹菜单**（例如点在分割线上而没有可用命令）。
    /// 让菜单为空窗口是对用户的欺骗：弹出一个小方框却什么都不能点。
    #[must_use]
    pub fn build(item: Option<&Item>, running: bool) -> Option<Self> {
        let mut entries = Vec::new();
        match item.map(|i| i.kind) {
            None => {
                entries.push(MenuEntry::new("退出 Dock", MenuCmd::ExitDock, true, false));
            }
            Some(ItemKind::Separator) => {
                entries.push(MenuEntry::new(
                    "删除分割线",
                    MenuCmd::RemoveSep,
                    true,
                    false,
                ));
            }
            Some(kind) => {
                // 运行中的普通应用给"显示 / 最小化"，
                // 与上游 `it->running && it->type == T_FILE ? ...` 一致。
                let label = if running && kind == ItemKind::File {
                    "显示 / 最小化"
                } else {
                    "打开"
                };
                entries.push(MenuEntry::new(label, MenuCmd::Open, false, false));
                if kind == ItemKind::Folder {
                    entries.push(MenuEntry::new(
                        "在资源管理器中打开",
                        MenuCmd::OpenInExplorer,
                        false,
                        false,
                    ));
                }
                if running && matches!(kind, ItemKind::File | ItemKind::Url) {
                    entries.push(MenuEntry::new("退出应用", MenuCmd::QuitApp, false, false));
                }
                entries.push(MenuEntry::new(
                    "在右侧添加分割线",
                    MenuCmd::AddSepAfter,
                    false,
                    true,
                ));
                entries.push(MenuEntry::new(
                    "从 Dock 移除",
                    MenuCmd::RemoveItem,
                    true,
                    false,
                ));
            }
        }
        if entries.is_empty() {
            return None;
        }
        // "Dock 设置"必须在**任何**菜单里都有。
        //
        // 只放在空白处是不够的：图标一多，栏体就被占满，用户右键任何地方
        // 都落在某个图标上 —— 而设置窗口是目前的唯一入口（还没有托盘）。
        // 放在最后并用分隔线隔开，顺带保住"红色项不与常用项挨着"。
        if let Some(last) = entries.last_mut() {
            last.sep_after = true;
        }
        entries.push(MenuEntry::new("Dock 设置", MenuCmd::Settings, false, false));
        Some(Self {
            entries,
            hover: None,
        })
    }

    /// 菜单的逻辑尺寸 `(宽, 高)`。
    ///
    /// 宽度按最长标签估算：CJK 与拉丁字符的宽度差很大，统一按
    /// "每字符 13 px" 会明显偏窄（中文实际约等于 13 px，拉丁约 7 px），
    /// 所以分档估算再取最小值兜底。
    #[must_use]
    pub fn logical_size(&self) -> (f32, f32) {
        let mut w: f32 = MIN_W;
        let mut h = PAD_Y * 2.0;
        for e in &self.entries {
            w = w.max(estimate_width(&e.label) + PAD_X * 2.0);
            h += ITEM_H;
            if e.sep_after {
                h += SEP_H;
            }
        }
        (w, h)
    }

    /// 命中：给定菜单内的逻辑坐标，返回项下标。
    ///
    /// 分隔线不算可点区域（跳过），与视觉一致。
    #[must_use]
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        let (w, _) = self.logical_size();
        if x < 0.0 || x > w {
            return None;
        }
        let mut top = PAD_Y;
        for (i, e) in self.entries.iter().enumerate() {
            if y >= top && y < top + ITEM_H {
                return Some(i);
            }
            top += ITEM_H;
            if e.sep_after {
                top += SEP_H;
            }
        }
        None
    }

    /// 更新高亮，返回是否有变化（主循环据此决定要不要重绘）。
    pub fn set_hover(&mut self, at: Option<usize>) -> bool {
        if self.hover == at {
            return false;
        }
        self.hover = at;
        true
    }

    /// 选中项对应的命令。
    #[must_use]
    pub fn cmd_at(&self, index: usize) -> Option<MenuCmd> {
        self.entries.get(index).map(|e| e.cmd)
    }

    /// 把菜单放在光标附近，并保证整体落在 `bounds`（屏幕工作区）内。
    ///
    /// 优先放光标下方；下方放不下就翻到上方 —— 底边停靠的 Dock 菜单
    /// 必须向上弹，否则会顶出屏幕。
    #[must_use]
    pub fn place(cursor: (f32, f32), size: (f32, f32), bounds: (f32, f32, f32, f32)) -> (f32, f32) {
        let (cx, cy) = cursor;
        let (w, h) = size;
        let (left, top, right, bottom) = bounds;

        let mut x = cx;
        let mut y = cy;
        if x + w > right {
            x = right - w;
        }
        if x < left {
            x = left;
        }
        if y + h > bottom {
            y = cy - h;
        }
        if y < top {
            y = top;
        }
        (x, y)
    }
}

/// 菜单的配色。集中在一处，改主题只动这里。
struct Colors {
    bg: Color32,
    border: Color32,
    ink: Color32,
    danger: Color32,
    hover: Color32,
    sep: Color32,
}

impl Colors {
    fn for_theme(dark: bool) -> Self {
        if dark {
            Self {
                bg: Color32::from_rgba_unmultiplied(30, 30, 34, 244),
                border: Color32::from_rgba_unmultiplied(255, 255, 255, 34),
                ink: Color32::from_rgba_unmultiplied(240, 240, 245, 255),
                danger: Color32::from_rgba_unmultiplied(255, 116, 116, 255),
                hover: Color32::from_rgba_unmultiplied(255, 255, 255, 30),
                sep: Color32::from_rgba_unmultiplied(255, 255, 255, 30),
            }
        } else {
            Self {
                bg: Color32::from_rgba_unmultiplied(250, 250, 252, 248),
                border: Color32::from_rgba_unmultiplied(0, 0, 0, 26),
                ink: Color32::from_rgba_unmultiplied(28, 28, 32, 255),
                danger: Color32::from_rgba_unmultiplied(198, 40, 40, 255),
                hover: Color32::from_rgba_unmultiplied(0, 0, 0, 20),
                sep: Color32::from_rgba_unmultiplied(0, 0, 0, 22),
            }
        }
    }
}

impl Menu {
    /// 在 `ui` 的逻辑坐标系里绘制。原点 (0,0) 就是菜单窗口左上角。
    ///
    /// 尺寸必须与 [`Menu::logical_size`] 及 [`Menu::hit`] 完全一致 ——
    /// 三者任何一处偏差都表现为"看到的位置和点到的位置不一样"。
    pub fn paint(&self, ui: &egui::Ui, dark: bool) {
        let (w, h) = self.logical_size();
        let c = Colors::for_theme(dark);
        let painter = ui.painter();
        let rect = EguiRect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h));
        let radius = CornerRadius::same(RADIUS as u8);

        painter.rect_filled(rect, radius, c.bg);
        painter.rect_stroke(rect, radius, Stroke::new(1.0, c.border), StrokeKind::Inside);

        let mut top = PAD_Y;
        for (i, e) in self.entries.iter().enumerate() {
            if self.hover == Some(i) {
                let hr = EguiRect::from_min_size(egui::pos2(4.0, top), egui::vec2(w - 8.0, ITEM_H));
                painter.rect_filled(hr, CornerRadius::same(6), c.hover);
            }
            painter.text(
                egui::pos2(PAD_X, top + ITEM_H / 2.0),
                Align2::LEFT_CENTER,
                &e.label,
                FontId::proportional(13.0),
                if e.danger { c.danger } else { c.ink },
            );
            top += ITEM_H;
            if e.sep_after {
                let y = top + SEP_H / 2.0;
                painter.line_segment(
                    [egui::pos2(PAD_X, y), egui::pos2(w - PAD_X, y)],
                    Stroke::new(1.0, c.sep),
                );
                top += SEP_H;
            }
        }
    }
}

/// 粗估一行标签的像素宽度（`FontId::proportional(13.0)`）。
///
/// 不做精确测量：菜单宽度只要"够放下最长标签且不夸张地宽"就行。
/// 精确测量需要字体句柄，而这里要能在单元测试里跑。
fn estimate_width(s: &str) -> f32 {
    let mut w = 0.0;
    for c in s.chars() {
        // CJK / 全角按 13 px，其余按 7 px。菜单里中英混排很常见。
        w += if (c as u32) >= 0x2E80 { 13.0 } else { 7.0 };
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;
    use minidock_core::item::Item;

    fn item(kind: ItemKind, name: &str) -> Item {
        Item::new(kind, name, "")
    }

    /// 普通应用：打开 / 加分割线 / 移除。
    #[test]
    fn file_menu_has_open_and_remove() {
        let it = item(ItemKind::File, "记事本");
        let m = Menu::build(Some(&it), false).expect("应当有菜单");
        let cmds: Vec<_> = m.entries.iter().map(|e| e.cmd).collect();
        assert_eq!(
            cmds,
            vec![
                MenuCmd::Open,
                MenuCmd::AddSepAfter,
                MenuCmd::RemoveItem,
                MenuCmd::Settings,
            ]
        );
        assert!(!m.entries[0].danger);
        assert!(m.entries[1].sep_after, "加分割线之后要画分隔");
        assert!(m.entries[2].danger, "移除类必须是警告色");
    }

    /// 运行中的普通应用：首项改为"显示 / 最小化"，并多出"退出应用"。
    #[test]
    fn running_file_menu_offers_toggle_and_quit() {
        let it = item(ItemKind::File, "记事本");
        let m = Menu::build(Some(&it), true).expect("应当有菜单");
        assert_eq!(m.entries[0].label, "显示 / 最小化");
        assert!(m.entries.iter().any(|e| e.cmd == MenuCmd::QuitApp));
    }

    /// 没在跑的应用不该出现"退出应用"。
    ///
    /// 出现了用户就会去点，然后什么也不会发生 —— 比没有这一项更糟。
    #[test]
    fn idle_file_menu_has_no_quit() {
        let it = item(ItemKind::File, "记事本");
        let m = Menu::build(Some(&it), false).expect("应当有菜单");
        assert!(!m.entries.iter().any(|e| e.cmd == MenuCmd::QuitApp));
    }

    /// 只有文件夹才有"在资源管理器中打开"。
    #[test]
    fn only_folders_open_in_explorer() {
        let folder = item(ItemKind::Folder, "下载");
        assert!(Menu::build(Some(&folder), false)
            .expect("应当有菜单")
            .entries
            .iter()
            .any(|e| e.cmd == MenuCmd::OpenInExplorer));
        let file = item(ItemKind::File, "记事本");
        assert!(!Menu::build(Some(&file), false)
            .expect("应当有菜单")
            .entries
            .iter()
            .any(|e| e.cmd == MenuCmd::OpenInExplorer));
    }

    /// 空白处 = 对 Dock 本身说话：设置 + 退出。
    #[test]
    fn blank_area_offers_settings_and_exit() {
        let m = Menu::build(None, false).expect("应当有菜单");
        let cmds: Vec<MenuCmd> = m.entries.iter().map(|e| e.cmd).collect();
        assert_eq!(cmds, vec![MenuCmd::ExitDock, MenuCmd::Settings]);
        // 设置与退出之间要有分隔线：两者后果差别太大，挨在一起容易误点。
        assert!(m.entries[0].danger);
        assert!(m.entries[0].sep_after);
    }

    /// 分割线只给"删除"，外加总在最后的一项"Dock 设置"。
    #[test]
    fn separator_offers_delete_and_settings() {
        let sep = item(ItemKind::Separator, "");
        let m = Menu::build(Some(&sep), false).expect("应当有菜单");
        let cmds: Vec<_> = m.entries.iter().map(|e| e.cmd).collect();
        assert_eq!(cmds, vec![MenuCmd::RemoveSep, MenuCmd::Settings]);
        assert!(m.entries[0].danger);
    }

    /// 命中与绘制必须用同一套纵坐标。
    ///
    /// 分隔线占的高度也要算进去：漏了的话，菜单里出现分隔线之后下面的项
    /// 全部错位 —— 看到的是 A、点到的是 B。
    ///
    /// 这里手工造一个带分隔的菜单：`build` 目前不产生分隔（那属于
    /// 带"从 Dock 移除"的那一版），但布局代码必须现在就是对的。
    #[test]
    fn hit_matches_layout_with_separators() {
        let m = Menu {
            entries: vec![
                MenuEntry::new("打开", MenuCmd::Open, false, false),
                MenuEntry::new("退出应用", MenuCmd::QuitApp, false, true),
                MenuEntry::new("移除", MenuCmd::ExitDock, true, false),
            ],
            hover: None,
        };
        assert_eq!(m.hit(10.0, PAD_Y + 1.0), Some(0));
        assert_eq!(m.hit(10.0, PAD_Y + ITEM_H + 1.0), Some(1));
        // 第 1 项之后才是分隔带，那一段不可点。
        let sep_band = PAD_Y + ITEM_H * 2.0 + 2.0;
        assert_eq!(m.hit(10.0, sep_band), None, "分隔带不该命中任何项");
        // 第 2 项从分隔带之后开始。
        let third_top = PAD_Y + ITEM_H * 2.0 + SEP_H;
        assert_eq!(m.hit(10.0, third_top + 1.0), Some(2));
        // 横向超出菜单宽度不算命中。
        let (w, _) = m.logical_size();
        assert_eq!(m.hit(w + 5.0, third_top + 1.0), None);
        // 底部之外也不算。
        assert_eq!(m.hit(10.0, 1000.0), None);
    }

    /// 菜单高度必须等于各项累加（含分隔），否则窗口尺寸与实际内容不符。
    #[test]
    fn menu_height_covers_all_entries() {
        let m = Menu {
            entries: vec![
                MenuEntry::new("打开", MenuCmd::Open, false, false),
                MenuEntry::new("退出应用", MenuCmd::QuitApp, false, true),
                MenuEntry::new("移除", MenuCmd::ExitDock, true, false),
            ],
            hover: None,
        };
        let (_, h) = m.logical_size();
        let expect = PAD_Y * 2.0 + ITEM_H * 3.0 + SEP_H;
        assert!((h - expect).abs() < 0.01, "实际 {h}");
    }

    /// 屏幕底部放不下时向上翻。
    ///
    /// 底边停靠的 Dock 菜单**总是**这种情形：光标在屏幕最下方，
    /// 向下弹会整块顶出屏幕。
    #[test]
    fn place_flips_up_near_bottom_edge() {
        let size = (200.0, 240.0);
        let bounds = (0.0, 0.0, 2560.0, 1600.0);
        let (x, y) = Menu::place((1200.0, 1500.0), size, bounds);
        assert_eq!(x, 1200.0);
        assert!((y - (1500.0 - 240.0)).abs() < 0.01, "应当翻到光标上方");
    }

    /// 屏幕右边缘要往左收，不能顶出去。
    #[test]
    fn place_clamps_to_right_edge() {
        let size = (200.0, 100.0);
        let bounds = (0.0, 0.0, 2560.0, 1600.0);
        let (x, _) = Menu::place((2500.0, 100.0), size, bounds);
        assert!((x - (2560.0 - 200.0)).abs() < 0.01);
    }

    /// 下方放不下、翻到上方也放不下时，贴住工作区顶部。
    ///
    /// 宁可压住菜单里的内容（用户还看得到大部分），也不要跑到屏幕外 ——
    /// 那样连点都点不到。
    #[test]
    fn place_clamps_to_top_edge() {
        let size = (200.0, 1200.0);
        let bounds = (0.0, 60.0, 2560.0, 1600.0);
        // 1200 + 1200 > 1600 → 翻到上方 → 1200-1200 = 0 < 60 → 贴顶 60。
        let (_, y) = Menu::place((500.0, 1200.0), size, bounds);
        assert!((y - 60.0).abs() < 0.01, "实际 {y}");
    }

    /// 放得下时不该动位置。
    #[test]
    fn place_keeps_position_when_it_fits() {
        let size = (200.0, 300.0);
        let bounds = (0.0, 60.0, 2560.0, 1600.0);
        let (x, y) = Menu::place((500.0, 700.0), size, bounds);
        assert_eq!((x, y), (500.0, 700.0));
    }

    /// 中文标签要比纯英文宽，否则长长的中文项会被截。
    #[test]
    fn chinese_labels_are_wider_than_latin() {
        assert!(estimate_width("在资源管理器中打开") > estimate_width("open in explorer"));
        let m = Menu::build(None, false).expect("应当有菜单");
        let (w, _) = m.logical_size();
        assert!(w >= MIN_W, "宽度不小于最小值，实际 {w}");
    }

    /// 高亮变化要能报告出来（主循环靠它决定重绘）。
    #[test]
    fn set_hover_reports_change() {
        let mut m = Menu::build(None, false).expect("应当有菜单");
        assert!(m.set_hover(Some(0)));
        assert!(!m.set_hover(Some(0)), "同一个下标不算变化");
        assert!(m.set_hover(None));
    }

    /// 命令查询越界返回 `None`，不能 panic。
    #[test]
    fn cmd_at_is_bounds_checked() {
        let m = Menu::build(None, false).expect("应当有菜单");
        assert_eq!(m.cmd_at(0), Some(MenuCmd::ExitDock));
        assert_eq!(m.cmd_at(9), None);
    }
}
