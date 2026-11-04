//! 常规页：启动行为、锁定、快捷键、数据目录。

use egui::Ui;
use minidock_core::config::Key;

use super::options::UI_THEME_OPTS;
use super::widgets::{button, card, combo, toggle, Ctx};
use super::Command;

pub(super) fn page(ctx: &mut Ctx<'_>, ui: &mut Ui) {
    card(ctx, ui, "启动与常规", "", |ctx, ui| {
        // 开机自启动没有对应的配置键：它写的是注册表，显示的也必须是
        // 注册表里的真值，而不是配置里的意图。所以不走 `toggle`（那要一个
        // `Key`），但仍要按同样的"标题在左、开关在右"排版。
        let before = ctx.state.auto_start;
        let mut auto = before;
        super::widgets::row(
            ui,
            ctx.state,
            "开机自启动",
            "显示的是注册表里的真实状态",
            |ui, id| {
                ui.checkbox(&mut auto, "").labelled_by(id);
            },
        );
        if auto != before {
            ctx.state.auto_start = auto;
            ctx.out.command = Some(Command::ToggleAutostart(auto));
        }

        toggle(
            ctx,
            ui,
            Key::Lock,
            "锁定 Dock 布局",
            "防止误拖动、误移除图标",
        );
        toggle(
            ctx,
            ui,
            Key::ShowRunning,
            "显示未固定的运行中应用",
            "在分割线之后显示正在运行的应用，可拖入固定",
        );
        toggle(
            ctx,
            ui,
            Key::Hotkey,
            "全局快捷键 Ctrl + Alt + D",
            "随时显示 / 隐藏 Dock",
        );
        toggle(
            ctx,
            ui,
            Key::WebAppMode,
            "网页默认以应用窗口打开",
            "使用 Edge / Chrome 的 --app 模式",
        );
    });

    card(ctx, ui, "设置界面", "", |ctx, ui| {
        combo(ctx, ui, Key::UiTheme, "设置窗口主题", UI_THEME_OPTS);
        // 界面缩放：字体与按钮、行高、卡片间距一起缩（egui 的 ppp）。
        // 改完由主循环就地重建窗口，不用重开。
        combo(
            ctx,
            ui,
            Key::UI_SCALE,
            "界面缩放",
            &[
                (0, "跟随系统"),
                (100, "100%"),
                (110, "110%"),
                (125, "125%"),
                (150, "150%"),
                (175, "175%"),
                (200, "200%"),
            ],
        );
    });

    card(ctx, ui, "数据", "", |ctx, ui| {
        ui.horizontal(|ui| {
            ui.label("配置目录");
            ui.monospace(&ctx.state.config_dir);
        });
        ui.add_space(4.0);
        button(ctx, ui, "打开配置文件夹", Command::OpenConfigDir);
    });
}
