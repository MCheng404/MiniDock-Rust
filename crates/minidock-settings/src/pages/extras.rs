//! 附件页：时钟、倒影、小组件、浏览器。

use egui::Ui;
use minidock_core::config::Key;

use super::options::BROWSER_OPTS;
use super::widgets::{card, combo, slider, toggle, Ctx};

pub(super) fn page(ctx: &mut Ctx<'_>, ui: &mut Ui) {
    card(ctx, ui, "Dock 附件", "", |ctx, ui| {
        toggle(
            ctx,
            ui,
            Key::Clock,
            "在 Dock 右侧显示时间与日期",
            "每 30 秒刷新一次，占用极低",
        );
        toggle(ctx, ui, Key::Reflect, "图标倒影", "仅底部停靠时有效");
        slider(ctx, ui, Key::REFL_SIZE, "倒影大小", " %");
        slider(ctx, ui, Key::REFL_GAP, "倒影与 Dock 的距离", " px");
        slider(ctx, ui, Key::REFL_OPACITY, "倒影不透明度", " %");
    });

    card(
        ctx,
        ui,
        "小组件",
        "Dock 右侧的条带。每个小组件都能单独开关。",
        |ctx, ui| {
            toggle(ctx, ui, Key::NOTIFY_W, "通知", "");
            toggle(ctx, ui, Key::CTRL_CENTER, "控制中心", "");
            toggle(ctx, ui, Key::CC_ENERGY, "控制中心 · 节能磁贴", "");
            toggle(ctx, ui, Key::CC_HOTSPOT, "控制中心 · 移动热点磁贴", "");
            toggle(ctx, ui, Key::RES_MON, "资源监视器", "");
            toggle(ctx, ui, Key::TOOL_W, "计时器 / 待办", "");
            toggle(ctx, ui, Key::MODE_W, "专注模式", "");
            toggle(
                ctx,
                ui,
                Key::PANEL_DISMISS,
                "点面板外面收起",
                "日历 / 控制中心 / 资源监视器 / 计时器都适用；关掉后改为再点一次卡片收起",
            );
            super::widgets::row(ui, ctx.state, "小组件位置", "", |ui, id| {
                let mut cur = ctx.cfg.get_i32(Key::W_SIDE);
                egui::ComboBox::from_id_salt(Key::W_SIDE.name())
                    .selected_text(if cur == 0 { "右侧" } else { "左侧" })
                    .width(168.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut cur, 0, "右侧");
                        ui.selectable_value(&mut cur, 1, "左侧");
                    })
                    .response
                    .labelled_by(id);
                if cur != ctx.cfg.get_i32(Key::W_SIDE) {
                    ctx.cfg.apply_i32(Key::W_SIDE, cur);
                    ctx.out.changed = true;
                }
            });
            super::widgets::row(ui, ctx.state, "小组件顺序", "", |ui, id| {
                let mut cur = ctx.cfg.get_i32(Key::W_ORDER);
                egui::ComboBox::from_id_salt(Key::W_ORDER.name())
                    .selected_text(if cur == 0 {
                        "时钟在前"
                    } else {
                        "通知在前"
                    })
                    .width(168.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut cur, 0, "时钟在前");
                        ui.selectable_value(&mut cur, 1, "通知在前");
                    })
                    .response
                    .labelled_by(id);
                if cur != ctx.cfg.get_i32(Key::W_ORDER) {
                    ctx.cfg.apply_i32(Key::W_ORDER, cur);
                    ctx.out.changed = true;
                }
            });
        },
    );

    card(ctx, ui, "小工具", "", |ctx, ui| {
        combo(ctx, ui, Key::BROWSER, "网页使用的浏览器", BROWSER_OPTS);
        toggle(
            ctx,
            ui,
            Key::Notes,
            "速记（点击即可记录文字）",
            "开启后 Dock 上出现「速记」图标",
        );
    });
}
