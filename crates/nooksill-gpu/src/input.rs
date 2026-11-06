//! Win32 消息 → `egui::RawInput`。
//!
//! 这是"不用 eframe"的代价所在：winit 帮你做的输入翻译，这里要自己写。
//! M0 只覆盖 Dock/设置界面真正用得到的部分：鼠标移动、左右键、滚轮、
//! Esc/Enter/Tab 这几个键。IME 留到 M1（见 PORTING.md 的风险项）。

use std::time::Instant;

use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Vec2};

#[derive(Debug)]
pub struct Input {
    events: Vec<Event>,
    pointer: Pos2,
    modifiers: Modifiers,
    start: Instant,
    hovering: bool,
}

impl Default for Input {
    fn default() -> Self {
        Self {
            events: Vec::new(),
            pointer: Pos2::ZERO,
            modifiers: Modifiers::default(),
            start: Instant::now(),
            hovering: false,
        }
    }
}

impl Input {
    pub fn pointer_moved(&mut self, x: f32, y: f32) {
        self.hovering = true;
        self.pointer = Pos2::new(x, y);
        self.events.push(Event::PointerMoved(self.pointer));
    }

    /// 光标离开窗口。egui 需要 `PointerGone` 才会清掉悬停态，
    /// 否则按钮会一直停在 hover 高亮。
    pub fn pointer_gone(&mut self) {
        self.hovering = false;
        self.events.push(Event::PointerGone);
    }

    #[must_use]
    pub const fn hovering(&self) -> bool {
        self.hovering
    }

    pub fn pointer_button(&mut self, pressed: bool, button: PointerButton) {
        self.events.push(Event::PointerButton {
            pos: self.pointer,
            button,
            pressed,
            modifiers: self.modifiers,
        });
    }

    /// `delta_y` 单位是"行"（Win32 的 `WHEEL_DELTA` 120 = 1 行）。
    pub fn wheel(&mut self, delta_y: f32) {
        self.events.push(Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: Vec2::new(0.0, delta_y),
            // 鼠标滚轮没有触控板那样的 phase/source 语义。
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: self.modifiers,
        });
    }

    pub fn key(&mut self, key: egui::Key, pressed: bool) {
        self.events.push(Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: self.modifiers,
        });
    }

    pub fn set_modifiers(&mut self, modifiers: Modifiers) {
        self.modifiers = modifiers;
    }

    /// 取出本帧的 `RawInput`。事件队列在此清空。
    pub fn take(&mut self, size_px: (u32, u32), pixels_per_point: f32) -> RawInput {
        let logical = Vec2::new(
            size_px.0 as f32 / pixels_per_point,
            size_px.1 as f32 / pixels_per_point,
        );
        let mut raw = RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, logical)),
            time: Some(self.start.elapsed().as_secs_f64()),
            events: std::mem::take(&mut self.events),
            ..Default::default()
        };
        // egui 0.37 用 `ViewportInfo::native_pixels_per_point` 告知原生缩放。
        // **不设的话 egui 按 1.0 缩放**：界面只占表面的 1/ppp，
        // 文字被放大后发虚（M0 的设置界面就有这个问题）。
        raw.viewports.insert(
            egui::ViewportId::ROOT,
            egui::ViewportInfo {
                native_pixels_per_point: Some(pixels_per_point),
                ..Default::default()
            },
        );
        raw
    }
}

/// VK 码 → egui 键。只映射设置界面需要的那些，其余返回 `None`。
#[must_use]
pub fn map_vk(vk: u32) -> Option<egui::Key> {
    use egui::Key;
    Some(match vk {
        0x1B => Key::Escape,
        0x0D => Key::Enter,
        0x09 => Key::Tab,
        0x20 => Key::Space,
        0x25 => Key::ArrowLeft,
        0x26 => Key::ArrowUp,
        0x27 => Key::ArrowRight,
        0x28 => Key::ArrowDown,
        0x08 => Key::Backspace,
        0x2E => Key::Delete,
        0x24 => Key::Home,
        0x23 => Key::End,
        _ => return None,
    })
}
