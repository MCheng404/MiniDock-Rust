//! 交互页：悬停放大、文件夹展开、动画。

use egui::Ui;
use nooksill_core::config::Key;

use super::options::MIN_STYLE_OPTS;
use super::widgets::{card, combo, toggle, Ctx};

pub(super) fn page(ctx: &mut Ctx<'_>, ui: &mut Ui) {
    card(ctx, ui, "交互", "", |ctx, ui| {
        toggle(ctx, ui, Key::Zoom, "悬停放大图标", "");
        toggle(
            ctx,
            ui,
            Key::FolderFan,
            "点击文件夹时弧形堆叠展开",
            "像 macOS 一样列出最近的文件，点击即可打开",
        );
        toggle(
            ctx,
            ui,
            Key::SEP_SCALE,
            "上下拖动分割线缩放 Dock",
            "按住图标之间的竖线上下拖，按比例调整图标与放大尺寸",
        );
    });

    card(ctx, ui, "动画", "", |ctx, ui| {
        combo(ctx, ui, Key::MIN_STYLE, "最小化动画风格", MIN_STYLE_OPTS);
        toggle(ctx, ui, Key::Bounce, "启动应用时弹跳", "");
        toggle(
            ctx,
            ui,
            Key::MinAnim,
            "最小化缩入 Dock 动画",
            "窗口最小化时缩小飞入对应图标",
        );
        toggle(ctx, ui, Key::Preview, "悬停运行中应用显示窗口预览", "");
    });
}
