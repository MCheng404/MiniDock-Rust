//! 速记面板：列出所有速记，把「新建 / 编辑 / 删除」交给主循环。
//!
//! # 为什么面板里没有输入框
//!
//! 面板是 `WS_EX_NOACTIVATE` 风格的分层窗口，拿键盘焦点很麻烦。文本的
//! 录入与中文输入法统一走 [`crate::edit_win`] 的模态弹窗（那里已经接了
//! `WM_CHAR` + `WM_IME_*` + 多行 `TextEdit`）。面板只负责"看到列表、
//! 点中哪一条"，动作由 `App` 取走后执行。
//!
//! # 为什么分页而不是滚动
//!
//! `ScrollArea` 里的子 Ui 有滚动偏移，`ui.interact` 的矩形与 `RawInput`
//! 的窗口坐标要额外换算，容易出现"看到的是 A、点到的是 B"。速记最多
//! [`MAX_NOTES`] 条，固定每页 6 条 + 翻页按钮既够用又不会错位。

use std::pin::Pin;
use std::time::Instant;

use egui::{Align2, Color32, CornerRadius, FontId, Rect as EguiRect, Stroke};
use nooksill_core::note::{Note, NoteBook};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetWindowLongPtrW, RegisterClassW, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, GWLP_USERDATA, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE,
    SW_SHOW, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR, WNDCLASSW, WS_EX_APPWINDOW, WS_EX_LAYERED,
    WS_EX_TOPMOST, WS_POPUP,
};

use crate::frame::FrameRenderer;
use crate::gpu::{Gpu, GpuError};

const CLASS_NAME: &str = "NookSillNotePanel";

/// 每页显示几条。
pub const PAGE_SIZE: usize = 6;
/// 面板逻辑尺寸。
const W: f32 = 420.0;
const H: f32 = 456.0;
const PAD: f32 = 16.0;
const HEADER_H: f32 = 40.0;
const ROW_H: f32 = 56.0;
const FOOTER_H: f32 = 34.0;
const BTN_W: f32 = 56.0;
const BTN_H: f32 = 24.0;

/// 面板逻辑尺寸（离屏验收用）。
pub const SIZE: (f32, f32) = (W, H);

/// 面板产出的动作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteAction {
    /// 新建一条（弹窗问标题与正文）。
    New,
    /// 编辑第 `k` 条。
    Edit(usize),
    /// 删除第 `k` 条。
    Remove(usize),
    /// 翻页（面板内部消化，不外发给主循环）。
    Page(i32),
}

/// 速记面板。
pub struct NoteWindow {
    hwnd: HWND,
    renderer: FrameRenderer,
    input: crate::input::Input,
    pos: (i32, i32),
    ppp: f32,
    dark: bool,
    visible: bool,
    notes: Vec<Note>,
    page: usize,
    hover: Option<usize>,
    /// 待主循环取走的动作。
    action: Option<NoteAction>,
    next_repaint: Option<Instant>,
}

impl std::fmt::Debug for NoteWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NoteWindow")
            .field("hwnd", &(self.hwnd.0 as isize))
            .field("visible", &self.visible)
            .field("notes", &self.notes.len())
            .finish_non_exhaustive()
    }
}

impl NoteWindow {
    /// 建窗（默认隐藏）。
    pub fn new(gpu: &Gpu, ppp: f32, dark: bool) -> Result<Pin<Box<Self>>, GpuError> {
        let hinst =
            unsafe { GetModuleHandleW(None) }.map_err(|_| GpuError::Win32("GetModuleHandleW"))?;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: windows::Win32::Foundation::HINSTANCE(hinst.0),
            lpszClassName: class_pcwstr(),
            hCursor: crate::cursor::class_cursor(),
            hIcon: crate::app_icon().unwrap_or_default(),
            ..Default::default()
        };
        unsafe {
            RegisterClassW(&raw const wc);
        }
        let pw = (W * ppp).round() as i32;
        let ph = (H * ppp).round() as i32;
        let (x, y) = centered_origin(pw, ph);
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_APPWINDOW | WS_EX_TOPMOST,
                class_pcwstr(),
                windows::core::w!("NookSill 速记"),
                WS_POPUP,
                x,
                y,
                pw,
                ph,
                None,
                None,
                Some(windows::Win32::Foundation::HINSTANCE(hinst.0)),
                None,
            )
        }
        .map_err(|_| GpuError::Win32("CreateWindowExW（速记面板）"))?;

        let renderer = FrameRenderer::new(gpu, pw as u32, ph as u32, ppp)?;
        let mut win = Box::pin(Self {
            hwnd,
            renderer,
            input: crate::input::Input::default(),
            pos: (x, y),
            ppp,
            dark,
            visible: false,
            notes: Vec::new(),
            page: 0,
            hover: None,
            action: None,
            next_repaint: None,
        });
        win.attach();
        Ok(win)
    }

    fn attach(&mut self) {
        unsafe {
            SetWindowLongPtrW(
                self.hwnd,
                GWLP_USERDATA,
                std::ptr::from_mut::<Self>(self) as isize,
            );
        }
    }

    #[must_use]
    pub const fn hwnd(&self) -> HWND {
        self.hwnd
    }

    #[must_use]
    pub const fn visible(&self) -> bool {
        self.visible
    }

    /// 更新内容并显示。`anchor` 是速记图标的屏幕矩形（物理像素）。
    pub fn show(&mut self, book: &NoteBook, anchor: (i32, i32, i32, i32)) {
        self.notes = book.notes.clone();
        self.clamp_page();
        self.hover = None;
        self.action = None;
        let pw = (W * self.ppp).round() as i32;
        let ph = (H * self.ppp).round() as i32;
        let (al, at, ar, _) = anchor;
        let cx = (al + ar) / 2;
        let work = nooksill_platform::display::work_area(0);
        let mut x = cx - pw / 2;
        if x < work.left {
            x = work.left;
        }
        if x + pw > work.right {
            x = work.right - pw;
        }
        let mut y = at - ph - 8;
        if y < work.top {
            y = work.top;
        }
        self.pos = (x, y);
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                x,
                y,
                pw,
                ph,
                SWP_NOACTIVATE | SWP_NOZORDER,
            );
            let _ = ShowWindow(self.hwnd, SW_SHOW);
        }
        self.visible = true;
        self.next_repaint = Some(Instant::now());
    }

    /// 数据变了之后重新喂一份，保留当前页。
    pub fn set_notes(&mut self, book: &NoteBook) {
        self.notes = book.notes.clone();
        self.clamp_page();
        self.next_repaint = Some(Instant::now());
    }

    fn clamp_page(&mut self) {
        let pages = self.pages();
        if self.page >= pages {
            self.page = pages.saturating_sub(1);
        }
    }

    /// 总页数（至少 1 页，空面板也要能点「新建」）。
    #[must_use]
    pub fn pages(&self) -> usize {
        page_count(self.notes.len())
    }

    fn turn_page(&mut self, delta: i32) {
        let pages = self.pages() as i32;
        let next = (self.page as i32 + delta).clamp(0, pages - 1);
        self.page = next.max(0) as usize;
        self.hover = None;
    }

    pub fn hide(&mut self) {
        if !self.visible {
            return;
        }
        self.visible = false;
        self.hover = None;
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    /// 取走待执行的动作。
    pub fn take_action(&mut self) -> Option<NoteAction> {
        self.action.take()
    }

    #[must_use]
    pub fn due(&self) -> bool {
        self.visible && self.next_repaint.is_some_and(|t| t <= Instant::now())
    }

    /// 画一帧。
    pub fn render(&mut self, gpu: &Gpu) -> Result<(), GpuError> {
        self.next_repaint = None;
        let pos = self.pos;
        let hwnd = self.hwnd;
        let dark = self.dark;
        let notes = self.notes.clone();
        let page = self.page;
        let hover = self.hover;
        let pages = self.pages();
        let raw = self.input.take(self.renderer.size(), self.ppp);
        let mut action = None;
        self.renderer.render(gpu, raw, hwnd, pos, |ui| {
            action = paint(ui, dark, &notes, page, pages, hover);
        })?;
        match action {
            // 翻页只改面板自己的状态，不占用主循环的动作通道。
            Some(NoteAction::Page(delta)) => {
                self.turn_page(delta);
                self.next_repaint = Some(Instant::now());
            }
            Some(a) => {
                self.action = Some(a);
                self.next_repaint = Some(Instant::now());
            }
            None => {}
        }
        Ok(())
    }

    fn on_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_SETCURSOR => {
                crate::cursor::set(crate::cursor::arrow());
                Some(LRESULT(1))
            }
            WM_MOUSEMOVE => {
                let (x, y) = client_pos(lparam, self.ppp);
                self.input.pointer_moved(x, y);
                let h = row_at(x, y, self.notes.len(), self.page);
                if h != self.hover {
                    self.hover = h;
                    self.next_repaint = Some(Instant::now());
                }
                None
            }
            WM_LBUTTONDOWN => {
                let (x, y) = client_pos(lparam, self.ppp);
                self.input.pointer_moved(x, y);
                self.input
                    .pointer_button(true, egui::PointerButton::Primary);
                self.next_repaint = Some(Instant::now());
                None
            }
            WM_LBUTTONUP => {
                let (x, y) = client_pos(lparam, self.ppp);
                self.input.pointer_moved(x, y);
                self.input
                    .pointer_button(false, egui::PointerButton::Primary);
                self.next_repaint = Some(Instant::now());
                None
            }
            WM_RBUTTONDOWN | WM_RBUTTONUP => None,
            WM_KEYDOWN => {
                if wparam.0 == 0x1B {
                    self.hide();
                }
                Some(LRESULT(0))
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                unsafe {
                    let _ = BeginPaint(self.hwnd, &mut ps);
                    let _ = EndPaint(self.hwnd, &ps);
                }
                Some(LRESULT(0))
            }
            WM_ERASEBKGND => Some(LRESULT(1)),
            _ => None,
        }
    }
}

/// 第 `k` 行在窗口里的矩形（逻辑坐标）。
fn row_rect(k: usize) -> EguiRect {
    EguiRect::from_min_size(
        egui::pos2(PAD, PAD + HEADER_H + k as f32 * ROW_H),
        egui::vec2(W - PAD * 2.0, ROW_H - 4.0),
    )
}

/// 命中哪一行（不含页脚按钮）。
fn row_at(x: f32, y: f32, total: usize, page: usize) -> Option<usize> {
    let start = page * PAGE_SIZE;
    let shown = total.saturating_sub(start).min(PAGE_SIZE);
    (0..shown).find(|&k| row_rect(k).contains(egui::pos2(x, y)))
}

/// 画面板。返回本帧的动作。
fn paint(
    ui: &mut egui::Ui,
    dark: bool,
    notes: &[Note],
    page: usize,
    pages: usize,
    hover: Option<usize>,
) -> Option<NoteAction> {
    let rect = ui.max_rect();
    let (bg, ink, faint, border, field) = palette(dark);
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(16), bg);
    painter.rect_stroke(
        rect,
        CornerRadius::same(16),
        Stroke::new(1.0, border),
        egui::StrokeKind::Inside,
    );
    painter.text(
        egui::pos2(rect.left() + PAD, rect.top() + PAD + 2.0),
        Align2::LEFT_TOP,
        "速记",
        FontId::proportional(15.0),
        ink,
    );

    let mut action = None;
    // 「新建」按钮（标题栏右侧）。
    let new_rect = EguiRect::from_min_size(
        egui::pos2(rect.right() - PAD - BTN_W, rect.top() + PAD),
        egui::vec2(BTN_W, BTN_H),
    );
    let new_resp = ui.interact(new_rect, egui::Id::unique("note-new"), egui::Sense::click());
    painter.rect_filled(new_rect, CornerRadius::same(7), field);
    painter.text(
        new_rect.center(),
        Align2::CENTER_CENTER,
        "＋ 新建",
        FontId::proportional(12.0),
        ink,
    );
    if new_resp.clicked() {
        action = Some(NoteAction::New);
    }

    // 列表。
    let start = page * PAGE_SIZE;
    let shown = notes.len().saturating_sub(start).min(PAGE_SIZE);
    if shown == 0 {
        painter.text(
            egui::pos2(rect.center().x, rect.top() + HEADER_H + 80.0),
            Align2::CENTER_CENTER,
            "还没有速记，点右上角「＋ 新建」",
            FontId::proportional(13.0),
            faint,
        );
    }
    for k in 0..shown {
        let note = &notes[start + k];
        let r = row_rect(k);
        if hover == Some(k) {
            painter.rect_filled(r, CornerRadius::same(9), field);
        }
        let row_resp = ui.interact(r, egui::Id::unique(("note-row", k)), egui::Sense::click());
        painter.text(
            egui::pos2(r.left() + 10.0, r.top() + 8.0),
            Align2::LEFT_TOP,
            fit_text(painter, &note.display_title(), r.width() - 140.0, 13.0),
            FontId::proportional(13.0),
            ink,
        );
        painter.text(
            egui::pos2(r.left() + 10.0, r.top() + 30.0),
            Align2::LEFT_TOP,
            fit_text(painter, &note.preview(), r.width() - 140.0, 11.0),
            FontId::proportional(11.0),
            faint,
        );

        // 行内按钮。注册在整行之后，egui 的命中顺序让按钮优先。
        let del_rect = EguiRect::from_min_size(
            egui::pos2(r.right() - BTN_W - 6.0, r.center().y - BTN_H / 2.0),
            egui::vec2(BTN_W, BTN_H),
        );
        let edit_rect = EguiRect::from_min_size(
            egui::pos2(del_rect.left() - BTN_W - 6.0, del_rect.top()),
            egui::vec2(BTN_W, BTN_H),
        );
        for (br, label, act) in [
            (edit_rect, "编辑", NoteAction::Edit(start + k)),
            (del_rect, "删除", NoteAction::Remove(start + k)),
        ] {
            let resp = ui.interact(
                br,
                egui::Id::unique(("note-btn", k, label)),
                egui::Sense::click(),
            );
            painter.rect_filled(br, CornerRadius::same(6), field);
            painter.text(
                br.center(),
                Align2::CENTER_CENTER,
                label,
                FontId::proportional(11.5),
                if label == "删除" {
                    Color32::from_rgb(230, 110, 110)
                } else {
                    ink
                },
            );
            if resp.clicked() {
                action = Some(act);
            }
        }
        // 按钮没被点时，点整行 = 编辑。
        if row_resp.clicked() && action.is_none() {
            action = Some(NoteAction::Edit(start + k));
        }
    }

    // 页脚：翻页 + 条数。
    let fy = rect.bottom() - PAD - FOOTER_H;
    painter.text(
        egui::pos2(rect.left() + PAD, fy + FOOTER_H / 2.0),
        Align2::LEFT_CENTER,
        format!("{} 条 · 第 {} / {} 页", notes.len(), page + 1, pages),
        FontId::proportional(11.0),
        faint,
    );
    let prev_rect = EguiRect::from_min_size(
        egui::pos2(rect.right() - PAD - BTN_W * 2.0 - 8.0, fy + 5.0),
        egui::vec2(BTN_W, BTN_H),
    );
    let next_rect = EguiRect::from_min_size(
        egui::pos2(rect.right() - PAD - BTN_W, fy + 5.0),
        egui::vec2(BTN_W, BTN_H),
    );
    let prev = ui.interact(
        prev_rect,
        egui::Id::unique("note-prev"),
        egui::Sense::click(),
    );
    let next = ui.interact(
        next_rect,
        egui::Id::unique("note-next"),
        egui::Sense::click(),
    );
    for (r, label, enabled) in [
        (prev_rect, "上一页", page > 0),
        (next_rect, "下一页", page + 1 < pages),
    ] {
        painter.rect_filled(r, CornerRadius::same(6), field);
        painter.text(
            r.center(),
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(11.0),
            if enabled { ink } else { faint },
        );
    }
    if prev.clicked() && page > 0 {
        return Some(NoteAction::Page(-1));
    }
    if next.clicked() && page + 1 < pages {
        return Some(NoteAction::Page(1));
    }
    action
}

/// 总页数。抽成纯函数才能在不开窗口的情况下测。
fn page_count(total: usize) -> usize {
    total.div_ceil(PAGE_SIZE).max(1)
}

/// 离屏验收入口（`examples/panels_shot.rs`）。
///
/// 与 `settings_win::shell` 同一个理由：截图必须走**真实窗口那一帧**的
/// 绘制代码，自己拼一个近似版验不出问题。这里不建窗口、不碰 Win32，
/// 只把面板内容画进调用方给的 `Ui`。
pub fn paint_preview(
    ui: &mut egui::Ui,
    dark: bool,
    book: &NoteBook,
    page: usize,
    hover: Option<usize>,
) {
    let _ = paint(
        ui,
        dark,
        &book.notes,
        page,
        page_count(book.notes.len()),
        hover,
    );
}

fn palette(dark: bool) -> (Color32, Color32, Color32, Color32, Color32) {
    if dark {
        (
            Color32::from_rgba_unmultiplied(30, 30, 34, 246),
            Color32::from_rgba_unmultiplied(240, 240, 245, 255),
            Color32::from_rgba_unmultiplied(255, 255, 255, 130),
            Color32::from_rgba_unmultiplied(255, 255, 255, 34),
            Color32::from_rgba_unmultiplied(255, 255, 255, 18),
        )
    } else {
        (
            Color32::from_rgba_unmultiplied(250, 250, 252, 248),
            Color32::from_rgba_unmultiplied(28, 28, 32, 255),
            Color32::from_rgba_unmultiplied(0, 0, 0, 120),
            Color32::from_rgba_unmultiplied(0, 0, 0, 26),
            Color32::from_rgba_unmultiplied(23, 26, 35, 14),
        )
    }
}

/// 超宽就截断加省略号。
fn fit_text(painter: &egui::Painter, text: &str, max_w: f32, size: f32) -> String {
    let font = FontId::proportional(size);
    if painter
        .layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE)
        .size()
        .x
        <= max_w
    {
        return text.to_owned();
    }
    let mut out: Vec<char> = text.chars().collect();
    while !out.is_empty() {
        out.pop();
        let s: String = out.iter().collect();
        if painter
            .layout_no_wrap(format!("{s}…"), font.clone(), Color32::WHITE)
            .size()
            .x
            <= max_w
        {
            return format!("{s}…");
        }
    }
    String::new()
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut NoteWindow;
    if ptr.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    // SAFETY: 指针由 `attach` 挂在 `Pin<Box<Self>>` 的最终地址上。
    let win = unsafe { &mut *ptr };
    if let Some(result) = win.on_message(msg, wparam, lparam) {
        return result;
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

impl Drop for NoteWindow {
    fn drop(&mut self) {
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
        }
    }
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

fn centered_origin(pw: i32, ph: i32) -> (i32, i32) {
    let work = nooksill_platform::display::work_area(0);
    (
        (work.left + work.right) / 2 - pw / 2,
        (work.top + work.bottom) / 2 - ph / 2,
    )
}

fn client_pos(lparam: LPARAM, ppp: f32) -> (f32, f32) {
    let x = (lparam.0 & 0xFFFF) as i16 as f32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
    (x / ppp, y / ppp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_laid_out_sequentially() {
        let a = row_rect(0);
        let b = row_rect(1);
        assert!(b.top() > a.top());
        assert_eq!(a.left(), b.left());
        assert_eq!(a.width(), b.width());
    }

    #[test]
    fn row_at_matches_the_visible_page() {
        let a = row_rect(0).center();
        assert_eq!(row_at(a.x, a.y, 3, 0), Some(0));
        let b = row_rect(1).center();
        assert_eq!(row_at(b.x, b.y, 3, 0), Some(1));
        // 第二页的第 0 行也是页内下标 0；全局下标由调用方加 `page * PAGE_SIZE`。
        assert_eq!(row_at(a.x, a.y, 10, 1), Some(0));
        assert_eq!(row_at(5.0, 5.0, 3, 0), None, "标题栏不算行");
    }

    #[test]
    fn page_count_has_a_floor_of_one() {
        assert_eq!(page_count(0), 1);
        assert_eq!(page_count(1), 1);
        assert_eq!(page_count(PAGE_SIZE), 1);
        assert_eq!(page_count(PAGE_SIZE + 1), 2);
    }

    #[test]
    fn max_notes_divides_into_pages() {
        assert_eq!(page_count(nooksill_core::note::MAX_NOTES), 6);
    }
}
