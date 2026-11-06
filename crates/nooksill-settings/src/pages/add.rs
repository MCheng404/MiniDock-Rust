//! 添加页：加条目、导入导出、重置。

use egui::{Id, Sense, Ui};

use super::widgets::{button, card, Ctx};
use super::Command;
use crate::text_field::{text_field, FieldOpts, FIELD_H};

pub(super) fn page(ctx: &mut Ctx<'_>, ui: &mut Ui) {
    card(ctx, ui, "添加", "", |ctx, ui| {
        ui.horizontal_wrapped(|ui| {
            button(ctx, ui, "添加应用 / 文件…", Command::AddFile);
            button(ctx, ui, "添加文件夹…", Command::AddFolder);
            button(ctx, ui, "新建应用组", Command::NewGroup);
            button(ctx, ui, "添加回收站", Command::AddTrash);
            button(ctx, ui, "添加分割线", Command::AddSeparator);
        });
    });

    // 网址不做模态输入框：设置窗口本来就在前台，多一层对话框只是多一次点击。
    // 输入框 + 「添加」按钮，回车也能提交。
    card(
        ctx,
        ui,
        "添加网页应用",
        "地址会以应用窗口或默认浏览器打开。",
        |ctx, ui| {
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 92.0).max(160.0);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(width, FIELD_H), Sense::hover());
                text_field(
                    ui,
                    rect,
                    Id::unique("add-url-field"),
                    &mut ctx.state.url_input,
                    &FieldOpts {
                        hint: "https://example.com",
                        clearable: true,
                        ..FieldOpts::default()
                    },
                );
                let ready = !ctx.state.url_input.trim().is_empty();
                if ready {
                    button(ctx, ui, "添加", Command::AddUrl);
                }
                let entered = rect
                    .contains(ui.input(|i| i.pointer.hover_pos().unwrap_or_default()))
                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ready && entered {
                    ctx.out.command = Some(Command::AddUrl);
                }
            });
            ui.label(
                egui::RichText::new("例：https://github.com —— 名称取域名，图标用首字占位。")
                    .small()
                    .weak(),
            );
        },
    );

    card(
        ctx,
        ui,
        "导入与导出",
        "导出的是整份 ini（含条目与分组）。",
        |ctx, ui| {
            ui.horizontal(|ui| {
                button(ctx, ui, "导出完整配置…", Command::ExportAll);
                button(ctx, ui, "导入配置…", Command::Import);
            });
        },
    );

    card(ctx, ui, "重置", "", |ctx, ui| {
        button(ctx, ui, "恢复默认设置", Command::ResetDefaults);
    });
}
