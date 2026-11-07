//! 编辑弹窗：改名、新建分组、改目标与启动参数。
//!
//! # 为什么是独立窗口
//!
//! Dock 窗口只有栏体那么大，画出去的部分会被分层窗口裁掉；菜单能弹到
//! 栏外是因为它有自己的窗口。这个弹窗同理。
//!
//! 它是**模态**的：自己跑消息循环，阻塞到用户确定或取消（与右键菜单
//! 同一套做法，见 `bin/dock.rs` 的 `show_menu`）。
//!
//! # 输入
//!
//! 文字输入走 `WM_CHAR` + `WM_IME_COMPOSITION`：
//!
//! - `WM_CHAR` → `Input::text` → egui 的 `Event::Text`（egui 的输入框
//!   只认这个事件，`Event::Key` 不会插入字符）；
//! - `WM_IME_COMPOSITION` 的上屏串同样走 `Input::text`，候选窗由系统画。
//!
//! 退格 / Esc / 回车走 `WM_KEYDOWN` → `Input::key`。

use egui::{Align2, Color32, CornerRadius, FontId, Id, Rect as EguiRect, Sense, Stroke};
use nooksill_settings::text_field::{text_field, FieldOpts, FIELD_H};
use windows::core::w;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_CONTROL, VK_MENU, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetForegroundWindow, GetWindowLongPtrW,
    MsgWaitForMultipleObjectsEx, PeekMessageW, RegisterClassW, SetForegroundWindow,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage, GWLP_USERDATA, HWND_TOPMOST,
    MSG, MWMO_INPUTAVAILABLE, PM_REMOVE, QS_ALLINPUT, SWP_NOZORDER, SW_HIDE, SW_SHOW, WM_CHAR,
    WM_IME_COMPOSITION, WM_IME_ENDCOMPOSITION, WM_IME_STARTCOMPOSITION, WM_KEYDOWN, WM_KEYUP,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_SETCURSOR, WNDCLASSW, WS_EX_APPWINDOW,
    WS_EX_LAYERED, WS_EX_TOPMOST, WS_POPUP,
};

use crate::frame::FrameRenderer;
use crate::gpu::Gpu;
use crate::ime::Ime;
use crate::input::Input;

/// 窗口类名。
const CLASS_NAME: &str = "NookSillEditWindow";

/// 弹窗宽度（逻辑像素）。
const W: f32 = 380.0;
/// 四周留白。
const PAD: f32 = 20.0;
/// 标题高度。
const TITLE_H: f32 = 30.0;
/// 字段标签高度。
const LABEL_H: f32 = 18.0;
/// 字段之间的留白。
const FIELD_GAP: f32 = 14.0;
/// 按钮高度。
const BTN_H: f32 = 30.0;
/// 多行字段的高度。
const MULTI_H: f32 = 170.0;

/// 一个待编辑的字段。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditField {
    /// 字段名（画在输入框上方）。
    pub label: String,
    /// 字段的当前值。
    pub value: String,
    /// 空值时的占位提示。
    pub hint: String,
    /// 多行编辑。此时回车插入换行而不是提交，窗口也更长。
    pub multiline: bool,
}

/// 一次编辑请求。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditRequest {
    /// 弹窗标题。
    pub title: String,
    /// 字段（按顺序画）。
    pub fields: Vec<EditField>,
    /// 确定按钮上的字。
    pub ok_label: String,
}

/// 编辑结果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditOutcome {
    /// 用户点了确定（`false` = 取消）。
    pub accepted: bool,
    /// 各字段的新值，与请求里的字段一一对应。
    pub values: Vec<String>,
}

impl EditOutcome {
    /// 取第 `i` 个字段的值。越界返回空串。
    #[must_use]
    pub fn value(&self, i: usize) -> &str {
        self.values.get(i).map_or("", String::as_str)
    }
}

/// 弹窗窗口的复用槽：窗口句柄 + 专属渲染器。
///
/// 渲染器要独立：Dock 的 `FrameRenderer` 尺寸固定为栏体大小，而弹窗
/// 尺寸完全不同。`resize` 只重建离屏目标与 DIB，egui 上下文与 wgpu
/// 管线（最贵的两步）都保留。
pub type EditSlot = Option<(HWND, FrameRenderer)>;

/// 当前按着的修饰键（`egui::Modifiers`）。Ctrl+A / Ctrl+V 之类要靠它。
fn current_modifiers() -> egui::Modifiers {
    // SAFETY: `GetKeyState` 只读全局状态。
    let down = |vk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| unsafe {
        GetKeyState(i32::from(vk.0)) < 0
    };
    let ctrl = down(VK_CONTROL);
    egui::Modifiers {
        alt: down(VK_MENU),
        ctrl,
        shift: down(VK_SHIFT),
        mac_cmd: false,
        command: ctrl,
    }
}

/// 单个字段占的高度。
fn field_height(field: &EditField) -> f32 {
    LABEL_H + if field.multiline { MULTI_H } else { FIELD_H } + FIELD_GAP
}

/// 弹窗的逻辑高度。
fn logical_height(fields: &[EditField]) -> f32 {
    PAD * 2.0 + TITLE_H + fields.iter().map(field_height).sum::<f32>() + BTN_H
}

/// 弹窗的逻辑尺寸。
#[must_use]
pub fn logical_size(fields: &[EditField]) -> (f32, f32) {
    (W, logical_height(fields))
}

/// 弹窗放在哪：贴着光标，且整体落在 `bounds`（屏幕工作区）内。
#[must_use]
pub fn place(cursor: (f32, f32), size: (f32, f32), bounds: (f32, f32, f32, f32)) -> (f32, f32) {
    let (cx, cy) = cursor;
    let (w, h) = size;
    let (left, top, right, bottom) = bounds;
    let mut x = cx;
    let mut y = cy;
    if x + w > right {
        x = right - w;
    }
    if x < left {
        x = left;
    }
    if y + h > bottom {
        y = cy - h;
    }
    if y < top {
        y = top;
    }
    (x, y)
}

/// 弹窗的内部状态。挂在 `GWLP_USERDATA` 上，窗口过程靠它回传。
struct EditState {
    req: EditRequest,
    values: Vec<String>,
    /// 模态循环的退出标志。
    done: bool,
    /// 用户点了确定。
    accepted: bool,
    /// DPI 缩放。物理尺寸按它算，命中也要用它换算回逻辑坐标。
    ppp: f32,
    dark: bool,
    input: Input,
    ime: Ime,
    /// 弹窗里有没有多行字段。决定回车是"提交"还是"换行"。
    multiline: bool,
    /// 已经把焦点给过第一个输入框了。
    focused: bool,
}

fn state_ptr(hwnd: HWND) -> *mut EditState {
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut EditState }
}

/// 客户区物理坐标 → 逻辑坐标。
fn client_pos(lparam: LPARAM, ppp: f32) -> (f32, f32) {
    let x = (lparam.0 & 0xFFFF) as i16 as f32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
    (x / ppp, y / ppp)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    const VK_RETURN: usize = 0x0D;
    const VK_ESCAPE: usize = 0x1B;

    let ptr = state_ptr(hwnd);
    if ptr.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    // SAFETY: 指针由 `show_edit` 挂在栈上的 `Box` 上，整个模态循环期间有效。
    let win = unsafe { &mut *ptr };
    match msg {
        WM_MOUSEMOVE => {
            let (x, y) = client_pos(lparam, win.ppp);
            win.input.pointer_moved(x, y);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = client_pos(lparam, win.ppp);
            win.input.pointer_moved(x, y);
            win.input.pointer_button(true, egui::PointerButton::Primary);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = client_pos(lparam, win.ppp);
            win.input.pointer_moved(x, y);
            win.input
                .pointer_button(false, egui::PointerButton::Primary);
            LRESULT(0)
        }
        WM_CHAR => {
            if let Some(c) = char::from_u32(wparam.0 as u32) {
                if c == '\r' && win.multiline {
                    // 多行正文里回车是换行。`WM_KEYDOWN` 那边已经不再
                    // 把回车当提交了，别让这两个分支互相打架。
                    win.input.text("\n");
                } else if !c.is_control() {
                    win.input.text(&c.to_string());
                }
            }
            LRESULT(0)
        }
        WM_IME_COMPOSITION | WM_IME_STARTCOMPOSITION | WM_IME_ENDCOMPOSITION => {
            let mut committed = String::new();
            let handled = win.ime.handle(hwnd, msg, wparam, lparam, &mut committed);
            if !committed.is_empty() {
                win.input.text(&committed);
            }
            if handled {
                LRESULT(0)
            } else {
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
        }
        WM_KEYDOWN | WM_KEYUP => {
            let down = msg == WM_KEYDOWN;
            // egui 的 `TextEdit` 只从 `Event::Key` 读退格、方向键、Ctrl+A
            // 这些控制键 —— `WM_CHAR` 只给可打印字符，而且我们本来就
            // 把控制字符过滤掉了。少了这一段，输入框能打字却删不掉。
            win.input.set_modifiers(current_modifiers());
            if down {
                match wparam.0 {
                    // 组合中的回车是"选词"，不是提交；多行正文里的回车是
                    // 换行（由 WM_CHAR 插入），所以只有单行弹窗用回车提交。
                    VK_RETURN if !win.multiline && !win.ime.is_composing() => {
                        win.accepted = true;
                        win.done = true;
                    }
                    VK_ESCAPE if !win.ime.is_composing() => {
                        win.accepted = false;
                        win.done = true;
                    }
                    _ => {}
                }
            }
            // Enter / Esc 已经把弹窗关了，不必再喂给 egui。
            if !win.done {
                if let Some(key) = crate::input::map_vk(wparam.0 as u32) {
                    win.input.key(key, down);
                }
            }
            LRESULT(0)
        }
        WM_SETCURSOR => {
            // 输入框里是 I 型光标，别处是箭头。类光标留空时系统会退回
            // 忙光标（ADR-045），所以这里必须自己回。
            crate::cursor::set(crate::cursor::arrow());
            LRESULT(1)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// 注册窗口类。重复调用是安全的（`RegisterClassW` 失败就忽略）。
fn register_class() -> Result<(), Box<dyn std::error::Error>> {
    let hinst = unsafe { GetModuleHandleW(None)? };
    let wc = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hInstance: HINSTANCE(hinst.0),
        lpszClassName: class_pcwstr(),
        hCursor: crate::cursor::class_cursor(),
        hIcon: crate::app_icon().unwrap_or_default(),
        ..Default::default()
    };
    unsafe {
        RegisterClassW(&raw const wc);
    }
    Ok(())
}

fn class_pcwstr() -> windows::core::PCWSTR {
    static BUF: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    let buf = BUF.get_or_init(|| {
        CLASS_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect()
    });
    windows::core::PCWSTR(buf.as_ptr())
}

/// 打开编辑弹窗，阻塞到用户确定或取消。
///
/// # Panics
///
/// 不会 panic：`slot` 未初始化时返回错误。
pub fn show_edit(
    gpu: &Gpu,
    slot: &mut EditSlot,
    req: EditRequest,
    cursor: (f32, f32),
    bounds: (f32, f32, f32, f32),
    dark: bool,
    ppp: f32,
) -> Result<EditOutcome, Box<dyn std::error::Error>> {
    let (lw, lh) = logical_size(&req.fields);
    let pw = (lw * ppp).ceil() as i32;
    let ph = (lh * ppp).ceil() as i32;
    let (lx, ly) = place(cursor, (lw, lh), bounds);
    let (px, py) = ((lx * ppp).round() as i32, (ly * ppp).round() as i32);

    // 懒建：第一次开弹窗时建窗口与渲染器，之后复用。
    if slot.is_none() {
        register_class()?;
        let hinst = unsafe { GetModuleHandleW(None)? };
        unsafe {
            let hwnd = CreateWindowExW(
                // **不要** WS_EX_NOACTIVATE：输入框需要键盘焦点。
                // WS_EX_APPWINDOW 让它能被激活（Dock 自己是 NOACTIVATE，
                // 从来不是前台窗口，不带这一位时 SetForegroundWindow
                // 更容易被前台锁挡掉）。
                WS_EX_LAYERED | WS_EX_APPWINDOW | WS_EX_TOPMOST,
                w!("NookSillEditWindow"),
                w!("NookSill 编辑"),
                WS_POPUP,
                px,
                py,
                pw,
                ph,
                None,
                None,
                Some(HINSTANCE(hinst.0)),
                None,
            )?;
            let renderer = FrameRenderer::new(gpu, pw as u32, ph as u32, ppp)?;
            *slot = Some((hwnd, renderer));
        }
    }

    let Some((hwnd, renderer)) = slot.as_mut() else {
        return Err("编辑弹窗未初始化".into());
    };
    renderer.resize(gpu, pw as u32, ph as u32)?;

    unsafe {
        let _ = SetWindowPos(*hwnd, Some(HWND_TOPMOST), px, py, pw, ph, SWP_NOZORDER);
    }

    let mut state = Box::new(EditState {
        values: req.fields.iter().map(|f| f.value.clone()).collect(),
        multiline: req.fields.iter().any(|f| f.multiline),
        req,
        done: false,
        accepted: false,
        ppp,
        dark,
        input: Input::default(),
        ime: Ime::new(),
        focused: false,
    });
    unsafe {
        SetWindowLongPtrW(*hwnd, GWLP_USERDATA, (&raw mut *state) as isize);
        let _ = ShowWindow(*hwnd, SW_SHOW);
        // 输入框要有键盘焦点，窗口必须在前台。
        if GetForegroundWindow() != *hwnd {
            let _ = SetForegroundWindow(*hwnd);
        }
        // `SetForegroundWindow` 不保证成功（前台锁）；`SetFocus` 是拿到
        // 键盘消息的最后一道。少了它，`WM_CHAR` 会发到别的窗口，
        // 表现为"弹窗能点、打不进字"。
        let _ = SetFocus(Some(*hwnd));
        SetCapture(*hwnd);
    }
    crate::ime::Ime::enable(*hwnd);

    let mut msg = MSG::default();
    let mut painted = false;
    loop {
        // 只取本窗口的消息：模态期间 Dock 的输入要压住。
        let mut got = false;
        while unsafe { PeekMessageW(&raw mut msg, Some(*hwnd), 0, 0, PM_REMOVE) }.as_bool() {
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            got = true;
        }
        if state.done {
            break;
        }
        // 只在**有输入**或首帧出帧。模态循环每 60 ms 醒一次，若无条件
        // 提交，弹窗开着不动也会常驻 16 fps 的 GPU 开销 —— 与产品
        // "空闲不提交"的承诺冲突。
        if got || !painted {
            painted = true;
            let raw = state.input.take(renderer.size(), ppp);
            let st = &mut *state;
            renderer.render(gpu, raw, *hwnd, (px, py), |ui| {
                paint(ui, st);
            })?;
        }
        if !got {
            unsafe {
                MsgWaitForMultipleObjectsEx(None, 100, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
            }
        }
    }

    let values = std::mem::take(&mut state.values);
    let accepted = state.accepted;
    unsafe {
        if GetCapture() == *hwnd {
            let _ = ReleaseCapture();
        }
        SetWindowLongPtrW(*hwnd, GWLP_USERDATA, 0);
        let _ = ShowWindow(*hwnd, SW_HIDE);
    }
    Ok(EditOutcome { accepted, values })
}

/// 画弹窗：标题 + 若干输入框 + 确定/取消。
fn paint(ui: &mut egui::Ui, st: &mut EditState) {
    let rect = ui.max_rect();
    let dark = st.dark;
    let bg = if dark {
        Color32::from_rgba_unmultiplied(30, 30, 34, 246)
    } else {
        Color32::from_rgba_unmultiplied(250, 250, 252, 248)
    };
    let border = if dark {
        Color32::from_rgba_unmultiplied(255, 255, 255, 34)
    } else {
        Color32::from_rgba_unmultiplied(0, 0, 0, 26)
    };
    let ink = if dark {
        Color32::from_rgba_unmultiplied(240, 240, 245, 255)
    } else {
        Color32::from_rgba_unmultiplied(28, 28, 32, 255)
    };
    let faint = if dark {
        Color32::from_rgba_unmultiplied(255, 255, 255, 120)
    } else {
        Color32::from_rgba_unmultiplied(0, 0, 0, 110)
    };

    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(12), bg);
    painter.rect_stroke(
        rect,
        CornerRadius::same(12),
        Stroke::new(1.0, border),
        egui::StrokeKind::Inside,
    );

    // 标题
    painter.text(
        egui::pos2(rect.left() + PAD, rect.top() + PAD),
        Align2::LEFT_TOP,
        &st.req.title,
        FontId::proportional(16.0),
        ink,
    );

    // 字段
    //
    // `painter` 是 `ui` 的不可变借用，而 `text_field` 要 `&mut Ui`。
    // 借用检查器按最后一个使用点算生命周期，所以这里必须**先画完标题**
    // 再进入字段循环（字段里的标签改用 `ui.painter()` 现场取）。
    let mut y = rect.top() + PAD + TITLE_H;
    for (i, field) in st.req.fields.iter().enumerate() {
        ui.painter().text(
            egui::pos2(rect.left() + PAD, y),
            Align2::LEFT_TOP,
            &field.label,
            FontId::proportional(12.0),
            faint,
        );
        let h = if field.multiline { MULTI_H } else { FIELD_H };
        let fr = EguiRect::from_min_size(
            egui::pos2(rect.left() + PAD, y + LABEL_H),
            egui::vec2(rect.width() - PAD * 2.0, h),
        );
        let id = Id::unique(("edit-field", i));
        // 第一个字段自动获得焦点：用户打开就能直接打字。
        if !st.focused {
            ui.memory_mut(|m| m.request_focus(id));
            st.focused = true;
        }
        let mut value = st.values[i].clone();
        if field.multiline {
            paint_multiline(ui, fr, id, &field.hint, &mut value, st.dark);
        } else {
            text_field(
                ui,
                fr,
                id,
                &mut value,
                &FieldOpts {
                    hint: &field.hint,
                    ..FieldOpts::default()
                },
            );
        }
        st.values[i] = value;
        y += field_height(field);
    }

    // 按钮
    let ok_w = 92.0;
    let cancel_w = 76.0;
    let by = rect.bottom() - PAD - BTN_H;
    let ok_rect = EguiRect::from_min_size(
        egui::pos2(rect.right() - PAD - ok_w, by),
        egui::vec2(ok_w, BTN_H),
    );
    let cancel_rect = EguiRect::from_min_size(
        egui::pos2(ok_rect.left() - 10.0 - cancel_w, by),
        egui::vec2(cancel_w, BTN_H),
    );

    if button(ui, cancel_rect, "取消", false, ink, faint) {
        st.accepted = false;
        st.done = true;
    }
    if button(ui, ok_rect, &st.req.ok_label, true, ink, faint) {
        st.accepted = true;
        st.done = true;
    }
}

/// 画一个多行输入框。
///
/// 不复用 `text_field`：那个控件固定是 `TextEdit::singleline`，外壳和
/// 高度都是按一行算的。这里只借它的配色约定（焦点态底色 + 圆角），
/// 内容是 `TextEdit::multiline`。
fn paint_multiline(
    ui: &mut egui::Ui,
    rect: EguiRect,
    id: Id,
    hint: &str,
    value: &mut String,
    dark: bool,
) {
    let focused = ui.ctx().memory(|m| m.has_focus(id));
    let (field, focus_bg, border) = if dark {
        (
            Color32::from_rgba_unmultiplied(255, 255, 255, 16),
            Color32::from_rgb(38, 39, 47),
            Color32::from_rgba_unmultiplied(255, 255, 255, 34),
        )
    } else {
        (
            Color32::from_rgba_unmultiplied(23, 26, 35, 14),
            Color32::WHITE,
            Color32::from_rgba_unmultiplied(0, 0, 0, 26),
        )
    };
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(nooksill_settings::text_field::FIELD_R as u8),
        if focused { focus_bg } else { field },
    );
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(nooksill_settings::text_field::FIELD_R as u8),
        Stroke::new(1.0, border),
        egui::StrokeKind::Inside,
    );
    let inner = rect.shrink(10.0);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.add_sized(
        inner.size(),
        egui::TextEdit::multiline(value)
            .id(id)
            .hint_text(hint)
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::ZERO))
            .desired_width(inner.width()),
    );
}

/// 一个按钮。返回是否被点了。
fn button(
    ui: &mut egui::Ui,
    rect: EguiRect,
    label: &str,
    primary: bool,
    ink: Color32,
    faint: Color32,
) -> bool {
    let id = Id::unique(("edit-btn", label));
    let resp = ui.interact(rect, id, Sense::click());
    let accent = ui.visuals().selection.bg_fill;
    let (bg, fg) = if primary {
        (accent, Color32::WHITE)
    } else {
        (
            if ui.visuals().dark_mode {
                Color32::from_rgba_unmultiplied(255, 255, 255, 18)
            } else {
                Color32::from_rgba_unmultiplied(23, 26, 35, 14)
            },
            ink,
        )
    };
    ui.painter().rect_filled(rect, CornerRadius::same(9), bg);
    if resp.hovered() {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(9),
            if primary {
                accent.linear_multiply(1.12)
            } else {
                faint.linear_multiply(0.18)
            },
        );
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(13.0),
        fg,
    );
    resp.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn height_grows_with_the_field_count() {
        let one = logical_height(&[EditField::default()]);
        let two = logical_height(&[EditField::default(), EditField::default()]);
        assert!(two > one);
        assert!((two - one - (LABEL_H + FIELD_H + FIELD_GAP)).abs() < 0.01);
    }

    /// 多行字段要真的撑高窗口，否则正文会被裁掉。
    #[test]
    fn a_multiline_field_makes_the_window_taller() {
        let single = logical_height(&[EditField::default()]);
        let multi = logical_height(&[EditField {
            multiline: true,
            ..EditField::default()
        }]);
        assert!((multi - single - (MULTI_H - FIELD_H)).abs() < 0.01);
        assert!(
            logical_size(&[EditField {
                multiline: true,
                ..EditField::default()
            }])
            .1 > single
        );
    }

    #[test]
    fn place_keeps_the_window_inside_the_work_area() {
        let size = (380.0, 200.0);
        let bounds = (0.0, 0.0, 2560.0, 1600.0);
        let (x, y) = place((2500.0, 1500.0), size, bounds);
        assert!(x + size.0 <= bounds.2 + 0.01, "不该顶出右边");
        assert!(y >= bounds.1 - 0.01, "不该顶出上边");
        assert!(y + size.1 <= bounds.3 + 0.01, "不该顶出下边");
    }

    #[test]
    fn outcome_reads_fields_by_index() {
        let o = EditOutcome {
            accepted: true,
            values: vec!["a".into(), "b".into()],
        };
        assert_eq!(o.value(0), "a");
        assert_eq!(o.value(9), "", "越界返回空串");
    }
}
