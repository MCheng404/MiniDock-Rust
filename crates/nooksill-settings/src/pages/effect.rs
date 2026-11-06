//! 效果页：栏体周围的环境烟雾。
//!
//! 一项设置对应 `[effect.smoke]` 的一个键。这一页总共一屏，所以不折叠 ——
//! 折叠起来反而要多点一次才看得到「浓度」。

use egui::Ui;
use nooksill_core::config::Key;

use super::options::{
    SMOKE_COLOR2_OPTS, SMOKE_COLOR_OPTS, SMOKE_DENSITY_OPTS, SMOKE_OPACITY_OPTS, SMOKE_REACH_OPTS,
    SMOKE_SPEED_OPTS, SMOKE_THICK_OPTS,
};
use super::widgets::{card, combo, toggle, Ctx};

pub(super) fn page(ctx: &mut Ctx<'_>, ui: &mut Ui) {
    card(
        ctx,
        ui,
        "环境烟雾",
        "移植自 Windhawk 的 Taskbar Smoke：真实流体模拟，烟会上升、卷曲、随鼠标让开。\
         只在底部停靠时启用 —— 原版就是照底部任务栏调的，顶端停靠时烟会往屏幕外冒。",
        |ctx, ui| {
            toggle(ctx, ui, Key::SMOKE, "启用环境烟雾", "");
            combo(ctx, ui, Key::SMOKE_COLOR, "主色", SMOKE_COLOR_OPTS);
            combo(ctx, ui, Key::SMOKE_COLOR2, "第二色", SMOKE_COLOR2_OPTS);
            combo(ctx, ui, Key::SMOKE_OPACITY, "不透明度", SMOKE_OPACITY_OPTS);
            combo(
                ctx,
                ui,
                Key::SMOKE_TONE,
                "色调",
                &[(0, "烟（暗）"), (1, "辉光（亮）")],
            );
            toggle(ctx, ui, Key::SMOKE_CLOUDS, "上升的小烟团", "");
        },
    );

    card(
        ctx,
        ui,
        "烟雾手感",
        "浓度与速度直接改流体参数；厚度同时决定光晕往栏体外延伸多远。",
        |ctx, ui| {
            combo(ctx, ui, Key::SMOKE_DENSITY, "浓度", SMOKE_DENSITY_OPTS);
            combo(ctx, ui, Key::SMOKE_SPEED, "速度", SMOKE_SPEED_OPTS);
            combo(ctx, ui, Key::SMOKE_THICK, "光晕厚度", SMOKE_THICK_OPTS);
            combo(
                ctx,
                ui,
                Key::SMOKE_MOUSE,
                "鼠标反应",
                &[(0, "关闭"), (1, "柔和"), (2, "明显")],
            );
            combo(ctx, ui, Key::SMOKE_REACH, "鼠标范围", SMOKE_REACH_OPTS);
        },
    );
}
