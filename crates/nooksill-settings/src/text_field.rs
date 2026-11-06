//! 共享的自绘输入框。
//!
//! 为什么要自己画壳：egui 0.37 的 `TextEdit` 高度由「文字 + 内边距」
//! 决定，而圆角是固定值 —— 同一套 10px 圆角放在 22px 高的搜索框上是
//! 胶囊、放在 30px 的按钮上却是小圆角矩形，看起来像两套设计。而且
//! 焦点态描边走 `selection.stroke`、非焦点走 `bg_stroke`，状态切换还会
//! 再变一次。
//!
//! 所以外壳（圆角 / 底色 / 焦点光晕 / 图标 / 快捷键 / 清除按钮）由这里
//! 统一画，`TextEdit` 只负责编辑文字。所有窗口的输入框都调这一个函数，
//! 不再各自手写一遍 painter + interact。
//!
//! **垂直对齐用官方的 `align(Align2::LEFT_CENTER)`**（docs.rs 0.37：
//! `vertical_align` 已弃用，改用 `align`）。之前把 `TextEdit` 用
//! `add_sized` 强塞进 34px 高、又给 `Frame::NONE`（零内边距），文字就
//! 按内容高度顶格排，肉眼可见地偏上。

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Margin, Rect, Sense, Stroke,
    StrokeKind, Ui,
};

/// 输入框高度与圆角：所有输入框共用，控件之间曲率才一致。
pub const FIELD_H: f32 = 34.0;
pub const FIELD_R: f32 = 12.0;

/// 左侧图标。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum FieldIcon {
    #[default]
    None,
    Search,
}

/// 输入框配置。默认值 = 无图标、无右侧提示、无清除按钮。
#[derive(Copy, Clone, Debug, Default)]
pub struct FieldOpts<'a> {
    /// 空内容时的占位文字。
    pub hint: &'a str,
    pub icon: FieldIcon,
    /// 右侧灰字提示（例如 `Ctrl K`）；有内容且可清除时让位给清除按钮。
    pub trailing: Option<&'a str>,
    /// 有内容时显示 × 清除按钮。
    pub clearable: bool,
}

/// 画一个自绘输入框，返回**文字是否发生变化**。
///
/// `rect` 是外壳矩形（高度建议用 [`FIELD_H`]）；`id` 必须是稳定的，
/// Ctrl+K 之类的快捷键要靠它跨帧请求焦点。
pub fn text_field(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    text: &mut String,
    opts: &FieldOpts<'_>,
) -> bool {
    // 先把需要的颜色复制出来，结束 visuals 借用，后面才能自由 interact。
    let (accent, faint, field, focus_bg) = {
        let v = ui.visuals();
        let dark = v.dark_mode;
        (
            v.selection.stroke.color,
            v.weak_text_color(),
            if dark {
                Color32::from_rgba_unmultiplied(255, 255, 255, 16)
            } else {
                Color32::from_rgba_unmultiplied(23, 26, 35, 14)
            },
            if dark {
                Color32::from_rgb(38, 39, 47)
            } else {
                Color32::WHITE
            },
        )
    };
    let focused = ui.ctx().memory(|m| m.has_focus(id));
    let has_text = !text.is_empty();
    let r = CornerRadius::same(FIELD_R as u8);

    // ---- 外壳 ----
    ui.painter()
        .rect_filled(rect, r, if focused { focus_bg } else { field });
    if focused {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            r,
            Stroke::new(2.0, accent.linear_multiply(0.55)),
            StrokeKind::Inside,
        );
    }

    // 整块外壳可点聚焦。必须**先于**右侧按钮注册：egui 的命中顺序按
    // 注册先后，后画的在上层；放后面会把清除按钮的点击整块吃掉。
    let shell_resp = ui.interact(rect, id.with("shell"), Sense::click());
    if shell_resp.clicked() {
        ui.memory_mut(|m| m.request_focus(id));
    }

    let cy = rect.center().y;
    let left_pad = match opts.icon {
        FieldIcon::None => 12.0,
        FieldIcon::Search => {
            let cx = rect.left() + 17.0;
            let glass = Stroke::new(1.6, faint);
            ui.painter()
                .circle_stroke(egui::pos2(cx, cy - 1.5), 5.0, glass);
            ui.painter().line_segment(
                [
                    egui::pos2(cx + 3.8, cy + 2.3),
                    egui::pos2(cx + 8.0, cy + 6.5),
                ],
                glass,
            );
            32.0
        }
    };

    // ---- 右侧：清除按钮优先，否则快捷键提示 ----
    let mut right_pad = 12.0;
    let mut cleared = false;
    if opts.clearable && has_text {
        let clear =
            Rect::from_center_size(egui::pos2(rect.right() - 20.0, cy), egui::vec2(22.0, 22.0));
        let clear_resp = ui.interact(clear, id.with("clear"), Sense::click());
        if clear_resp.hovered() {
            ui.painter().rect_filled(
                clear,
                CornerRadius::same(8),
                ui.visuals().selection.bg_fill.linear_multiply(0.75),
            );
        }
        let c = clear.center();
        let d = 3.5;
        let stroke = Stroke::new(1.6, faint);
        ui.painter().line_segment(
            [egui::pos2(c.x - d, c.y - d), egui::pos2(c.x + d, c.y + d)],
            stroke,
        );
        ui.painter().line_segment(
            [egui::pos2(c.x + d, c.y - d), egui::pos2(c.x - d, c.y + d)],
            stroke,
        );
        clear_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "清空输入"));
        if clear_resp.clicked() {
            text.clear();
            cleared = true;
            ui.memory_mut(|m| m.request_focus(id));
        }
        right_pad = 44.0;
    } else if let Some(shortcut) = opts.trailing {
        ui.painter().text(
            egui::pos2(rect.right() - 13.0, cy),
            Align2::RIGHT_CENTER,
            shortcut,
            FontId::proportional(10.5),
            faint,
        );
        right_pad = 58.0;
    }

    // ---- 文字区：官方 align 居中，不再靠硬算内边距 ----
    let text_rect = Rect::from_min_max(
        egui::pos2(rect.left() + left_pad, rect.top() + 2.0),
        egui::pos2(rect.right() - right_pad, rect.bottom() - 2.0),
    );
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(text_rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    let hint = hint_for(focused, text.is_empty(), opts.hint);
    let resp = inner.add_sized(
        text_rect.size(),
        egui::TextEdit::singleline(text)
            .hint_text(hint)
            .id(id)
            .frame(egui::Frame::NONE.inner_margin(Margin::ZERO))
            .align(Align2::LEFT_CENTER)
            .desired_width(text_rect.width()),
    );
    resp.changed() || cleared
}

/// 无框 TextEdit 内边距为 0，焦点时光标会压在 hint 第一个字上 ——
/// 聚焦且未输入时收起 hint。抽成纯函数是为了能单测（hint 不是独立的
/// AccessKit 节点，kittest 查不到绘制出来的提示文字）。
fn hint_for(focused: bool, empty: bool, hint: &str) -> &str {
    if focused && empty {
        ""
    } else {
        hint
    }
}

#[cfg(test)]
mod tests {
    use super::hint_for;

    #[test]
    fn 焦点且空值时收起提示() {
        assert_eq!(hint_for(true, true, "搜索设置…"), "");
        assert_eq!(hint_for(false, true, "搜索设置…"), "搜索设置…");
        assert_eq!(hint_for(true, false, "搜索设置…"), "搜索设置…");
    }
}
