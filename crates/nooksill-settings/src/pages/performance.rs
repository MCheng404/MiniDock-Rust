//! 性能页：显卡、实时占用、高耗能功能、系统与兼容、预设。

use egui::{RichText, Ui};
use nooksill_core::config::{BgMode, Config, Key};

use super::options::{CLOCK_SHOW_OPTS, DATE_FMT_OPTS, MSAA_OPTS};
use super::widgets::{button, card, combo, row, slider, toggle, toggle_derived, Ctx};
use super::Command;

pub(super) fn page(ctx: &mut Ctx<'_>, ui: &mut Ui) {
    card(ctx, ui, "图形", "", |ctx, ui| {
        adapter_combo(ctx, ui);
        ui.label(
            RichText::new("自动：由后端按功耗偏好挑选，通常落在核显上。改动在下次启动生效。")
                .small()
                .weak(),
        );
    });

    card(
        ctx,
        ui,
        "帧率与同步",
        "默认不限制帧率；需要省电时可设上限或对齐刷新率。",
        |ctx, ui| {
            slider(ctx, ui, Key::FRAME_CAP, "帧率上限（0 = 不限制）", " FPS");
            toggle(
                ctx,
                ui,
                Key::VSYNC,
                "垂直同步",
                "按 DWM 合成刷新率对齐出帧，覆盖帧率上限",
            );
            combo(ctx, ui, Key::MSAA, "抗锯齿（下次启动生效）", MSAA_OPTS);
            ui.label(
                RichText::new("降低抗锯齿档位可显著减少每窗口的常驻显存。")
                    .small()
                    .weak(),
            );
        },
    );

    card(ctx, ui, "当前占用", "", |ctx, ui| {
        ui.monospace(&ctx.state.stats);
    });

    card(
        ctx,
        ui,
        "高耗能功能",
        "每项都标了开销，可以单独关掉。",
        |ctx, ui| {
            // 磨砂是 BgMode 的派生状态，不是独立布尔键。
            toggle_derived(
                ctx,
                ui,
                "磨砂玻璃背景",
                "中等：桌面捕获 + GPU 模糊，每 2.5 秒一次",
                ctx.cfg.bg_mode == BgMode::Frost,
                |cfg, on| cfg.apply_i32(Key::BgMode, if on { 2 } else { 1 }),
            );

            // 一个开关同时决定两个键：指示点与"运行中应用"列表。
            toggle_derived(
                ctx,
                ui,
                "扫描运行中应用（指示点 / 运行中列表）",
                "中等：每 2.5 秒枚举窗口；关闭后几乎不做后台扫描",
                ctx.cfg.scan_enabled(),
                |cfg, on| {
                    cfg.apply_i32(Key::Dots, i32::from(on));
                    cfg.apply_i32(Key::ShowRunning, i32::from(on));
                },
            );

            toggle(
                ctx,
                ui,
                Key::Preview,
                "悬停窗口预览",
                "低：仅悬停时使用系统缩略图",
            );
            toggle(ctx, ui, Key::Clock, "时间与日期", "低：每 30 秒刷新一次");
            combo(ctx, ui, Key::CLOCK_SHOW, "时钟显示内容", CLOCK_SHOW_OPTS);
            combo(ctx, ui, Key::DATE_FMT, "日期格式", DATE_FMT_OPTS);
            toggle(ctx, ui, Key::CLOCK_WEEK, "日期显示周几", "");
            toggle(
                ctx,
                ui,
                Key::CLOCK_SEC,
                "时间显示秒数",
                "每秒刷新一次，占用略增",
            );
            toggle(ctx, ui, Key::Reflect, "图标倒影", "低：多画一层半透明图像");
            toggle(
                ctx,
                ui,
                Key::MinAnim,
                "最小化缩入动画",
                "低：仅在窗口最小化时运行",
            );
        },
    );

    card(ctx, ui, "系统与兼容", "", |ctx, ui| {
        toggle(
            ctx,
            ui,
            Key::GUARDIAN,
            "防止被清理工具关闭",
            "启动一个极小的守护进程，被结束后自动重开",
        );
        toggle(
            ctx,
            ui,
            Key::CLICK_TOGGLE,
            "点击运行中的应用 = 最小化 / 恢复",
            "窗口在最前时再次点击会缩回 Dock",
        );
        toggle(ctx, ui, Key::MN_AUTO, "右键菜单显示开机自启开关", "");
        toggle(ctx, ui, Key::MN_NOTES, "右键菜单显示速记开关", "");
        toggle(ctx, ui, Key::MN_ADD, "空白处右键显示新建项", "");
    });

    card(ctx, ui, "预设", "", |ctx, ui| {
        ui.horizontal(|ui| {
            button(ctx, ui, "一键低占用模式", Command::LowPower);
            button(ctx, ui, "恢复推荐设置", Command::Recommended);
        });
        ui.label(
            RichText::new("空闲时没有常驻动画定时器，只在交互时提交 GPU 帧。")
                .small()
                .weak(),
        );
    });
}

/// 显卡选择。与 `combo` 的区别：值是**字符串**，且不走 `Key` 表
/// （`[Graphics]` 段是 Rust 版独有的，见 ADR-012）。
fn adapter_combo(ctx: &mut Ctx<'_>, ui: &mut Ui) {
    let mut cur = ctx.cfg.adapter.clone();
    let adapters = ctx.state.adapters.clone();
    row(ui, ctx.state, "显卡", "", |ui, id| {
        let text = if cur.is_empty() {
            "自动（推荐）"
        } else {
            cur.as_str()
        };
        egui::ComboBox::from_id_salt("gpu_adapter")
            .selected_text(text)
            .width(220.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut cur, String::new(), "自动（推荐）");
                for name in &adapters {
                    ui.selectable_value(&mut cur, name.clone(), name);
                }
            })
            .response
            .labelled_by(id);
    });

    if cur != ctx.cfg.adapter {
        ctx.cfg.adapter = Config::sanitize_adapter(&cur);
        ctx.out.changed = true;
    }

    // 实际生效的那块卡与配置可能不同（名字对不上会回退），必须显示真值，
    // 否则用户会以为"选了就生效了"。
    if ctx.state.active_adapter.is_empty() {
        ui.small("当前实际使用：尚未初始化");
    } else {
        ui.small(format!("当前实际使用：{}", ctx.state.active_adapter));
    }
    if !ctx.state.adapter_note.is_empty() {
        ui.colored_label(ui.visuals().warn_fg_color, &ctx.state.adapter_note);
    }
    if ctx.state.adapters.is_empty() {
        ui.label(
            RichText::new("没有枚举到可用显卡，只能用自动。")
                .small()
                .weak(),
        );
    }
}
