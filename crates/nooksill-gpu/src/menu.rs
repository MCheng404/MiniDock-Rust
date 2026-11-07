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
//! 模型是阻塞式自绘菜单：打开 → 选一项或取消 → 返回命令。

use nooksill_core::item::{Item, ItemKind};

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
    /// 切换开机自启（菜单里的勾选项）。
    ToggleAutostart,
    /// 空白处右键的「新建」：加一个应用/文件。
    AddFile,
    /// 空白处右键的「新建」：加一个已有的文件夹。
    AddFolder,
    /// 空白处右键的「新建」：在磁盘上**新建**一个文件夹并加进 Dock。
    NewFolder,
    /// 空白处右键的「新建」：建一个分组（目录）。
    NewGroup,
    /// 重命名 / 改目标与参数（编辑弹窗）。
    Rename,
    /// 复制一份这个条目。
    Duplicate,
    /// 打开分组（弹出它的内容）。
    OpenGroup,
    /// 解散分组：子项摊平回栏体。
    DissolveGroup,
    /// 给这个条目挑一个自定义图标文件。
    ChangeIcon,
    /// 清掉这个条目的自定义图标。
    ResetIcon,
    /// 切换 Dock 的显示 / 隐藏（托盘菜单用）。
    ToggleDock,
    /// 打开速记面板。
    Notes,
    /// 打开起动台。
    Launchpad,
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
    /// 这一项带勾选态（例如「开机自启」）。只要有这么一项，整张菜单都
    /// 留出勾选槽 —— 否则同一张菜单里两种左边距，看起来像没对齐。
    pub checkable: bool,
    /// 当前是否勾上。
    pub checked: bool,
}

impl MenuEntry {
    /// 带勾选态的一项。`on` 是当前是否开启。
    fn toggle(label: &str, cmd: MenuCmd, on: bool) -> Self {
        Self {
            checkable: true,
            checked: on,
            ..Self::new(label, cmd, false, false)
        }
    }

    fn new(label: &str, cmd: MenuCmd, danger: bool, sep_after: bool) -> Self {
        Self {
            label: label.to_owned(),
            cmd,
            danger,
            checkable: false,
            checked: false,
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
pub const RADIUS: f32 = 10.0;

/// 构建菜单时的开关，来自配置里的 `Mn*` 键。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuOpts {
    /// `MnAuto`：显示「开机自启」，值 = 当前是否开启。`None` = 不显示。
    pub autostart: Option<bool>,
    /// `MnAdd`：空白处右键显示「添加文件 / 文件夹 / 新建」。
    pub add_items: bool,
    /// `MnNotes`：空白处右键显示「速记」。
    pub notes: bool,
}

impl Default for MenuOpts {
    fn default() -> Self {
        // 默认值必须与 `MnAdd` / `MnNotes` 的 ini 默认一致（1 = 显示）。
        Self {
            autostart: None,
            add_items: true,
            notes: true,
        }
    }
}

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
    /// `temp` = 这个条目是"未固定的运行中应用"（临时条目）。它们不改配置，
    /// 所以不给"加分割线""从 Dock 移除"—— 那两条按配置下标改数据，
    /// 用临时条目的下标去打会改到别的条目上。
    pub fn build(item: Option<&Item>, running: bool, temp: bool, opts: MenuOpts) -> Option<Self> {
        let mut entries = Vec::new();
        match item.map(|i| i.kind) {
            None => {
                // `MnAdd`：空白处右键给"新建"。只放文件/文件夹 —— 网址要
                // 输入框，菜单里没法打字，那条留在设置页。
                if opts.add_items {
                    entries.push(MenuEntry::new("添加文件…", MenuCmd::AddFile, false, false));
                    entries.push(MenuEntry::new(
                        "添加文件夹…",
                        MenuCmd::AddFolder,
                        false,
                        false,
                    ));
                    entries.push(MenuEntry::new(
                        "新建文件夹…",
                        MenuCmd::NewFolder,
                        false,
                        true,
                    ));
                    entries.push(MenuEntry::new(
                        "新建应用组…",
                        MenuCmd::NewGroup,
                        false,
                        false,
                    ));
                }
                // `MnAuto`：开机自启的勾选项。状态由调用方传进来 —— 菜单层
                // 不碰注册表。
                if let Some(on) = opts.autostart {
                    entries.push(MenuEntry::toggle("开机自启", MenuCmd::ToggleAutostart, on));
                }
                // `MnNotes`：速记面板。与"Dock 上有没有速记图标"无关，
                // 这是个随时能打开的入口。
                if opts.notes {
                    entries.push(MenuEntry::new("速记", MenuCmd::Notes, false, false));
                }
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
            Some(ItemKind::Group) => {
                // 分组是一个"目录"：点开看内容、可以改名、可以解散。
                entries.push(MenuEntry::new("打开", MenuCmd::OpenGroup, false, false));
                entries.push(MenuEntry::new("重命名…", MenuCmd::Rename, false, false));
                entries.push(MenuEntry::new(
                    "解散分组",
                    MenuCmd::DissolveGroup,
                    true,
                    true,
                ));
                if !temp {
                    entries.push(MenuEntry::new(
                        "在右侧添加分割线",
                        MenuCmd::AddSepAfter,
                        false,
                        false,
                    ));
                    entries.push(MenuEntry::new(
                        "从 Dock 移除",
                        MenuCmd::RemoveItem,
                        true,
                        false,
                    ));
                }
            }
            Some(kind) => {
                // 运行中的文件条目给"显示 / 最小化"，其余给"打开"。
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
                if !temp {
                    entries.push(MenuEntry::new("重命名…", MenuCmd::Rename, false, false));
                    entries.push(MenuEntry::new("复制一份", MenuCmd::Duplicate, false, false));
                    // 图标只对"能取到 shell 图标"的类型有意义；网址/速记
                    // 目前走首字占位，给个换图标的入口等于多一个死按钮。
                    if matches!(kind, ItemKind::File | ItemKind::Folder | ItemKind::Trash) {
                        entries.push(MenuEntry::new(
                            "更换图标…",
                            MenuCmd::ChangeIcon,
                            false,
                            false,
                        ));
                        entries.push(MenuEntry::new(
                            "恢复默认图标",
                            MenuCmd::ResetIcon,
                            false,
                            false,
                        ));
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
        // 起动台排在设置前面：它是"日常用"的，设置是"折腾"的。
        entries.push(MenuEntry::new("起动台", MenuCmd::Launchpad, false, false));
        entries.push(MenuEntry::new("Dock 设置", MenuCmd::Settings, false, false));
        Some(Self {
            entries,
            hover: None,
        })
    }

    /// 托盘图标的右键菜单。
    ///
    /// 不用 Win32 的 `TrackPopupMenu`：那个 API 要求调用线程拥有前台窗口，
    /// 而 Dock 是 `WS_EX_NOACTIVATE`，实测菜单会"点不消失"。这里复用
    /// 已经验证过的自绘模态菜单窗口，行为与 Dock 右键完全一致。
    #[must_use]
    pub fn tray(hidden: bool) -> Self {
        let label = if hidden { "显示 Dock" } else { "隐藏 Dock" };
        Self {
            entries: vec![
                MenuEntry::new(label, MenuCmd::ToggleDock, false, false),
                MenuEntry::new("起动台", MenuCmd::Launchpad, false, false),
                MenuEntry::new("Dock 设置", MenuCmd::Settings, false, true),
                MenuEntry::new("退出 Dock", MenuCmd::ExitDock, true, false),
            ],
            hover: None,
        }
    }

    /// 勾选槽宽度：只要有可勾选项就留，保证同菜单内左边距一致。
    fn gutter(&self) -> f32 {
        if self.entries.iter().any(|e| e.checkable) {
            18.0
        } else {
            0.0
        }
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
        let gutter = self.gutter();
        for e in &self.entries {
            w = w.max(estimate_width(&e.label) + PAD_X * 2.0 + gutter);
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
    pub fn paint(&self, ui: &egui::Ui, dark: bool, frost: bool) {
        let (w, h) = self.logical_size();
        let c = Colors::for_theme(dark);
        let painter = ui.painter();
        let rect = EguiRect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h));
        let radius = CornerRadius::same(RADIUS as u8);

        painter.rect_filled(
            rect,
            radius,
            if frost {
                crate::frost::panel_bg(dark, true)
            } else {
                c.bg
            },
        );
        painter.rect_stroke(rect, radius, Stroke::new(1.0, c.border), StrokeKind::Inside);

        let mut top = PAD_Y;
        let gutter = self.gutter();
        for (i, e) in self.entries.iter().enumerate() {
            if self.hover == Some(i) {
                let hr = EguiRect::from_min_size(egui::pos2(4.0, top), egui::vec2(w - 8.0, ITEM_H));
                painter.rect_filled(hr, CornerRadius::same(6), c.hover);
            }
            if e.checked {
                // 勾是**画**出来的，不是打字符：`✓`(U+2713) 落在 Dingbats 区，
                // 我们的字体不保证有，缺字会变成豆腐块 —— 画两条线段没有这个风险。
                let cy = top + ITEM_H / 2.0;
                let x0 = PAD_X + 2.0;
                painter.line_segment(
                    [egui::pos2(x0, cy), egui::pos2(x0 + 4.0, cy + 4.0)],
                    Stroke::new(1.6, c.ink),
                );
                painter.line_segment(
                    [
                        egui::pos2(x0 + 4.0, cy + 4.0),
                        egui::pos2(x0 + 11.0, cy - 4.5),
                    ],
                    Stroke::new(1.6, c.ink),
                );
            }
            painter.text(
                egui::pos2(PAD_X + gutter, top + ITEM_H / 2.0),
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
    use nooksill_core::item::Item;

    fn item(kind: ItemKind, name: &str) -> Item {
        Item::new(kind, name, "")
    }

    /// 普通应用：打开 / 重命名 / 复制 / 加分割线 / 移除。
    #[test]
    fn file_menu_has_open_and_remove() {
        let it = item(ItemKind::File, "记事本");
        let m = Menu::build(Some(&it), false, false, MenuOpts::default()).expect("应当有菜单");
        let cmds: Vec<_> = m.entries.iter().map(|e| e.cmd).collect();
        assert_eq!(
            cmds,
            vec![
                MenuCmd::Open,
                MenuCmd::Rename,
                MenuCmd::Duplicate,
                MenuCmd::ChangeIcon,
                MenuCmd::ResetIcon,
                MenuCmd::AddSepAfter,
                MenuCmd::RemoveItem,
                MenuCmd::Launchpad,
                MenuCmd::Settings,
            ]
        );
        assert!(!m.entries[0].danger);
        assert!(m.entries[5].sep_after, "加分割线之后要画分隔");
        assert!(m.entries[6].danger, "移除类必须是警告色");
    }

    /// `MnNotes`：空白处右键的「速记」入口受开关控制。
    #[test]
    fn notes_entry_follows_the_switch() {
        let opts = MenuOpts {
            autostart: None,
            add_items: false,
            notes: true,
        };
        let on = Menu::build(None, false, false, opts).expect("应当有菜单");
        assert!(on.entries.iter().any(|e| e.cmd == MenuCmd::Notes));
        let off = Menu::build(
            None,
            false,
            false,
            MenuOpts {
                notes: false,
                ..opts
            },
        )
        .expect("应当有菜单");
        assert!(
            !off.entries.iter().any(|e| e.cmd == MenuCmd::Notes),
            "关掉开关后不该留一个死入口"
        );
    }

    /// 托盘菜单：隐藏时给「显示 Dock」，且不带 Dock 条目的那堆新建命令。
    #[test]
    fn tray_menu_offers_toggle_settings_and_exit() {
        let hidden = Menu::tray(true);
        let cmds: Vec<_> = hidden.entries.iter().map(|e| e.cmd).collect();
        assert_eq!(
            cmds,
            vec![
                MenuCmd::ToggleDock,
                MenuCmd::Launchpad,
                MenuCmd::Settings,
                MenuCmd::ExitDock,
            ]
        );
        assert_eq!(hidden.entries[0].label, "显示 Dock");
        assert_eq!(Menu::tray(false).entries[0].label, "隐藏 Dock");
        assert!(!hidden.entries.iter().any(|e| e.cmd == MenuCmd::AddFile));
    }

    /// 分组：能打开、能改名、能解散 —— 它是一个"目录"。
    #[test]
    fn group_menu_offers_open_rename_and_dissolve() {
        let g = item(ItemKind::Group, "工具");
        let m = Menu::build(Some(&g), false, false, MenuOpts::default()).expect("应当有菜单");
        let cmds: Vec<_> = m.entries.iter().map(|e| e.cmd).collect();
        assert_eq!(
            cmds,
            vec![
                MenuCmd::OpenGroup,
                MenuCmd::Rename,
                MenuCmd::DissolveGroup,
                MenuCmd::AddSepAfter,
                MenuCmd::RemoveItem,
                MenuCmd::Launchpad,
                MenuCmd::Settings,
            ]
        );
        assert!(
            m.entries
                .iter()
                .any(|e| e.cmd == MenuCmd::DissolveGroup && e.danger),
            "解散是不可逆操作，要用警告色"
        );
        assert!(
            !m.entries.iter().any(|e| e.cmd == MenuCmd::Open),
            "组不能走\"打开应用\"那条路"
        );
    }

    /// 运行中的普通应用：首项改为"显示 / 最小化"，并多出"退出应用"。
    #[test]
    fn running_file_menu_offers_toggle_and_quit() {
        let it = item(ItemKind::File, "记事本");
        let m = Menu::build(Some(&it), true, false, MenuOpts::default()).expect("应当有菜单");
        assert_eq!(m.entries[0].label, "显示 / 最小化");
        assert!(m.entries.iter().any(|e| e.cmd == MenuCmd::QuitApp));
    }

    /// 没在跑的应用不该出现"退出应用"。
    ///
    /// 出现了用户就会去点，然后什么也不会发生 —— 比没有这一项更糟。
    #[test]
    fn idle_file_menu_has_no_quit() {
        let it = item(ItemKind::File, "记事本");
        let m = Menu::build(Some(&it), false, false, MenuOpts::default()).expect("应当有菜单");
        assert!(!m.entries.iter().any(|e| e.cmd == MenuCmd::QuitApp));
    }

    /// 只有文件夹才有"在资源管理器中打开"。
    #[test]
    fn only_folders_open_in_explorer() {
        let folder = item(ItemKind::Folder, "下载");
        assert!(
            Menu::build(Some(&folder), false, false, MenuOpts::default())
                .expect("应当有菜单")
                .entries
                .iter()
                .any(|e| e.cmd == MenuCmd::OpenInExplorer)
        );
        let file = item(ItemKind::File, "记事本");
        assert!(!Menu::build(Some(&file), false, false, MenuOpts::default())
            .expect("应当有菜单")
            .entries
            .iter()
            .any(|e| e.cmd == MenuCmd::OpenInExplorer));
    }

    /// 空白处 = 对 Dock 本身说话：退出 + 起动台 + 设置。
    #[test]
    fn blank_area_offers_settings_and_exit() {
        // 关掉两个 `Mn*` 开关，看菜单的最小集合。
        let opts = MenuOpts {
            autostart: None,
            add_items: false,
            notes: false,
        };
        let m = Menu::build(None, false, false, opts).expect("应当有菜单");
        let cmds: Vec<MenuCmd> = m.entries.iter().map(|e| e.cmd).collect();
        assert_eq!(
            cmds,
            vec![MenuCmd::ExitDock, MenuCmd::Launchpad, MenuCmd::Settings]
        );
        // 设置与退出之间要有分隔线：两者后果差别太大，挨在一起容易误点。
        assert!(m.entries[0].danger);
        assert!(m.entries[0].sep_after);
    }

    /// `MnAdd`：空白处右键给「添加文件 / 文件夹」。
    #[test]
    fn blank_menu_offers_new_items_when_enabled() {
        let m = Menu::build(
            None,
            false,
            false,
            MenuOpts {
                autostart: None,
                add_items: true,
                notes: false,
            },
        )
        .expect("应当有菜单");
        let labels: Vec<&str> = m.entries.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(
            &labels[..4],
            &["添加文件…", "添加文件夹…", "新建文件夹…", "新建应用组…"]
        );
        assert!(labels.contains(&"退出 Dock"));
        // "新建文件夹"与"新建应用组"之间要断开：一个是磁盘上的目录，
        // 一个是 Dock 内部的目录，混在一起容易点错。
        assert!(m.entries[2].sep_after);
    }

    /// `MnAuto`：勾选项带上"当前是否开启"，并给整张菜单留出勾选槽。
    #[test]
    fn autostart_entry_reflects_state_and_reserves_a_gutter() {
        let on = Menu::build(
            None,
            false,
            false,
            MenuOpts {
                autostart: Some(true),
                add_items: false,
                notes: false,
            },
        )
        .expect("应当有菜单");
        let entry = on
            .entries
            .iter()
            .find(|e| e.cmd == MenuCmd::ToggleAutostart)
            .expect("应当有开机自启项");
        assert!(entry.checked, "开启时要显示勾");

        let off = Menu::build(
            None,
            false,
            false,
            MenuOpts {
                autostart: Some(false),
                add_items: false,
                notes: false,
            },
        )
        .expect("应当有菜单");
        let entry = off
            .entries
            .iter()
            .find(|e| e.cmd == MenuCmd::ToggleAutostart)
            .expect("应当有开机自启项");
        assert!(!entry.checked, "关闭时不该有勾");

        // 有可勾选项 → 整张菜单留出勾选槽（宽度是否变宽取决于 MIN_W，
        // 所以直接问 `gutter`，不去赌标签长度）。
        let narrow = Menu::build(
            None,
            false,
            false,
            MenuOpts {
                autostart: None,
                add_items: false,
                notes: false,
            },
        )
        .expect("应当有菜单");
        assert!(on.gutter() > 0.0, "有勾选项时要留槽");
        assert_eq!(narrow.gutter(), 0.0, "没有勾选项就不留");
    }

    /// 条目菜单不该被 `Mn*` 污染：那两个开关只管空白处的 Dock 菜单。
    #[test]
    fn item_menu_ignores_dock_level_options() {
        let it = item(ItemKind::File, "应用");
        let m = Menu::build(
            Some(&it),
            false,
            false,
            MenuOpts {
                autostart: Some(true),
                add_items: true,
                notes: false,
            },
        )
        .expect("应当有菜单");
        assert!(!m.entries.iter().any(|e| e.cmd == MenuCmd::AddFile));
        assert!(!m.entries.iter().any(|e| e.cmd == MenuCmd::ToggleAutostart));
    }

    /// 临时条目（运行中但没固定的应用）只给"显示 / 最小化"和"退出应用"。
    ///
    /// 结构性命令（加分割线 / 从 Dock 移除）是按**配置下标**改数据的，
    /// 而临时条目根本不在配置里 —— 放出来就会改到别人头上。
    #[test]
    fn temp_item_has_no_structural_commands() {
        let mut it = Item::new(ItemKind::File, "Chrome", r"C:\x\chrome.exe");
        it.running = true;
        let m = Menu::build(Some(&it), true, true, MenuOpts::default()).expect("应当有菜单");
        let has = |c: MenuCmd| m.entries.iter().any(|e| e.cmd == c);
        assert!(has(MenuCmd::Open), "临时条目必须能点击打开");
        assert!(has(MenuCmd::QuitApp), "运行中的条目要能退出");
        assert!(!has(MenuCmd::RemoveItem), "临时条目不在配置里，不能移除");
        assert!(!has(MenuCmd::AddSepAfter), "临时条目不能插分割线");
    }

    /// 分割线只给"删除"，外加总在最后的一项"Dock 设置"。
    #[test]
    fn separator_offers_delete_and_settings() {
        let sep = item(ItemKind::Separator, "");
        let m = Menu::build(Some(&sep), false, false, MenuOpts::default()).expect("应当有菜单");
        let cmds: Vec<_> = m.entries.iter().map(|e| e.cmd).collect();
        assert_eq!(
            cmds,
            vec![MenuCmd::RemoveSep, MenuCmd::Launchpad, MenuCmd::Settings]
        );
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
        let m = Menu::build(None, false, false, MenuOpts::default()).expect("应当有菜单");
        let (w, _) = m.logical_size();
        assert!(w >= MIN_W, "宽度不小于最小值，实际 {w}");
    }

    /// 高亮变化要能报告出来（主循环靠它决定重绘）。
    #[test]
    fn set_hover_reports_change() {
        let mut m = Menu::build(None, false, false, MenuOpts::default()).expect("应当有菜单");
        assert!(m.set_hover(Some(0)));
        assert!(!m.set_hover(Some(0)), "同一个下标不算变化");
        assert!(m.set_hover(None));
    }

    /// 命令查询越界返回 `None`，不能 panic。
    #[test]
    fn cmd_at_is_bounds_checked() {
        // 用最小集合（关掉两个 `Mn*`），下标才是稳定的。
        let opts = MenuOpts {
            autostart: None,
            add_items: false,
            notes: false,
        };
        let m = Menu::build(None, false, false, opts).expect("应当有菜单");
        assert_eq!(m.cmd_at(0), Some(MenuCmd::ExitDock));
        assert_eq!(m.cmd_at(9), None);
    }
}
