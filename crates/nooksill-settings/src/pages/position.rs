//! 位置页：停靠边、显示器、显示模式。

use egui::Ui;
use nooksill_core::config::Key;

use super::options::{EDGE_OPTS, MODE_OPTS};
use super::widgets::{card, combo, slider, toggle, Ctx};

pub(super) fn page(ctx: &mut Ctx<'_>, ui: &mut Ui) {
    card(ctx, ui, "位置", "", |ctx, ui| {
        combo(ctx, ui, Key::Edge, "Dock 位置（居中）", EDGE_OPTS);

        // 显示器列表是运行期信息，选项动态生成。
        let before = ctx.cfg.monitor;
        let mut cur = before;
        let monitors = ctx.state.monitors.clone();
        super::widgets::row(ui, ctx.state, "显示在", "", |ui, id| {
            let text = monitors
                .get(cur.max(0) as usize)
                .cloned()
                .unwrap_or_else(|| format!("显示器 {}", cur + 1));
            egui::ComboBox::from_id_salt("monitor")
                .selected_text(text)
                .width(168.0)
                .show_ui(ui, |ui| {
                    for (i, name) in monitors.iter().enumerate() {
                        ui.selectable_value(&mut cur, i as i32, name);
                    }
                })
                .response
                .labelled_by(id);
        });
        if cur != before {
            ctx.cfg.apply_i32(Key::Monitor, cur);
            ctx.out.changed = true;
        }

        slider(ctx, ui, Key::Margin, "距屏幕边缘", " px");
    });

    card(ctx, ui, "显示模式", "", |ctx, ui| {
        combo(ctx, ui, Key::Mode, "模式", MODE_OPTS);
        toggle(
            ctx,
            ui,
            Key::Reserve,
            "最大化窗口避让 Dock",
            "仅「常驻显示」模式有效，像 macOS 一样为 Dock 预留桌面空间",
        );
    });
}
