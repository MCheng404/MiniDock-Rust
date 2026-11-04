//! 界面主题色：跟随系统 DWM 强调色，或用用户指定的 COLORREF。
//!
//! 设置窗口的所有控件（导航选中、开关、滑杆、选区、提示点）与可选的
//! Dock 栏体共用这一份解析结果。放在 GPU 层是因为只有它能同时碰到
//! `minidock_platform`（读 DWM）与 egui 的 `Visuals`。

use egui::{Color32, CornerRadius, Ui};
use minidock_core::color;
use minidock_core::config::{Config, Key};

/// 解析出当前生效的主题色（RGB 0..255）。`None` = 用主题默认配色。
///
/// - `UiAccent >= 0`：用户自定义 COLORREF，原样尊重（不调明度）。
/// - `UiAccent = -1`：跟随系统。DWM 给的颜色明度差异极大，统一压到
///   最大分量 190 再交给 UI —— 既保留色相，又保证在浅色背景下看得清。
///   近乎无彩的颜色（用户选了白/灰）直接放弃，回落到 egui 默认蓝。
#[must_use]
pub fn resolve(cfg: &Config) -> Option<[u8; 3]> {
    let v = cfg.get_i32(Key::UI_ACCENT);
    if v >= 0 {
        let c = color::from_colorref(v as u32);
        return Some([color::red(c), color::green(c), color::blue(c)]);
    }
    let raw = minidock_platform::theme::accent_raw()?;
    let max = raw.iter().copied().max()? as f32;
    let min = raw.iter().copied().min()? as f32;
    if max < 1.0 || max - min < 8.0 {
        return None;
    }
    let k = 190.0 / max;
    Some([
        (raw[0] as f32 * k).min(255.0) as u8,
        (raw[1] as f32 * k).min(255.0) as u8,
        (raw[2] as f32 * k).min(255.0) as u8,
    ])
}

/// 把主题色灌进设置窗口的 egui `Visuals`，并统一控件圆角。
///
/// 用 `Ui::visuals_mut` 而不是 `Context::set_visuals`：后者会把样式标脏、
/// 每帧调用等于每帧重绘（空闲不提交是这个窗口的硬约定）。前者只改
/// 当前帧这个 Ui 的样式，子 Ui 会继承。
pub fn apply_visuals(ui: &mut Ui, cfg: &Config) {
    let Some(rgb) = resolve(cfg) else { return };
    let accent = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);

    let vis = ui.visuals_mut();
    // 选区 / 滑杆填充的底色：45% alpha 的主题色。滑杆轨道吃的是这个
    // 原值，太淡会看不见；导航块与行悬停等"大面积底"在控件里再各自
    // 淡化一层（见 widgets.rs 的 row_hover / nav_item / hint_button）。
    vis.selection.bg_fill = accent.linear_multiply(0.45);
    vis.selection.stroke.color = accent;
    vis.widgets.active.bg_fill = accent.linear_multiply(0.85);
    vis.widgets.active.weak_bg_fill = accent.linear_multiply(0.16);
    vis.widgets.hovered.weak_bg_fill = accent.linear_multiply(0.10);
    vis.hyperlink_color = accent;
    // 滑杆已填充段默认不画，打开全局开关后才会用 selection.bg_fill
    // 铺左半段 —— 主题色在滑杆上才看得见。
    vis.slider_trailing_fill = true;

    // 元素优化：控件圆角统一到 10（按钮/开关/下拉/输入框），浮层 12。
    // 之前沿用 egui 默认的 2~4px，和 18px 的卡片放一起像两套设计。
    let r = CornerRadius::same(10);
    vis.widgets.noninteractive.corner_radius = r;
    vis.widgets.inactive.corner_radius = r;
    vis.widgets.hovered.corner_radius = r;
    vis.widgets.active.corner_radius = r;
    vis.widgets.open.corner_radius = r;
    vis.menu_corner_radius = CornerRadius::same(12);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 自定义主题色按 COLORREF(0x00BBGGRR) 解析，自定义值原样尊重。
    #[test]
    fn 自定义主题色按_colorref_解析() {
        let mut cfg = Config::defaults(96);
        cfg.apply_i32(Key::UI_ACCENT, 0x0088_4422);
        assert_eq!(resolve(&cfg), Some([0x22, 0x44, 0x88]));

        // 越界值被 clamp 而不是被当成"跟随系统"
        cfg.apply_i32(Key::UI_ACCENT, 0x00FF_FFFF);
        assert_eq!(resolve(&cfg), Some([0xFF, 0xFF, 0xFF]));
    }
}
