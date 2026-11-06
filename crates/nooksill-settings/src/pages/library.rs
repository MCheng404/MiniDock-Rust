//! 图标库页：从文件夹批量设置图标。

use egui::{RichText, Ui};

use super::widgets::{button, card, Ctx};
use super::Command;

pub(super) fn page(ctx: &mut Ctx<'_>, ui: &mut Ui) {
    card(ctx, ui, "从文件夹批量设置图标", "", |ctx, ui| {
        ui.horizontal(|ui| {
            button(ctx, ui, "选择图标文件夹…", Command::PickIconDir);
            let picked = !ctx.state.icon_dir.is_empty();
            if picked {
                button(ctx, ui, "自动匹配并应用", Command::AutoMatch);
            }
        });
        if ctx.state.icon_dir.is_empty() {
            ui.label(RichText::new("还没选文件夹。").small().weak());
        } else {
            ui.label(
                RichText::new(format!("图标文件夹：{}", ctx.state.icon_dir))
                    .small()
                    .weak(),
            );
        }
        ui.label(
            RichText::new(
                "文件名与应用名相同即可匹配，扩展名认 .ico（Windows 图标格式，可直接进图像列表）。",
            )
            .small()
            .weak(),
        );
    });
}
