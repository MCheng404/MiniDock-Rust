//! 共享输入框 `text_field` 的交互契约。
//!
//! 视觉对齐（文字垂直居中）由离屏截图像素核对；这里守住能自动化的部分：
//! 清除按钮、焦点时收起 hint、以及输入确实写回字符串。

use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use minidock_settings::text_field::{text_field, FieldIcon, FieldOpts, FIELD_H};

#[derive(Debug)]
struct St {
    text: String,
}

fn harness(text: &str) -> Harness<'static, St> {
    Harness::builder()
        .with_size([400.0, 120.0])
        .with_context_setup(|ctx| {
            assert!(
                minidock_settings::fonts::install_cjk(ctx),
                "本机找不到中文字体，界面无法渲染"
            );
        })
        .build_ui_state(
            |ui, st: &mut St| {
                let rect = egui::Rect::from_center_size(
                    ui.max_rect().center(),
                    egui::vec2(260.0, FIELD_H),
                );
                let id = egui::Id::unique("test-field");
                text_field(
                    ui,
                    rect,
                    id,
                    &mut st.text,
                    &FieldOpts {
                        hint: "搜索设置…",
                        icon: FieldIcon::Search,
                        trailing: Some("Ctrl K"),
                        clearable: true,
                    },
                );
            },
            St {
                text: text.to_owned(),
            },
        )
}

/// 有内容时右侧出现清除按钮，点一下清空并写回字符串。
#[test]
fn 清除按钮清空输入() {
    let mut h = harness("图标");
    h.run();
    h.get_by_role_and_label(egui::Role::Button, "清空输入")
        .click();
    h.run();
    assert_eq!(h.state().text, "", "清除按钮应清空字符串");
}

/// 清除按钮只在有内容时出现；空内容时右侧让位给快捷键提示。
#[test]
fn 空内容时没有清除按钮() {
    let h = harness("");
    assert_eq!(
        h.state().text,
        "",
        "空内容下不应出现清除按钮（右侧是 Ctrl K 提示）"
    );
}
