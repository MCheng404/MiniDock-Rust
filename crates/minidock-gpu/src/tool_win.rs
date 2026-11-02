//! 计时器 / 待办面板：点条带上的钟表卡片弹出来。
//!
//! 对应上游 v1.6.0 的 `ShowTool()` / `ToolDraw()` / `ToolMove()` / `ToolUp()`
//! （minidock.c:6455-6510）。
//!
//! 与资源监视器面板（ADR-039）的差别：**这个要交互**。上游用 `RECT` 数组
//! 记每个按钮/待办行的命中区，`ToolMove` 里逐个数。这里照同样的做法 ——
//! 面板用 painter 显式定位，命中区在绘制时一并算出来存进 `Hits`。
//!
//! 为什么不用 egui 的控件：ADR-039 已经踩过——`with_layout(right_to_left)`
//! 在 `horizontal` 里拿不到剩余宽度，右边的东西会消失。面板类 UI 一律
//! 自己算矩形、自己判命中。

use std::pin::Pin;
use std::time::{Duration, Instant};

use egui::{Align2, Color32, CornerRadius, FontId, Rect as EguiRect};
use minidock_core::tool::{format_ms, Todo, ToolMode, ToolState, MAX_TODOS};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetWindowLongPtrW, LoadCursorW, RegisterClassW, SetCursor,
    SetWindowLongPtrW, ShowWindow, GWLP_USERDATA, IDC_ARROW, IDC_HAND, SW_HIDE, SW_SHOW,
    WM_ERASEBKGND, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SETCURSOR, WNDCLASSW,
    WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_TOPMOST, WS_POPUP,
};

use crate::frame::FrameRenderer;
use crate::gpu::{Gpu, GpuError};
use crate::ime::Ime;

/// 窗口类名。
pub const CLASS_NAME: &str = "MiniDockTool";

/// 面板宽度。上游是 `Sc(360)`（v1.6.0:6487）。
const PANEL_W: f32 = 360.0;

/// 没有待办时的面板高度。
///
/// 上游固定 `Sc(470)`，但那条高是按 16 条待办算的 —— 少于 16 条时
/// 下面是一大片空白。这里按实际条数算，见 [`panel_h`]。
const PANEL_H_BASE: f32 = 330.0;

/// 每条待办占的高度（逻辑像素）。
const TODO_ROW_H: f32 = 25.0;

/// 面板高度：底高 + 每条待办一行，上限 640（别超出屏幕）。
fn panel_h(todos: usize) -> f32 {
    (PANEL_H_BASE + todos as f32 * TODO_ROW_H).min(640.0)
}

/// 跑着的时候的重绘间隔。
///
/// 秒表要显示到 0.01 秒，所以不能等到 1 秒。「重绘」在这里只是重画
/// 一个数字，代价很低。
const TICK: Duration = Duration::from_millis(50);

/// 面板上的可点区域。绘制时算出来，鼠标事件里用。
///
/// 对应上游 `ToolU.rb[5]` + `tr[17]`。
#[derive(Clone, Debug)]
struct Hits {
    /// 0 = 计时，1 = 秒表。
    mode: [EguiRect; 2],
    /// 0 = 开始/暂停，1 = 重置，2 = 减 1 分钟，3 = 加 1 分钟。
    action: [EguiRect; 4],
    /// 待办行（最后一项是「添加」）。
    todos: Vec<EguiRect>,
    /// 待办行右侧的删除按钮。
    deletes: Vec<EguiRect>,
    /// 「添加待办」行。
    add: EguiRect,
}

impl Default for Hits {
    fn default() -> Self {
        // `egui::Rect` 没有 `Default`，用空矩形占位 ——
        // 空矩形的 `contains` 恒为假，不会误命中。
        let n = EguiRect::NOTHING;
        Self {
            mode: [n; 2],
            action: [n; 4],
            todos: Vec::new(),
            deletes: Vec::new(),
            add: n,
        }
    }
}

impl Hits {
    /// 鼠标落在哪个可点区域。
    fn at(&self, p: egui::Pos2) -> Option<Hit> {
        let modes = [ToolMode::Countdown, ToolMode::Stopwatch];
        for (i, r) in self.mode.iter().enumerate() {
            if r.contains(p) {
                return Some(Hit::Mode(modes[i]));
            }
        }
        let actions = [
            Action::ToggleRun,
            Action::Reset,
            Action::MinusMinute,
            Action::PlusMinute,
        ];
        for (i, r) in self.action.iter().enumerate() {
            if r.contains(p) {
                return Some(Hit::Action(actions[i]));
            }
        }
        for (i, r) in self.deletes.iter().enumerate() {
            if r.contains(p) {
                return Some(Hit::Delete(i));
            }
        }
        for (i, r) in self.todos.iter().enumerate() {
            if r.contains(p) {
                return Some(Hit::Todo(i));
            }
        }
        if self.add.contains(p) {
            return Some(Hit::Add);
        }
        None
    }
}

/// 动作按钮。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Action {
    /// 开始 / 暂停。
    ToggleRun,
    /// 重置。
    Reset,
    /// 总时长减一分钟。
    MinusMinute,
    /// 总时长加一分钟。
    PlusMinute,
}

/// 一次点击命中的目标。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Hit {
    Mode(ToolMode),
    Action(Action),
    Todo(usize),
    Delete(usize),
    Add,
}

/// 计时器 / 待办面板。
pub struct ToolWindow {
    hwnd: HWND,
    renderer: FrameRenderer,
    input: crate::input::Input,
    pos: (i32, i32),
    /// 当前窗口尺寸（物理像素），用来判断要不要重排。
    size: (i32, i32),
    ppp: f32,
    /// 计时器与待办的状态。与 ini 同步。
    pub state: ToolState,
    /// 上一帧算出来的命中区。
    hits: Hits,
    /// 鼠标位置（逻辑点），用来画悬停高亮。
    hover: Option<egui::Pos2>,
    /// 下一次重绘的时刻。
    next_repaint: Instant,
    visible: bool,
    dark: bool,
    /// 状态脏了，需要写盘。
    pub dirty: bool,
    /// 请求关闭（Esc 或点外面）。
    pub close_requested: bool,
    /// 击键缓冲：添加待办时逐字输入。
    pub typed: String,
    /// 是否在输入待办文本。
    typing: bool,
    /// 输入法状态（待办文本可以是中文）。
    ime: Ime,
}

impl std::fmt::Debug for ToolWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolWindow")
            .field("hwnd", &(self.hwnd.0 as isize))
            .field("visible", &self.visible)
            .field("running", &self.state.running)
            .finish_non_exhaustive()
    }
}

impl ToolWindow {
    /// 建窗（默认隐藏）。
    pub fn new(
        gpu: &Gpu,
        ppp: f32,
        dark: bool,
        state: ToolState,
    ) -> Result<Pin<Box<Self>>, GpuError> {
        let hinst =
            unsafe { GetModuleHandleW(None) }.map_err(|_| GpuError::Win32("GetModuleHandleW"))?;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: windows::Win32::Foundation::HINSTANCE(hinst.0),
            lpszClassName: class_pcwstr(),
            ..Default::default()
        };
        unsafe {
            RegisterClassW(&raw const wc);
        }

        let (pw, ph) = physical_size(ppp, panel_h(state.todos.len()));
        let (x, y) = centered_origin(pw, ph);
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_APPWINDOW | WS_EX_TOPMOST,
                class_pcwstr(),
                windows::core::w!("计时器 · 待办"),
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
        .map_err(|_| GpuError::Win32("CreateWindowExW（计时器面板）"))?;

        let renderer = FrameRenderer::new(gpu, pw as u32, ph as u32, ppp)?;
        let mut win = Box::pin(Self {
            hwnd,
            renderer,
            input: crate::input::Input::default(),
            pos: (x, y),
            size: (pw, ph),
            ppp,
            state,
            hits: Hits::default(),
            hover: None,
            next_repaint: Instant::now(),
            visible: false,
            dark,
            dirty: false,
            close_requested: false,
            typed: String::new(),
            typing: false,
            ime: Ime::new(),
        });
        win.attach();
        // 待办输入框要能打中文，先确保窗口有输入法上下文。
        Ime::enable(hwnd);
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
    pub const fn visible(&self) -> bool {
        self.visible
    }

    pub fn show(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
        }
        self.visible = true;
        self.next_repaint = Instant::now();
    }

    pub fn hide(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.visible = false;
        self.typing = false;
        self.typed.clear();
    }

    /// 是否需要下一帧。
    #[must_use]
    pub fn due(&self) -> bool {
        self.visible && self.next_repaint <= Instant::now()
    }

    /// 画一帧。`now_ms` 是调用方的时间基准。
    pub fn render(&mut self, gpu: &Gpu, now_ms: i64) -> Result<(), GpuError> {
        // 倒计时归零要停表。
        self.state.tick(now_ms);
        // 待办增减后面板高度要跟着变（空白太多不好看）。
        self.fit_height(gpu)?;

        let raw = self.input.take(self.renderer.size(), self.ppp);
        let pos = self.pos;
        let hwnd = self.hwnd;
        let dark = self.dark;
        let state = &self.state;
        let hover = self.hover;
        let typing = self.typing;
        let typed = self.typed.clone();
        let composing = self.ime.composing().to_owned();
        let mut hits = Hits::default();

        self.renderer.render(gpu, raw, hwnd, pos, |ui| {
            hits = draw_panel(
                ui,
                dark,
                state,
                now_ms,
                hover,
                Input {
                    typing,
                    typed: &typed,
                    composing: &composing,
                },
            );
        })?;
        self.hits = hits;

        // 跑着的时候按 50 ms 重画（秒表要显示 0.01 秒）；
        // 停着的时候不催帧。
        self.next_repaint = if self.state.running {
            Instant::now() + TICK
        } else {
            Instant::now() + Duration::from_secs(3600)
        };
        Ok(())
    }

    /// 按当前待办条数调整窗口高度（居中重摆）。
    ///
    /// 只重建离屏目标与 DIB，保留 egui 上下文与 wgpu 管线
    /// （`FrameRenderer::resize` 的语义，ADR-032）。
    fn fit_height(&mut self, gpu: &Gpu) -> Result<(), GpuError> {
        let (pw, ph) = physical_size(self.ppp, panel_h(self.state.todos.len()));
        if (pw, ph) == (self.size.0, self.size.1) {
            return Ok(());
        }
        self.renderer.resize(gpu, pw as u32, ph as u32)?;
        self.size = (pw, ph);
        let (x, y) = centered_origin(pw, ph);
        self.pos = (x, y);
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
                self.hwnd,
                None,
                x,
                y,
                pw,
                ph,
                windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER
                    | windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
            );
        }
        Ok(())
    }

    fn on_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        // 输入法优先：拼音的组合过程不产生 `WM_CHAR`，只走 `WM_IME_*`。
        if matches!(
            msg,
            windows::Win32::UI::WindowsAndMessaging::WM_IME_STARTCOMPOSITION
                | windows::Win32::UI::WindowsAndMessaging::WM_IME_COMPOSITION
                | windows::Win32::UI::WindowsAndMessaging::WM_IME_ENDCOMPOSITION
                | windows::Win32::UI::WindowsAndMessaging::WM_IME_SETCONTEXT
        ) {
            let mut committed = String::new();
            // 只有在输入态才收字；否则上屏的文字没地方放。
            let handled = self
                .ime
                .handle(self.hwnd, msg, wparam, lparam, &mut committed);
            if self.typing && !committed.is_empty() {
                for c in committed.chars() {
                    if self.typed.chars().count() < 90 {
                        self.typed.push(c);
                    }
                }
            }
            if msg == windows::Win32::UI::WindowsAndMessaging::WM_IME_STARTCOMPOSITION
                && self.typing
            {
                // 候选窗摆到输入行附近（面板底部）。
                let y = self.size.1 as f32 / self.ppp - 34.0;
                self.ime.set_caret(self.hwnd, 16.0, y, self.ppp);
            }
            self.next_repaint = Instant::now();
            if handled {
                return Some(LRESULT(0));
            }
            return Some(LRESULT(Ime::default_proc(self.hwnd, msg, wparam, lparam)));
        }
        match msg {
            WM_MOUSEMOVE => {
                // 有一个像素宽的问题值得记：`WM_MOUSEMOVE` 的 lparam 是
                // 客户区坐标（物理），要除以 ppp 才是逻辑点。
                let (x, y) = client_pos(lparam, self.ppp);
                let p = egui::pos2(x, y);
                if self.hover != Some(p) {
                    self.hover = Some(p);
                    self.next_repaint = Instant::now();
                }
                None
            }
            WM_LBUTTONUP => {
                let (x, y) = client_pos(lparam, self.ppp);
                let p = egui::pos2(x, y);
                if let Some(hit) = self.hits.at(p) {
                    self.click(hit);
                    self.next_repaint = Instant::now();
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN => Some(LRESULT(0)),
            WM_SETCURSOR => {
                // 可点区域给手型光标。分层窗口不吃 `WM_NCHITTEST`，
                // 光标要自己设。
                let cursor = if self.hover.and_then(|p| self.hits.at(p)).is_some() {
                    IDC_HAND
                } else {
                    IDC_ARROW
                };
                if let Ok(h) = unsafe { LoadCursorW(None, cursor) } {
                    unsafe { SetCursor(Some(h)) };
                }
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

    /// 执行一次点击。对应上游 `ToolUp()`。
    fn click(&mut self, hit: Hit) {
        let now = now_ms();
        match hit {
            Hit::Mode(mode) => {
                self.state.set_mode(mode);
                self.dirty = true;
            }
            Hit::Action(Action::ToggleRun) => {
                self.state.toggle_run(now);
                self.dirty = true;
            }
            Hit::Action(Action::Reset) => {
                self.state.reset();
                self.dirty = true;
            }
            // 减 / 加一分钟：上游没有这个功能，但倒计时默认写死 5 分钟、
            // 界面上改不了，等于这个小组件只有一种时长可用。
            // 这里补上两个按钮，改动写在 ADR-040。
            Hit::Action(Action::MinusMinute) => {
                self.state.total_ms = (self.state.total_ms - 60_000).max(60_000);
                self.state.reset();
                self.dirty = true;
            }
            Hit::Action(Action::PlusMinute) => {
                self.state.total_ms = (self.state.total_ms + 60_000).min(99 * 60_000);
                self.state.reset();
                self.dirty = true;
            }
            Hit::Todo(i) => {
                if self.state.toggle_todo(i) {
                    self.dirty = true;
                }
            }
            Hit::Delete(i) => {
                if self.state.remove_todo(i) {
                    self.dirty = true;
                }
            }
            Hit::Add => {
                // 输入待办文本：先进入"输入中"状态，字符由键盘消息累积，
                // 回车提交。**这是中文输入法的边界** —— `WM_CHAR` 拿得到
                // 已上屏的字符，走 IME 组合中的候选还没处理
                // （见 PORTING 的 IME 待办）。英文与粘贴可用。
                if self.state.todos.len() >= MAX_TODOS {
                    return;
                }
                self.typing = true;
                self.typed.clear();
            }
        }
    }

    /// 键盘：输入待办文本时用。
    fn on_char(&mut self, ch: u32) -> bool {
        if !self.typing {
            return false;
        }
        match char::from_u32(ch) {
            Some('\r') | Some('\n') => {
                if !self.typed.trim().is_empty() {
                    self.state.add_todo(&self.typed.clone());
                    self.dirty = true;
                }
                self.typed.clear();
                self.typing = false;
            }
            Some('\u{1b}') => {
                self.typed.clear();
                self.typing = false;
            }
            Some('\u{8}') => {
                self.typed.pop();
            }
            // 90 字符的硬上限：`Todo::new` 会截到 96，但输入时先挡住，
            // 免得用户敲了半天再被无声截断。
            Some(c) if !c.is_control() && self.typed.chars().count() < 90 => {
                self.typed.push(c);
            }
            _ => {}
        }
        true
    }
}

/// 画面板，返回本帧的命中区。
/// 待办输入框的状态（三个字段一起传，否则这个函数参数就超过 7 个了）。
#[derive(Copy, Clone, Debug)]
struct Input<'a> {
    /// 是否在输入态。
    typing: bool,
    /// 已上屏的文字。
    typed: &'a str,
    /// 正在组合的拼音。
    composing: &'a str,
}

fn draw_panel(
    ui: &mut egui::Ui,
    dark: bool,
    state: &ToolState,
    now_ms: i64,
    hover: Option<egui::Pos2>,
    input: Input<'_>,
) -> Hits {
    let rect = ui.max_rect();
    let painter = ui.painter();
    let bg = if dark {
        Color32::from_rgba_unmultiplied(24, 25, 30, 250)
    } else {
        Color32::from_rgba_unmultiplied(246, 247, 250, 250)
    };
    painter.rect_filled(rect, CornerRadius::same(16), bg);

    let (tc, sub) = if dark {
        (
            Color32::from_rgba_unmultiplied(255, 242, 242, 246),
            Color32::from_rgba_unmultiplied(150, 200, 202, 214),
        )
    } else {
        (
            Color32::from_rgba_unmultiplied(255, 28, 28, 34),
            Color32::from_rgba_unmultiplied(150, 90, 94, 108),
        )
    };
    let accent = ui.visuals().selection.bg_fill;
    // 次级按钮的底：白色 60/255（深色）或黑色 26/255（浅色）。
    //
    // **参数顺序是 (r, g, b, a)**，别写成 (a, r, g, b) —— 那样会得到
    // 一个纯青色实心块（踩过：四个按钮全变亮青色）。
    let pill = if dark {
        Color32::from_rgba_unmultiplied(255, 255, 255, 60)
    } else {
        Color32::from_rgba_unmultiplied(0, 0, 0, 26)
    };
    let hover_bg = if dark {
        Color32::from_rgba_unmultiplied(255, 255, 255, 40)
    } else {
        Color32::from_rgba_unmultiplied(0, 0, 0, 20)
    };

    let pad = 16.0;
    let mut hits = Hits::default();
    let x0 = rect.left() + pad;
    let w = rect.width() - 2.0 * pad;

    // ---- 标题 ----
    painter.text(
        egui::pos2(rect.left() + pad, rect.top() + pad),
        Align2::LEFT_TOP,
        "计时器 · 待办",
        FontId::proportional(16.0),
        tc,
    );

    // ---- 模式分段（计时 / 秒表）----
    let sy = rect.top() + pad + 28.0;
    for (i, mode) in [ToolMode::Countdown, ToolMode::Stopwatch]
        .iter()
        .enumerate()
    {
        let r =
            EguiRect::from_min_size(egui::pos2(x0 + i as f32 * 68.0, sy), egui::vec2(64.0, 26.0));
        hits.mode[i] = r;
        let active = state.mode == *mode;
        painter.rect_filled(
            r,
            CornerRadius::same(13),
            if active { accent } else { pill },
        );
        painter.text(
            r.center(),
            Align2::CENTER_CENTER,
            mode.title(),
            FontId::proportional(12.0),
            if active { Color32::WHITE } else { sub },
        );
    }

    // ---- 大数字 ----
    let big_y = sy + 34.0;
    painter.text(
        egui::pos2(x0, big_y),
        Align2::LEFT_TOP,
        format_ms(state.display_ms(now_ms), state.mode),
        FontId::proportional(44.0),
        tc,
    );

    // ---- 四个按钮：开始/暂停、重置、−1 分、+1 分 ----
    let by = big_y + 62.0;
    let bw = (w - 8.0) / 2.0;
    let labels = [
        if state.running { "暂停" } else { "开始" },
        "重置",
        "− 1 分钟",
        "+ 1 分钟",
    ];
    for (i, label) in labels.iter().enumerate() {
        let col = i % 2;
        let row = i / 2;
        let r = EguiRect::from_min_size(
            egui::pos2(x0 + col as f32 * (bw + 8.0), by + row as f32 * 42.0),
            egui::vec2(bw, 34.0),
        );
        hits.action[i] = r;
        // 只有"开始/暂停"用强调色，其余是次级按钮。
        let fill = if i == 0 {
            accent
        } else if hover == Some(r.center()) || hover.is_some_and(|p| r.contains(p)) {
            hover_bg
        } else {
            pill
        };
        painter.rect_filled(r, CornerRadius::same(10), fill);
        painter.text(
            r.center(),
            Align2::CENTER_CENTER,
            *label,
            FontId::proportional(13.0),
            if i == 0 { Color32::WHITE } else { tc },
        );
    }

    // ---- 待办 ----
    let mut ty = by + 42.0 * 2.0 + 12.0;
    painter.text(
        egui::pos2(x0, ty),
        Align2::LEFT_TOP,
        "待办事项",
        FontId::proportional(13.0),
        tc,
    );
    ty += 22.0;

    for todo in &state.todos {
        let r = EguiRect::from_min_size(egui::pos2(x0, ty), egui::vec2(w, 24.0));
        hits.todos.push(r);
        let del = EguiRect::from_min_size(egui::pos2(r.right() - 24.0, ty), egui::vec2(24.0, 24.0));
        hits.deletes.push(del);
        if hover.is_some_and(|p| r.contains(p)) {
            painter.rect_filled(r, CornerRadius::same(8), hover_bg);
        }
        // 勾选框
        let box_rect =
            EguiRect::from_min_size(egui::pos2(x0 + 2.0, ty + 6.0), egui::vec2(12.0, 12.0));
        painter.rect_filled(
            box_rect,
            CornerRadius::same(6),
            if todo.done { accent } else { pill },
        );
        if todo.done {
            // 勾号用线段画，不用字符 —— 打包的字体里没有 `✓`，
            // 直接画字会得到一个豆腐块（实测过）。
            check_mark(painter, box_rect.center(), 3.0, Color32::WHITE);
        }
        painter.text(
            egui::pos2(x0 + 22.0, ty + 4.0),
            Align2::LEFT_TOP,
            &todo.text,
            FontId::proportional(12.0),
            if todo.done { sub } else { tc },
        );
        // 删除符号同理：`✕` 也不在字体里。
        cross_mark(painter, del.center(), 3.0, sub);
        ty += 25.0;
    }

    // ---- 添加行 / 输入框 ----
    let add = EguiRect::from_min_size(egui::pos2(x0, ty), egui::vec2(w, 24.0));
    hits.add = add;
    if input.typing {
        painter.rect_filled(add, CornerRadius::same(8), pill);
        // 已上屏的文字 + 正在组合的拼音。拼音下面画条线：让用户知道
        // "字还在输入法里，没丢"。
        let shown = format!("{}{}", input.typed, input.composing);
        if shown.is_empty() {
            painter.text(
                egui::pos2(x0 + 6.0, ty + 4.0),
                Align2::LEFT_TOP,
                "输入后回车…",
                FontId::proportional(12.0),
                sub,
            );
        } else {
            painter.text(
                egui::pos2(x0 + 6.0, ty + 4.0),
                Align2::LEFT_TOP,
                &shown,
                FontId::proportional(12.0),
                tc,
            );
            if !input.composing.is_empty() {
                let w_all = ui.ctx().fonts_mut(|f| {
                    f.layout_no_wrap(shown.clone(), FontId::proportional(12.0), Color32::WHITE)
                        .size()
                        .x
                });
                let w_done = ui.ctx().fonts_mut(|f| {
                    f.layout_no_wrap(
                        input.typed.to_owned(),
                        FontId::proportional(12.0),
                        Color32::WHITE,
                    )
                    .size()
                    .x
                });
                painter.line_segment(
                    [
                        egui::pos2(x0 + 6.0 + w_done, ty + 18.0),
                        egui::pos2(x0 + 6.0 + w_all, ty + 18.0),
                    ],
                    egui::Stroke::new(1.0, tc),
                );
            }
        }
    } else {
        if hover.is_some_and(|p| add.contains(p)) {
            painter.rect_filled(add, CornerRadius::same(8), hover_bg);
        }
        painter.text(
            egui::pos2(x0 + 2.0, ty + 4.0),
            Align2::LEFT_TOP,
            if state.todos.len() >= MAX_TODOS {
                "已达上限（16 条）"
            } else {
                "＋ 添加待办…"
            },
            FontId::proportional(12.0),
            sub,
        );
    }

    // ---- 底部提示：剩余条数 ----
    let pending = state.pending_count();
    if pending > 0 {
        painter.text(
            egui::pos2(rect.right() - pad, rect.top() + pad),
            Align2::RIGHT_TOP,
            format!("{pending} 项待办"),
            FontId::proportional(11.0),
            sub,
        );
    }

    hits
}

/// 画一个勾：两条线段，短的一撇 + 长的一捺。
///
/// 不用 `✓` 字符：打包的字体里没有这个字形，会渲染成豆腐块。
fn check_mark(painter: &egui::Painter, center: egui::Pos2, r: f32, color: Color32) {
    let stroke = egui::Stroke::new(1.6, color);
    let p0 = egui::pos2(center.x - r * 0.7, center.y);
    let p1 = egui::pos2(center.x - r * 0.15, center.y + r * 0.6);
    let p2 = egui::pos2(center.x + r * 0.75, center.y - r * 0.6);
    painter.line_segment([p0, p1], stroke);
    painter.line_segment([p1, p2], stroke);
}

/// 画一个叉：两条对角线。同样不用 `✕` 字符。
fn cross_mark(painter: &egui::Painter, center: egui::Pos2, r: f32, color: Color32) {
    let stroke = egui::Stroke::new(1.4, color);
    painter.line_segment(
        [
            egui::pos2(center.x - r, center.y - r),
            egui::pos2(center.x + r, center.y + r),
        ],
        stroke,
    );
    painter.line_segment(
        [
            egui::pos2(center.x + r, center.y - r),
            egui::pos2(center.x - r, center.y + r),
        ],
        stroke,
    );
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    const WM_CHAR: u32 = 0x0102;
    const WM_KEYDOWN: u32 = 0x0100;
    const VK_ESCAPE: usize = 0x1B;
    const VK_RETURN: usize = 0x0D;
    const VK_BACK: usize = 0x08;

    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut ToolWindow;
    if ptr.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    let win = unsafe { &mut *ptr };
    if let Some(result) = win.on_message(msg, wparam, lparam) {
        return result;
    }
    match msg {
        // 输入待办时的键盘处理。用 `WM_KEYDOWN` 转成字符而不是直接读
        // `WM_CHAR`：这样退格、回车、Esc 三个特殊键与字符走同一条路径。
        WM_KEYDOWN => {
            if win.typing {
                let handled = match wparam.0 {
                    VK_ESCAPE => win.on_char(0x1b),
                    VK_RETURN => win.on_char(b'\r' as u32),
                    VK_BACK => win.on_char(0x08),
                    _ => false,
                };
                if handled {
                    win.next_repaint = Instant::now();
                    return LRESULT(0);
                }
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_CHAR => {
            if win.on_char(wparam.0 as u32) {
                win.next_repaint = Instant::now();
                return LRESULT(0);
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

impl Drop for ToolWindow {
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

fn physical_size(ppp: f32, h: f32) -> (i32, i32) {
    ((PANEL_W * ppp).round() as i32, (h * ppp).round() as i32)
}

fn centered_origin(pw: i32, ph: i32) -> (i32, i32) {
    let work = minidock_platform::display::work_area(0);
    let cx = (work.left + work.right) / 2;
    let cy = (work.top + work.bottom) / 2;
    (cx - pw / 2, cy - ph / 2)
}

fn client_pos(lparam: LPARAM, ppp: f32) -> (f32, f32) {
    let x = (lparam.0 & 0xFFFF) as i16 as f32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
    (x / ppp, y / ppp)
}

/// 当前时刻的毫秒数（单调递增，用于计时）。
#[must_use]
pub fn now_ms() -> i64 {
    use std::sync::OnceLock;
    static START: OnceLock<Instant> = OnceLock::new();
    let start = START.get_or_init(Instant::now);
    i64::try_from(start.elapsed().as_millis()).unwrap_or(i64::MAX)
}

/// 面板上一行的文本（测试用）。
#[must_use]
pub fn summary(state: &ToolState, now_ms: i64) -> String {
    format!(
        "{} {}  待办 {}/{}",
        state.mode.title(),
        format_ms(state.display_ms(now_ms), state.mode),
        state.todos.iter().filter(|t| t.done).count(),
        state.todos.len()
    )
}

/// 空待办列表的占位（避免 `Todo` 未使用警告）。
#[allow(dead_code)]
fn _unused(_: Todo) {}
