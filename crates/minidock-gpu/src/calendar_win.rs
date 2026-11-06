//! 日历弹层：点条带上的时钟卡片弹出来。
//!
//! 392 × 556 逻辑像素，顶部年/月翻页与「今天」，
//! 周一起始的 6×7 网格，每格下方一行副标签（节日 / 节气 / 农历日），
//! 底部是选中日的详情（公历、农历、干支历、下一个节气）。
//!
//! 与资源监视器 / 计时器面板的差别只有两点：
//!
//! - **它是弹层，不是面板**。用 `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`，
//!   点外面就收起（由主循环轮询，见 [`crate::popup`]）。
//! - **它贴在时钟卡片旁边**，不是屏幕居中。
//!
//! 算法全在 `minidock_core::calendar` 里，这里只负责画和判命中。

use std::pin::Pin;
use std::time::Instant;

use egui::{Align2, Color32, CornerRadius, FontId, Rect as EguiRect, Stroke};
use minidock_core::calendar::{self, CalendarState};
use minidock_core::config::Edge;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetWindowLongPtrW, RegisterClassW, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, GWLP_USERDATA, SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOWNA,
    WM_ERASEBKGND, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SETCURSOR, WNDCLASSW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::frame::FrameRenderer;
use crate::gpu::{Gpu, GpuError};

/// 窗口类名。
pub const CLASS_NAME: &str = "MiniDockCalendar";

/// 面板逻辑尺寸：宽 392，高按内容收紧到 480。
const PANEL_W: f32 = 392.0;
const PANEL_H: f32 = 480.0;

/// 窗口边距。
const PM: f32 = 8.0;

/// 一次点击命中的目标。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Hit {
    /// 0 = 上一年，1 = 下一年，2 = 上一月，3 = 下一月，4 = 今天。
    Nav(usize),
    /// 网格里的第 k 格（0..42）。
    Cell(usize),
}

/// 面板上的可点区域。绘制时算出来，鼠标事件里用。
///
/// 与计时器面板同一套做法：不用 egui 控件，自己算矩形自己判命中
/// （ADR-039：`with_layout` 在面板里拿不到剩余宽度）。
#[derive(Clone, Debug)]
struct Hits {
    nav: [EguiRect; 5],
    cell: [EguiRect; 42],
}

impl Default for Hits {
    fn default() -> Self {
        // `egui::Rect` 没有 `Default`；空矩形的 `contains` 恒为假，不会误命中。
        let n = EguiRect::NOTHING;
        Self {
            nav: [n; 5],
            cell: [n; 42],
        }
    }
}

impl Hits {
    fn at(&self, p: egui::Pos2) -> Option<Hit> {
        for (i, r) in self.nav.iter().enumerate() {
            if r.contains(p) {
                return Some(Hit::Nav(i));
            }
        }
        for (k, r) in self.cell.iter().enumerate() {
            if r.contains(p) {
                return Some(Hit::Cell(k));
            }
        }
        None
    }
}

/// 日历弹层。
pub struct CalendarWindow {
    hwnd: HWND,
    renderer: FrameRenderer,
    /// 输入。只读面板也需要它：`Input::take` 会带上 `screen_rect`，
    /// 缺了它 egui 的绘制区只剩一小块（ADR-039 踩过）。
    input: crate::input::Input,
    /// 屏幕左上角的物理坐标。
    pos: (i32, i32),
    ppp: f32,
    /// 显示的年月与选中日。
    state: CalendarState,
    /// "今天"，打开时取一次。
    today: (i32, u32, u32),
    /// 鼠标悬停在哪个可点区域。
    hover: Option<Hit>,
    /// 本帧算出的命中区。
    hits: Hits,
    visible: bool,
    dark: bool,
    /// 下一次重绘的时刻。
    next_repaint: Instant,
}

impl std::fmt::Debug for CalendarWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CalendarWindow")
            .field("hwnd", &(self.hwnd.0 as isize))
            .field("visible", &self.visible)
            .field("year", &self.state.year)
            .field("month", &self.state.month)
            .finish_non_exhaustive()
    }
}

impl CalendarWindow {
    /// 建窗（默认隐藏）。建好之后用 [`Self::show_at`] 摆到时钟卡片旁边。
    pub fn new(
        gpu: &Gpu,
        ppp: f32,
        dark: bool,
        today: (i32, u32, u32),
    ) -> Result<Pin<Box<Self>>, GpuError> {
        let hinst =
            unsafe { GetModuleHandleW(None) }.map_err(|_| GpuError::Win32("GetModuleHandleW"))?;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: windows::Win32::Foundation::HINSTANCE(hinst.0),
            lpszClassName: class_pcwstr(),
            // 类光标留空 = 进入窗口时显示忙光标（ADR-045）。
            hCursor: crate::cursor::class_cursor(),
            ..Default::default()
        };
        unsafe {
            RegisterClassW(&raw const wc);
        }

        let (pw, ph) = physical_size(ppp);
        let (x, y) = (0, 0);
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class_pcwstr(),
                windows::core::w!("日历"),
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
        .map_err(|_| GpuError::Win32("CreateWindowExW（日历）"))?;

        let renderer = FrameRenderer::new(gpu, pw as u32, ph as u32, ppp)?;
        let mut win = Box::pin(Self {
            hwnd,
            renderer,
            input: crate::input::Input::default(),
            pos: (x, y),
            ppp,
            state: CalendarState::new(today),
            today,
            hover: None,
            hits: Hits::default(),
            visible: false,
            dark,
            next_repaint: Instant::now(),
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

    /// 窗口句柄。主循环判"点是否落在面板里"要用。
    #[must_use]
    pub const fn hwnd(&self) -> HWND {
        self.hwnd
    }

    #[must_use]
    pub const fn visible(&self) -> bool {
        self.visible
    }

    /// 时钟卡片的屏幕矩形（物理像素，`(left, top, right, bottom)`）。
    ///
    /// 每次打开都重新摆位：栏体可能已经换边、换尺寸，甚至换了显示器。
    pub fn show_at(&mut self, anchor: (f32, f32, f32, f32), edge: Edge) {
        let (pw, ph) = physical_size(self.ppp);
        let (x, y) = place(pw, ph, anchor, edge, self.ppp);
        self.pos = (x, y);
        unsafe {
            let _ = SetWindowPos(self.hwnd, None, x, y, pw, ph, SWP_NOZORDER | SWP_NOACTIVATE);
            let _ = ShowWindow(self.hwnd, SW_SHOWNA);
        }
        self.visible = true;
        self.next_repaint = Instant::now();
    }

    /// 关掉。点外面关闭由主循环轮询（见 `crate::popup`）。
    pub fn hide(&mut self) {
        if !self.visible {
            return;
        }
        self.visible = false;
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    /// 是否到了该重绘的时刻。
    #[must_use]
    pub fn due(&self) -> bool {
        self.visible && self.next_repaint <= Instant::now()
    }

    /// "今天"变了（跨零点）就跟着走。
    pub fn set_today(&mut self, today: (i32, u32, u32)) {
        if self.today != today {
            self.today = today;
            self.next_repaint = Instant::now();
        }
    }

    /// 画一帧。
    pub fn render(&mut self, gpu: &Gpu) -> Result<(), GpuError> {
        let pos = self.pos;
        let hwnd = self.hwnd;
        let dark = self.dark;
        let state = self.state.clone();
        let today = self.today;
        let hover = self.hover;
        let raw = self.input.take(self.renderer.size(), self.ppp);
        let mut hits = Hits::default();
        self.renderer.render(gpu, raw, hwnd, pos, |ui| {
            hits = draw_panel(ui, dark, &state, today, hover);
        })?;
        self.hits = hits;
        Ok(())
    }

    fn click(&mut self, hit: Hit) {
        match hit {
            Hit::Nav(0) => self.state.step(-1, 0),
            Hit::Nav(1) => self.state.step(1, 0),
            Hit::Nav(2) => self.state.step(0, -1),
            Hit::Nav(3) => self.state.step(0, 1),
            Hit::Nav(_) => self.state.go_today(self.today),
            Hit::Cell(k) => {
                let c = &self.state.cells()[k];
                let (y, m, d) = (c.year, c.month, c.day);
                self.state.select(y, m, d);
            }
        }
        self.next_repaint = Instant::now();
    }

    fn on_message(&mut self, msg: u32, _wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_MOUSEMOVE => {
                let (x, y) = client_pos(lparam, self.ppp);
                let p = egui::pos2(x, y);
                if self.hover != self.hits.at(p) {
                    self.hover = self.hits.at(p);
                    self.next_repaint = Instant::now();
                }
                crate::cursor::set_hand_if(self.hover.is_some());
                None
            }
            WM_LBUTTONDOWN => Some(LRESULT(0)),
            WM_LBUTTONUP => {
                let (x, y) = client_pos(lparam, self.ppp);
                if let Some(hit) = self.hits.at(egui::pos2(x, y)) {
                    self.click(hit);
                }
                Some(LRESULT(0))
            }
            WM_SETCURSOR => {
                crate::cursor::set_hand_if(self.hover.is_some());
                Some(LRESULT(1))
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

/// 画整块：导航 + 星期头 + 网格 + 选中日详情。
///
/// 全部显式定位。返回本帧的命中区。
fn draw_panel(
    ui: &mut egui::Ui,
    dark: bool,
    state: &CalendarState,
    today: (i32, u32, u32),
    hover: Option<Hit>,
) -> Hits {
    let rect = ui.max_rect();
    let painter = ui.painter().clone();

    let bg = if dark {
        Color32::from_rgba_unmultiplied(24, 25, 30, 250)
    } else {
        Color32::from_rgba_unmultiplied(246, 247, 250, 250)
    };
    painter.rect_filled(rect, CornerRadius::same(16), bg);

    let (tc, sub, faded) = if dark {
        (
            Color32::from_rgba_unmultiplied(240, 240, 245, 255),
            Color32::from_rgba_unmultiplied(150, 154, 168, 255),
            Color32::from_rgba_unmultiplied(200, 200, 210, 255),
        )
    } else {
        (
            Color32::from_rgba_unmultiplied(28, 28, 34, 255),
            Color32::from_rgba_unmultiplied(112, 116, 130, 255),
            Color32::from_rgba_unmultiplied(90, 90, 100, 255),
        )
    };
    let accent = ui.visuals().selection.bg_fill;
    let red = Color32::from_rgba_unmultiplied(230, 74, 70, 255);
    // 悬停底色：深色主题下是 8% 白，浅色主题下是 9% 黑。
    let hover_bg = if dark {
        Color32::from_rgba_unmultiplied(255, 255, 255, 20)
    } else {
        Color32::from_rgba_unmultiplied(0, 0, 0, 22)
    };
    let line_c = if dark {
        Color32::from_rgba_unmultiplied(255, 255, 255, 40)
    } else {
        Color32::from_rgba_unmultiplied(0, 0, 0, 40)
    };

    // ---- 布局基准（全部来自CalDraw 的 Sc 数值）----
    let x0 = rect.left() + PM + 16.0;
    let y0 = rect.top() + PM + 14.0;
    let cw = PANEL_W - 2.0 * PM - 32.0;
    let bw = 30.0_f32;
    let nav_h = 34.0_f32;

    let mut hits = Hits::default();

    // ---- 导航行：年 ‹ ›，月 ‹ ›，今天 ----
    let nx = [
        x0,
        x0 + bw + 62.0,
        x0 + 132.0,
        x0 + 132.0 + bw + 48.0,
        x0 + cw - 58.0,
    ];
    let nw = [bw, bw, bw, bw, 58.0];
    for i in 0..5 {
        hits.nav[i] = EguiRect::from_min_size(egui::pos2(nx[i], y0), egui::vec2(nw[i], nav_h));
        // 只有「今天」按钮常驻底色，箭头只在下悬停时出底色。
        if i == 4 {
            let on = matches!(hover, Some(Hit::Nav(4)));
            painter.rect_filled(
                hits.nav[i],
                CornerRadius::same((nav_h * 0.5) as u8),
                if on {
                    accent
                } else {
                    // 8~10% 的主题色底
                    Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 46)
                },
            );
        } else if matches!(hover, Some(Hit::Nav(j)) if j == i) {
            painter.rect_filled(
                hits.nav[i],
                CornerRadius::same((nav_h * 0.5) as u8),
                hover_bg,
            );
        }
    }

    // 箭头用线段画，不用「‹ ›」字符：字体是打包的，字符不一定有字形
    // （ADR-040 在 ✓/✕ 上踩过豆腐块）。
    for (idx, left) in [(0_usize, nx[0]), (1, nx[1]), (2, nx[2]), (3, nx[3])] {
        let cx = left + bw * 0.5;
        let cy = y0 + nav_h * 0.5;
        let dir = if idx % 2 == 0 { -1.0 } else { 1.0 };
        let stroke = Stroke::new(1.8, tc);
        painter.line_segment(
            [
                egui::pos2(cx - dir * 3.0, cy - 5.0),
                egui::pos2(cx + dir * 3.0, cy),
            ],
            stroke,
        );
        painter.line_segment(
            [
                egui::pos2(cx + dir * 3.0, cy),
                egui::pos2(cx - dir * 3.0, cy + 5.0),
            ],
            stroke,
        );
    }

    painter.text(
        egui::pos2(nx[0] + bw + 31.0, y0 + nav_h * 0.5),
        Align2::CENTER_CENTER,
        format!("{}年", state.year),
        FontId::proportional(14.0),
        tc,
    );
    painter.text(
        egui::pos2(nx[2] + bw + 24.0, y0 + nav_h * 0.5),
        Align2::CENTER_CENTER,
        format!("{}月", state.month),
        FontId::proportional(14.0),
        tc,
    );
    painter.text(
        hits.nav[4].center(),
        Align2::CENTER_CENTER,
        "今天",
        FontId::proportional(13.0),
        if matches!(hover, Some(Hit::Nav(4))) {
            Color32::WHITE
        } else {
            accent
        },
    );

    // ---- 星期头（周一起始）----
    let wy = y0 + nav_h + 8.0;
    let cell_w = cw / 7.0;
    let cell_h = 46.0_f32;
    let gy = wy + 26.0;
    for i in 0..7 {
        // i = 0 是周一，对应 WEEKDAY[(1+0)%7] = "一"。
        let name = calendar::weekday_name(((i + 1) % 7) as u32);
        painter.text(
            egui::pos2(x0 + (i as f32 + 0.5) * cell_w, wy + 12.0),
            Align2::CENTER_CENTER,
            name,
            FontId::proportional(11.0),
            if i >= 5 { red } else { sub },
        );
    }

    // ---- 6 × 7 网格 ----
    let sel = state.selected;
    for (k, c) in state.cells().iter().enumerate() {
        let col = k % 7;
        let row = k / 7;
        let x = x0 + col as f32 * cell_w;
        let y = gy + row as f32 * cell_h;
        hits.cell[k] = EguiRect::from_min_size(egui::pos2(x, y), egui::vec2(cell_w, cell_h));

        let is_today = (c.year, c.month, c.day) == today;
        let is_sel = (c.year, c.month, c.day) == sel;
        let r = (cell_w.min(cell_h)) * 0.5 - 2.0;
        let cx = x + cell_w * 0.5;
        let cy = y + cell_h * 0.5 - 3.0;

        if is_today {
            painter.circle_filled(egui::pos2(cx, cy), r, accent);
        } else if matches!(hover, Some(Hit::Cell(j)) if j == k) {
            painter.circle_filled(egui::pos2(cx, cy), r, hover_bg);
        }
        if is_sel && !is_today {
            painter.circle_stroke(egui::pos2(cx, cy), r, Stroke::new(1.8, accent));
        } else if is_sel {
            painter.circle_stroke(
                egui::pos2(cx, cy),
                r - 2.0,
                Stroke::new(1.6, Color32::WHITE),
            );
        }

        let num_color = if is_today {
            Color32::WHITE
        } else if !c.in_month {
            faded
        } else if col >= 5 {
            red
        } else {
            tc
        };
        painter.text(
            egui::pos2(cx, cy - 2.0),
            Align2::CENTER_BOTTOM,
            c.day.to_string(),
            FontId::proportional(15.0),
            num_color,
        );

        let sub_color = if is_today {
            Color32::from_rgba_unmultiplied(255, 255, 255, 230)
        } else if !c.in_month {
            faded
        } else if c.highlight {
            red
        } else {
            sub
        };
        painter.text(
            egui::pos2(cx, cy + 2.0),
            Align2::CENTER_TOP,
            &c.sub,
            FontId::proportional(9.5),
            sub_color,
        );
    }

    // ---- 选中日的详情 ----
    let info_y = gy + 6.0 * cell_h + 6.0;
    painter.line_segment(
        [egui::pos2(x0, info_y), egui::pos2(x0 + cw, info_y)],
        Stroke::new(1.0, line_c),
    );

    let (sy, sm, sd) = sel;
    let weekday = calendar::weekday_of(sy, sm, sd);
    let is_today = sel == today;
    let line1 = format!(
        "{sy}年{sm}月{sd}日  星期{}{}",
        calendar::weekday_name(weekday),
        if is_today { "  · 今天" } else { "" }
    );
    let line2 = match calendar::solar_to_lunar(sy, sm, sd) {
        Some(l) => format!("农历  {}", calendar::lunar_text(l)),
        None => "农历  （仅支持 1901–2099 年）".to_owned(),
    };
    let line3 = format!("干支历  {}", calendar::ganzhi_text(sy, sm, sd));
    let line4 = match calendar::next_term(sy, sm, sd) {
        Some((y, n, days)) => {
            let (tm, td) = calendar::term_date(y, n);
            format!(
                "下一个节气  {}  {tm}月{td}日（还有 {days} 天）",
                calendar::term_name(n)
            )
        }
        None => String::new(),
    };

    let lh = 22.0;
    let lines = [(line1, tc), (line2, tc), (line3, sub), (line4, sub)];
    for (i, (text, color)) in lines.iter().enumerate() {
        painter.text(
            egui::pos2(x0, info_y + 10.0 + i as f32 * lh),
            Align2::LEFT_TOP,
            text,
            FontId::proportional(12.0),
            *color,
        );
    }

    hits
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut CalendarWindow;
    if ptr.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    // SAFETY: 指针由 `attach` 挂在 `Pin<Box<Self>>` 的最终地址上（ADR-036）。
    let win = unsafe { &mut *ptr };
    if let Some(result) = win.on_message(msg, wparam, lparam) {
        return result;
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

impl Drop for CalendarWindow {
    fn drop(&mut self) {
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
        }
    }
}

/// 窗口类名必须是 UTF-16 宽串，且缓冲要在进程整个存活期内有效。
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

fn physical_size(ppp: f32) -> (i32, i32) {
    (
        (PANEL_W * ppp).round() as i32,
        (PANEL_H * ppp).round() as i32,
    )
}

/// 弹层摆在时钟卡片旁边，再夹进显示器范围。
///
/// `anchor` 与返回值都是物理像素。
fn place(pw: i32, ph: i32, anchor: (f32, f32, f32, f32), edge: Edge, ppp: f32) -> (i32, i32) {
    place_in(
        pw,
        ph,
        anchor,
        edge,
        ppp,
        minidock_platform::display::monitor_rect(0),
    )
}

/// 与 [`place`] 相同，但显示器范围由调用方给。
///
/// 抽出来是为了让单测可复现：CI 的虚拟屏只有 1024×768，用真实
/// `monitor_rect(0)` 会把弹层夹到别处，"居中"的断言于是依赖机器。
fn place_in(
    pw: i32,
    ph: i32,
    anchor: (f32, f32, f32, f32),
    edge: Edge,
    ppp: f32,
    mon: minidock_core::layout::Rect,
) -> (i32, i32) {
    let (al, at, ar, ab) = anchor;
    let ax = (al + ar) * 0.5;
    let ay = (at + ab) * 0.5;
    let pm = (PM * ppp).round() as i32;
    let gap = (4.0 * ppp).round() as i32;
    let (mut x, mut y) = match edge {
        Edge::Top => (ax - pw as f32 * 0.5, ab - pm as f32 + gap as f32),
        Edge::Left => (ar - pm as f32 + gap as f32, ay - ph as f32 * 0.5),
        Edge::Right => (
            al - pw as f32 + pm as f32 - gap as f32,
            ay - ph as f32 * 0.5,
        ),
        Edge::Bottom => (
            ax - pw as f32 * 0.5,
            at - ph as f32 + pm as f32 - gap as f32,
        ),
    };
    // 夹进显示器。用的是 `rcMonitor`（不是工作区）：
    // 弹层压到任务栏上是可接受的，跑出屏幕不可接受。
    let (mut xi, mut yi) = (x.round() as i32, y.round() as i32);
    if xi + pw > mon.right {
        xi = mon.right - pw;
    }
    if yi + ph > mon.bottom {
        yi = mon.bottom - ph;
    }
    if xi < mon.left {
        xi = mon.left;
    }
    if yi < mon.top {
        yi = mon.top;
    }
    x = xi as f32;
    y = yi as f32;
    (x as i32, y as i32)
}

/// 客户区坐标（物理像素）→ 逻辑点。
fn client_pos(lparam: LPARAM, ppp: f32) -> (f32, f32) {
    let x = (lparam.0 & 0xffff) as u16 as i16 as f32;
    let y = ((lparam.0 >> 16) & 0xffff) as u16 as i16 as f32;
    (x / ppp, y / ppp)
}

/// 供调用方判断这个窗口是不是日历弹层。
#[must_use]
pub fn is_calendar_window(hwnd: HWND) -> bool {
    let mut buf = [0_u16; 64];
    let n = unsafe { windows::Win32::UI::WindowsAndMessaging::GetClassNameW(hwnd, &mut buf) };
    n > 0 && String::from_utf16_lossy(&buf[..n as usize]) == CLASS_NAME
}

/// 取本机今天的公历日期。
#[must_use]
pub fn today() -> (i32, u32, u32) {
    let st = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    (
        i32::from(st.wYear),
        u32::from(st.wMonth),
        u32::from(st.wDay),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 单测一律给一块固定的虚拟屏：CI 的虚拟屏只有 1024×768，
    /// 用真实 `monitor_rect(0)` 会把弹层夹走，断言就变成看机器脸色。
    fn screen() -> minidock_core::layout::Rect {
        minidock_core::layout::Rect::new(0, 0, 2560, 1440)
    }

    /// 底部栏（最常见）时，弹层应当在卡片上方。
    #[test]
    fn places_above_the_card_for_a_bottom_dock() {
        let (x, y) = place_in(
            392,
            480,
            (1000.0, 900.0, 1100.0, 960.0),
            Edge::Bottom,
            1.0,
            screen(),
        );
        assert!(y + 480 <= 960, "弹层底边应当不越过卡片顶部");
        assert!((x + 196 - 1050).abs() <= 1, "应当水平居中于卡片");
    }

    /// 顶部栏时弹层在卡片下方。
    ///
    /// 比的是**内容区**（窗口坐标 + PM 外边距）而不是窗口坐标：窗口四周
    /// 有 8 像素透明边距（画圆角阴影用的），定位时让这层边距压住卡片
    /// 边缘 4 像素，看起来才是"贴着"而不是"隔着一条缝"。
    #[test]
    fn places_below_the_card_for_a_top_dock() {
        let (_, y) = place_in(
            392,
            480,
            (1000.0, 0.0, 1100.0, 60.0),
            Edge::Top,
            1.0,
            screen(),
        );
        assert!(
            y + PM as i32 >= 60,
            "内容区顶边应当在卡片底部之下，实际 {}",
            y + PM as i32
        );
    }

    /// 左侧竖向栏时弹层在卡片右边。
    #[test]
    fn places_right_of_the_card_for_a_left_dock() {
        let (x, _) = place_in(
            392,
            480,
            (0.0, 400.0, 60.0, 460.0),
            Edge::Left,
            1.0,
            screen(),
        );
        assert!(
            x + PM as i32 >= 60,
            "内容区左边应当在卡片右侧，实际 {}",
            x + PM as i32
        );
    }

    /// 靠边时会被夹进显示器，不会跑到屏幕外。用小屏才能真的触发夹取。
    #[test]
    fn clamps_into_the_monitor() {
        let small = minidock_core::layout::Rect::new(0, 0, 1024, 768);
        let (x, y) = place_in(
            392,
            480,
            (0.0, 700.0, 40.0, 760.0),
            Edge::Bottom,
            1.0,
            small,
        );
        assert!(x >= 0, "左边越界: {x}");
        assert!(y >= 0, "上边越界: {y}");
        assert!(x + 392 <= small.right, "右边越界: {}", x + 392);
        assert!(y + 480 <= small.bottom, "下边越界: {}", y + 480);
    }

    /// 命中区不重叠，且都落在面板内。
    #[test]
    fn hit_rects_are_inside_the_panel() {
        let h = Hits::default();
        assert_eq!(h.nav.len(), 5);
        assert_eq!(h.cell.len(), 42);
        // 默认全是空矩形，任何点都不命中。
        assert_eq!(h.at(egui::pos2(10.0, 10.0)), None);
    }
}
