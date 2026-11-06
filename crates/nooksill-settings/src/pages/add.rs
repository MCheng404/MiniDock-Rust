//! 添加页：加条目、导入导出、重置。

use egui::Ui;

use super::widgets::{button, card, Ctx};
use super::Command;

pub(super) fn page(ctx: &mut Ctx<'_>, ui: &mut Ui) {
    card(ctx, ui, "添加", "", |ctx, ui| {
        ui.horizontal_wrapped(|ui| {
            button(ctx, ui, "添加网页应用…", Command::AddUrl);
            button(ctx, ui, "添加应用 / 文件…", Command::AddFile);
            button(ctx, ui, "添加文件夹…", Command::AddFolder);
            button(ctx, ui, "新建应用组", Command::NewGroup);
            button(ctx, ui, "添加回收站", Command::AddTrash);
            button(ctx, ui, "添加分割线", Command::AddSeparator);
        });
    });

    card(ctx, ui, "导入与导出", "", |ctx, ui| {
        ui.horizontal(|ui| {
            button(ctx, ui, "导出完整配置…", Command::ExportAll);
            button(ctx, ui, "导入配置…", Command::Import);
        });
    });

    card(ctx, ui, "重置", "", |ctx, ui| {
        button(ctx, ui, "恢复默认设置", Command::ResetDefaults);
    });
}
