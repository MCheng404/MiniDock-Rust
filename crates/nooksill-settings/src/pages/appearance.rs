//! 外观页：尺寸、背景、配色、边框。

use egui::Ui;
use nooksill_core::config::Key;

use super::options::{BG_OPTS, MENU_THEME_OPTS, THEME_OPTS};
use super::widgets::{accent_row, card, color_row, combo, slider, toggle, Ctx};

pub(super) fn page(ctx: &mut Ctx<'_>, ui: &mut Ui) {
    card(ctx, ui, "主题色", "", |ctx, ui| {
        accent_row(ctx, ui);
        toggle(
            ctx,
            ui,
            Key::UI_ACCENT_DOCK,
            "同步到 Dock 栏体",
            "把主题色用到栏体渐变与运行指示点",
        );
        toggle(
            ctx,
            ui,
            Key::UI_ACCENT_DOCK_ICONS,
            "Dock 图标应用主题色",
            "开始 / 起动台图标底色与首字占位跟随主题色；关闭回落默认蓝",
        );
        toggle(
            ctx,
            ui,
            Key::UI_ACCENT_CC_ICONS,
            "控制中心图标应用主题色",
            "磁贴线稿图标跟随主题色；关闭用中性前景色",
        );
    });

    card(ctx, ui, "尺寸", "", |ctx, ui| {
        slider(ctx, ui, Key::IconSize, "图标大小", " px");
        slider(ctx, ui, Key::ZoomSize, "放大后大小", " px");
        slider(ctx, ui, Key::Spacing, "图标间距", " px");
        slider(ctx, ui, Key::Padding, "栏内边距（栏高）", " px");
        slider(ctx, ui, Key::Radius, "栏圆角", " px");
    });

    card(
        ctx,
        ui,
        "背景",
        "栏体背景是渐变 + 半透明板。三处弹层（起动台 / 设置 / 右键菜单）的磨砂在各自开关里，那个是真抓桌面再模糊。",
        |ctx, ui| {
            combo(ctx, ui, Key::Theme, "Dock 主题", THEME_OPTS);
            combo(ctx, ui, Key::BgMode, "背景材质", BG_OPTS);
            slider(ctx, ui, Key::Opacity, "不透明度", " %");
            slider(ctx, ui, Key::Blur, "磨砂模糊强度", "");
            toggle(ctx, ui, Key::Border, "显示栏边框", "");
            toggle(
                ctx,
                ui,
                Key::AccentTone,
                "跟随系统强调色（纯色 / 渐变）",
                "根据 Windows 强调色生成 Material You 风格色调",
            );
        },
    );

    card(ctx, ui, "自定义颜色", "", |ctx, ui| {
        color_row(ctx, ui, Key::BgColor1, "颜色 1");
        color_row(ctx, ui, Key::BgColor2, "颜色 2（渐变终点）");
        color_row(ctx, ui, Key::BG_COLOR3, "颜色 3（渐变中间色，可选）");
        slider(ctx, ui, Key::GRAD_A1, "颜色 1 不透明度", " %");
        slider(ctx, ui, Key::GRAD_A2, "颜色 2 不透明度", " %");
        slider(ctx, ui, Key::GRAD_A3, "颜色 3 不透明度", " %");
        slider(ctx, ui, Key::GRAD_ANGLE, "渐变方向", "°");
    });

    card(ctx, ui, "边框与菜单", "", |ctx, ui| {
        slider(ctx, ui, Key::BORDER_WIDTH, "边框粗细", " px");
        combo(ctx, ui, Key::MENU_THEME, "右键菜单主题", MENU_THEME_OPTS);
    });
}
