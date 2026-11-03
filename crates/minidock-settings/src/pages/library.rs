//! 图标库页：从文件夹批量设置图标。

use egui::{RichText, Ui};

use super::widgets::{button, card, Ctx};
use super::Command;

pub(super) fn page(ctx: &mut Ctx<'_>, ui: &mut Ui) {
    card(ctx, ui, "从文件夹批量设置图标", "", |ctx, ui| {
        ui.horizontal(|ui| {
            button(ctx, ui, "选择图标文件夹…", Command::PickIconDir);
            button(ctx, ui, "自动匹配并应用", Command::AutoMatch);
        });
        ui.label(
            RichText::new(
                "选择存放图标（png / ico / jpg）的文件夹，文件名与应用名称相同即可自动匹配。",
            )
            .small()
            .weak(),
        );
    });
}
