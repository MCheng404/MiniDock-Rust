//! 图标页：底盘、分割线、运行指示、标签。

use egui::{RichText, Ui};
use minidock_core::config::Key;

use super::options::{DOT_OPTS, PLATE_OPTS, PLATE_THEME_OPTS};
use super::widgets::{card, combo, slider, toggle, Ctx};

pub(super) fn page(ctx: &mut Ctx<'_>, ui: &mut Ui) {
    card(
        ctx,
        ui,
        "图标底盘（macOS 风格圆角方块）",
        "",
        |ctx, ui| {
            combo(ctx, ui, Key::PLATE_MODE, "底盘类型", PLATE_OPTS);
            combo(ctx, ui, Key::PLATE_THEME, "底盘配色", PLATE_THEME_OPTS);
            slider(ctx, ui, Key::PLATE_OPACITY, "底盘不透明度", " %");
            slider(ctx, ui, Key::PlateRadius, "底盘圆角", " %");
        },
    );

    card(ctx, ui, "分割线", "", |ctx, ui| {
        slider(ctx, ui, Key::SEP_WIDTH, "分割线宽度", " px");
        slider(ctx, ui, Key::SEP_GAP, "分割线间距", " px");
        ui.label(
            RichText::new("圆角 0% = 方形，22% = 默认圆角方块，50% = 圆形。")
                .small()
                .weak(),
        );
    });

    card(ctx, ui, "运行指示与标签", "", |ctx, ui| {
        toggle(ctx, ui, Key::Dots, "显示运行中指示", "");
        combo(ctx, ui, Key::DotStyle, "指示样式", DOT_OPTS);
        toggle(ctx, ui, Key::Labels, "悬停显示名称标签", "");
    });
}
