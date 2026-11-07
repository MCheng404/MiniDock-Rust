//! 条目页：管理 Dock 栏上的东西。
//!
//! 这一页是"条目"这件事的**唯一完整入口**。此前条目只能加、不能改：
//! 名字定死、顺序只能靠拖、删一个就找不回来。分组更是只有一个空壳 ——
//! 设置页能建一个组，却没有任何办法往里放东西。
//!
//! 三块内容：
//!
//! | 卡片 | 作用 |
//! |---|---|
//! | Dock 条目 | 列出全部条目：改名、改路径与参数、排序、删除 |
//! | 分组 | 列出全部分组：往里放条目、把条目捞出来、解散分组 |
//! | 新建 | 建组、加分割线、加回收站 |
//!
//! 条目列表由主程序每次同步进来（[`super::SettingsState::items`]）——
//! 设置界面不持有事实来源，只有一份快照。

use egui::{Align, Id, Layout, RichText, Sense, Ui};

use nooksill_core::item::{Item, ItemKind};

use super::widgets::{card, small_button, Ctx};
use super::{Command, ItemEdit};
use crate::text_field::{text_field, FieldOpts, FIELD_H};

/// 条目类型的徽标文字。
fn kind_tag(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::File => "应用",
        ItemKind::Folder => "文件夹",
        ItemKind::Url => "网页",
        ItemKind::Separator => "分割线",
        ItemKind::Group => "分组",
        ItemKind::Trash => "回收站",
        ItemKind::Note => "速记",
    }
}

/// 条目的第二行说明（路径 / 子项数）。
fn detail(item: &Item) -> String {
    match item.kind {
        ItemKind::Separator => "分隔栏体".to_owned(),
        ItemKind::Group => format!("{} 个子项", item.kids.len()),
        ItemKind::Trash => "系统回收站".to_owned(),
        _ => {
            if item.path.is_empty() {
                "（没有目标）".to_owned()
            } else {
                item.path.clone()
            }
        }
    }
}

pub(super) fn page(ctx: &mut Ctx<'_>, ui: &mut Ui) {
    items_card(ctx, ui);
    groups_card(ctx, ui);
    create_card(ctx, ui);
}

/// Dock 条目：改名、改路径与参数、排序、删除。
fn items_card(ctx: &mut Ctx<'_>, ui: &mut Ui) {
    card(
        ctx,
        ui,
        "Dock 条目",
        "栏体上的东西都在这里。改完立刻生效并写盘。",
        |ctx, ui| {
            if ctx.state.items.is_empty() {
                ui.label(
                    RichText::new("还没有条目。到「添加」页加几个，或直接把文件拖到 Dock 上。")
                        .small()
                        .weak(),
                );
                return;
            }
            // 倒序画：最上面的就是最靠近 Dock 左端的那一项。
            // 列表是"从上到下 = 从左到右"，所以索引要反过来取。
            let n = ctx.state.items.len();
            for i in (0..n).rev() {
                let item = ctx.state.items[i].clone();
                item_row(ctx, ui, i, &item);
            }
        },
    );
}

/// 一个条目的行。
fn item_row(ctx: &mut Ctx<'_>, ui: &mut Ui, index: usize, item: &Item) {
    let editing = ctx.state.editing == Some(index);
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_max_width(ui.available_width() - 190.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("[{}]", kind_tag(item.kind)))
                        .small()
                        .weak(),
                );
                let name = if item.name.is_empty() {
                    "（未命名）"
                } else {
                    item.name.as_str()
                };
                ui.label(RichText::new(name).strong());
            });
            ui.label(RichText::new(detail(item)).small().weak());
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if small_button(ui, "删除", "从 Dock 移除这个条目") {
                ctx.out.command = Some(Command::RemoveItem(index));
            }
            if small_button(ui, "编辑", "改名、改路径与启动参数") {
                begin_edit(ctx, index, item);
            }
            if small_button(ui, "↓", "往后移一位") {
                ctx.out.command = Some(Command::MoveItem(index, 1));
            }
            if small_button(ui, "↑", "往前移一位") {
                ctx.out.command = Some(Command::MoveItem(index, -1));
            }
        });
    });
    ui.add_space(6.0);

    if editing {
        edit_form(ctx, ui, index, item);
    }
}

/// 进入编辑态：把当前值抄进缓冲区。
fn begin_edit(ctx: &mut Ctx<'_>, index: usize, item: &Item) {
    ctx.state.editing = Some(index);
    ctx.state.edit_name = item.name.clone();
    ctx.state.edit_path = item.path.clone();
    ctx.state.edit_args = item.args.clone();
    ctx.state.edit_group = None;
}

/// 编辑表单：名称 / 路径 / 参数 / 归属分组。
fn edit_form(ctx: &mut Ctx<'_>, ui: &mut Ui, index: usize, item: &Item) {
    let groups: Vec<(usize, String)> = ctx
        .state
        .items
        .iter()
        .enumerate()
        .filter(|(i, it)| it.kind == ItemKind::Group && *i != index)
        .map(|(i, it)| (i, it.name.clone()))
        .collect();

    ui.push_id(index, |ui| {
        egui::Grid::new("item-edit")
            .num_columns(2)
            .spacing([10.0, 8.0])
            .show(ui, |ui| {
                ui.label("名称");
                let w = ui.available_width().max(140.0);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(w, FIELD_H), Sense::hover());
                text_field(
                    ui,
                    rect,
                    Id::unique("item-edit-name"),
                    &mut ctx.state.edit_name,
                    &FieldOpts {
                        hint: "显示在气泡与分组面板里的名字",
                        ..FieldOpts::default()
                    },
                );
                ui.end_row();

                ui.label("目标");
                let w = ui.available_width().max(140.0);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(w, FIELD_H), Sense::hover());
                let path_hint = match item.kind {
                    ItemKind::Url => "https://example.com",
                    ItemKind::Trash => "回收站不需要目标",
                    _ => "程序路径、文件夹路径",
                };
                text_field(
                    ui,
                    rect,
                    Id::unique("item-edit-path"),
                    &mut ctx.state.edit_path,
                    &FieldOpts {
                        hint: path_hint,
                        ..FieldOpts::default()
                    },
                );
                ui.end_row();

                ui.label("参数");
                let w = ui.available_width().max(140.0);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(w, FIELD_H), Sense::hover());
                text_field(
                    ui,
                    rect,
                    Id::unique("item-edit-args"),
                    &mut ctx.state.edit_args,
                    &FieldOpts {
                        hint: "启动时附加的命令行参数（可留空）",
                        ..FieldOpts::default()
                    },
                );
                ui.end_row();

                let group_label = ui.label("归组");
                egui::ComboBox::from_id_salt("item-edit-group")
                    .selected_text(match ctx.state.edit_group {
                        None => "留在栏体上".to_owned(),
                        Some(g) => groups
                            .iter()
                            .find(|(i, _)| *i == g)
                            .map_or_else(|| "留在栏体上".to_owned(), |(_, n)| n.clone()),
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut ctx.state.edit_group, None, "留在栏体上");
                        for (i, name) in &groups {
                            ui.selectable_value(&mut ctx.state.edit_group, Some(*i), name);
                        }
                    })
                    .response
                    // AccessKit 不会把相邻 label 当成控件的名字。少了这一句，
                    // 读屏软件看到的是一个没有名字的下拉框，kittest 也会拦下。
                    .labelled_by(group_label.id);
                ui.end_row();
            });

        // 图标行**不进 Grid**：一行里塞「路径文字 + 两个按钮」会把
        // Grid 的第二列撑宽，下一帧回来算列宽时其它行的
        // `available_width() - 60` 会变负（egui 0.37 直接 panic）。
        //
        // 只列 shell 能给图标的格式（`.ico` / `.exe` / `.dll`）——
        // PNG/JPEG 还没有解码器，放进过滤器会挑到一个用不了的文件。
        ui.horizontal(|ui| {
            ui.label("图标：");
            let shown = if item.icon_src.is_empty() {
                "默认图标".to_owned()
            } else {
                let short: String = item
                    .icon_src
                    .chars()
                    .rev()
                    .take(20)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                format!("…{short}")
            };
            ui.label(RichText::new(shown).small().weak());
            if small_button(ui, "更换…", "选一个 .ico / .exe / .dll 作为图标") {
                ctx.out.command = Some(Command::PickIcon(index));
            }
            if !item.icon_src.is_empty() && small_button(ui, "恢复默认", "清掉自定义图标")
            {
                ctx.out.command = Some(Command::ResetIcon(index));
            }
        });

        ui.horizontal(|ui| {
            if small_button(ui, "保存", "写回这个条目") {
                let edit = ItemEdit {
                    name: ctx.state.edit_name.trim().to_owned(),
                    path: ctx.state.edit_path.trim().to_owned(),
                    args: ctx.state.edit_args.trim().to_owned(),
                    group: ctx.state.edit_group,
                };
                ctx.out.command = Some(Command::EditItem(index, edit));
                ctx.state.editing = None;
            }
            if small_button(ui, "取消", "放弃改动") {
                ctx.state.editing = None;
            }
        });
    });
    ui.add_space(8.0);
}

/// 分组：往里放东西、把东西捞出来、解散分组。
fn groups_card(ctx: &mut Ctx<'_>, ui: &mut Ui) {
    card(
        ctx,
        ui,
        "分组",
        "把散落的条目收进一个组，组可以当成一个图标放在栏体上。",
        |ctx, ui| {
            let groups: Vec<(usize, Item)> = ctx
                .state
                .items
                .iter()
                .enumerate()
                .filter(|(_, it)| it.kind == ItemKind::Group)
                .map(|(i, it)| (i, it.clone()))
                .collect();
            if groups.is_empty() {
                ui.label(
                    RichText::new("还没有分组。下面的「新建」里可以建一个。")
                        .small()
                        .weak(),
                );
                return;
            }
            for (gi, group) in groups {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&group.name).strong());
                    ui.label(
                        RichText::new(format!("{} 个子项", group.kids.len()))
                            .small()
                            .weak(),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if small_button(ui, "解散", "子项全部放回栏体，分组本身消失")
                        {
                            ctx.out.command = Some(Command::DissolveGroup(gi));
                        }
                    });
                });
                if group.kids.is_empty() {
                    ui.label(
                        RichText::new("空分组。在上面选一个条目，编辑时把「归组」改成它。")
                            .small()
                            .weak(),
                    );
                } else {
                    for (ki, kid) in group.kids.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("· {}", display_name(kid))).small());
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if small_button(ui, "移出", "把这个子项放回栏体") {
                                    ctx.out.command = Some(Command::ExtractKid(gi, ki));
                                }
                                // 用文字而不是「↑/↓」：条目行里已经有同名的箭头按钮，
                                // 两个列表摆在同一页上，同名会让人分不清点的是哪一个。
                                if ki + 1 < group.kids.len()
                                    && small_button(ui, "下移", "在分组里往后移一位")
                                {
                                    ctx.out.command = Some(Command::MoveKid(gi, ki, 1));
                                }
                                if ki > 0 && small_button(ui, "上移", "在分组里往前移一位")
                                {
                                    ctx.out.command = Some(Command::MoveKid(gi, ki, -1));
                                }
                            });
                        });
                    }
                }
                ui.add_space(6.0);
            }
        },
    );
}

/// 新建：组、分割线、回收站。
fn create_card(ctx: &mut Ctx<'_>, ui: &mut Ui) {
    card(ctx, ui, "新建", "", |ctx, ui| {
        ui.horizontal_wrapped(|ui| {
            if small_button(ui, "新建分组…", "建一个空分组，再把条目放进去") {
                ctx.out.command = Some(Command::NewGroup);
            }
            if small_button(ui, "添加分割线", "在栏体末尾加一条分隔线") {
                ctx.out.command = Some(Command::AddSeparator);
            }
            if small_button(ui, "添加回收站", "加一个系统回收站入口") {
                ctx.out.command = Some(Command::AddTrash);
            }
        });
    });
}

/// 条目的显示名（空名退回类型名）。
fn display_name(item: &Item) -> String {
    if item.name.is_empty() {
        kind_tag(item.kind).to_owned()
    } else {
        item.name.clone()
    }
}
