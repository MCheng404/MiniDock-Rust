//! 设置窗口：Dock 进程内的第二个分层窗口。
//!
//! 为什么不是独立进程（ADR-036）：
//!
//! - **改设置要即时生效**。同一进程里 `settings_ui` 直接改的是 Dock 用的那份
//!   `Config`，下一帧就在栏体上体现出来；独立进程要么走 IPC，要么让用户重启。
//! - 上游 C 版本来也是进程内对话框（`DialogBox`），行为口径一致。
//! - GPU device 可以共享：第二个窗口只需要自己的 `FrameRenderer`（离屏目标
//!   + DIB），管线与字体仍在第一次付费（ADR-032 已验证过这套做法）。
//!
//! 与菜单窗口（ADR-032）的差别：
//!
//! - 菜单是**模态**的（`SetCapture` + 自己的消息循环），设置窗口是**非模态**
//!   的 —— 用户要能看着栏体的变化调参数。
//! - 因此需要 `WS_EX_APPWINDOW`：让它出现在任务栏与 Alt+Tab 里，否则窗口被
//!   别的窗口盖住之后就再也找不回来了（我们没有托盘可以兜底）。
//! - 也因此**不能**用 `WM_MOUSEACTIVATE → MA_NOACTIVATE`：颜色选择器、未来的
//!   文本输入都需要焦点。菜单不需要焦点，设置窗口需要。

use std::pin::Pin;
use std::time::{Duration, Instant};

use egui::{Align, Color32, CornerRadius, Layout, RichText};
use minidock_core::config::{Config, Key};
use minidock_settings::{settings_ui, Command, Outcome, SettingsState};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_CONTROL, VK_MENU, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetForegroundWindow, GetWindowLongPtrW, LoadCursorW,
    RegisterClassW, SetCursor, SetForegroundWindow, SetWindowLongPtrW, ShowWindow, GWLP_USERDATA,
    IDC_ARROW, IDC_HAND, IDC_IBEAM, SW_HIDE, SW_SHOW, WM_ERASEBKGND, WM_KEYDOWN, WM_KEYUP,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT, WM_RBUTTONDOWN,
    WM_RBUTTONUP, WM_SETCURSOR, WNDCLASSW, WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_TOPMOST, WS_POPUP,
};

use crate::frame::FrameRenderer;
use crate::gpu::{Gpu, GpuError};
use crate::input::{map_vk, Input};

/// 窗口类名。`minidock-platform::single_instance` 不用它，留着是给今后
/// "已经有设置窗口就激活它"用的。
pub const CLASS_NAME: &str = "MiniDockSettings";

/// 窗口逻辑尺寸（本地坐标）。宽 880 是为了让最宽的性能页统计文本不用换行。
const LOGICAL_SIZE: (f32, f32) = (880.0, 720.0);

/// 设置窗口及其专属渲染器。
pub struct SettingsWindow {
    hwnd: HWND,
    renderer: FrameRenderer,
    input: Input,
    /// 屏幕左上角的物理坐标，`UpdateLayeredWindow` 需要。
    pos: (i32, i32),
    ppp: f32,
    /// UI 私有状态（当前页、显示器名、统计文本……），不属于 `Config`。
    pub state: SettingsState,
    /// 上一帧的输出。主循环取走并处理命令。
    pub outcome: Outcome,
    /// egui 说"尽快"时的最小帧间隔。
    next_repaint: Option<Instant>,
    /// Esc / 关闭按钮请求关闭。由主循环消费。
    pub close_requested: bool,
    visible: bool,
    /// 上一次设置的主题码。变了才写 visuals，避免每帧都动样式。
    ui_theme: Option<i32>,
    /// 帧耗时统计（性能页显示）。
    frame_ms: f32,
    samples: u64,
    last_stats_push: Instant,
}

impl std::fmt::Debug for SettingsWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsWindow")
            .field("hwnd", &(self.hwnd.0 as isize))
            .field("visible", &self.visible)
            .finish_non_exhaustive()
    }
}

/// 窗口销毁后清掉 `GWLP_USERDATA`。
///
/// 不清的话，窗口虽已销毁但假如还有排队的消息落到默认窗口过程——那倒没事；
/// 真正危险的是**再建一个同名的响应式窗口**时地址复用，旧指针还会看着像有效。
impl Drop for SettingsWindow {
    fn drop(&mut self) {
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
        }
    }
}

impl SettingsWindow {
    /// 建窗（默认隐藏）并准备好渲染器。
    ///
    /// 返回 `Pin<Box<Self>>` 而不是裸值：窗口过程要通过 `GWLP_USERDATA`
    /// 找回它，**地址必须稳定**。裸值从这儿返回后被 `Box`/`Option` 挪一次，
    /// 窗口过程手里就是失效地址，下一次消息就是野指针读 —— 实测表现为
    /// "点一下设置窗口进程就崩"，且没有任何 Rust panic 输出。
    ///
    /// 第一次构造会慢几十毫秒：装 CJK 字体 + 编译 wgpu 管线。
    pub fn new(gpu: &Gpu, state: SettingsState, ppp: f32) -> Result<Pin<Box<Self>>, GpuError> {
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

        let (pw, ph) = physical_size(ppp);
        let (x, y) = centered_origin(pw, ph);
        let hwnd = unsafe {
            CreateWindowExW(
                // WS_EX_LAYERED 是 UpdateLayeredWindow 的前提；
                // WS_EX_APPWINDOW 让它进任务栏（见模块文档）；
                // WS_EX_TOPMOST 跟随 Dock 的层级 —— 不置顶的话别的窗口一盖上来，
                // 用户既点不到它、也没有托盘图标能把它唤回来。
                WS_EX_LAYERED | WS_EX_APPWINDOW | WS_EX_TOPMOST,
                class_pcwstr(),
                windows::core::w!("MiniDock 设置"),
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
        .map_err(|_| GpuError::Win32("CreateWindowExW（设置窗口）"))?;

        let renderer = FrameRenderer::new(gpu, pw as u32, ph as u32, ppp)?;
        let mut win = Box::pin(Self {
            hwnd,
            renderer,
            input: Input::default(),
            pos: (x, y),
            ppp,
            state,
            outcome: Outcome::default(),
            next_repaint: None,
            close_requested: false,
            visible: false,
            ui_theme: None,
            frame_ms: 0.0,
            samples: 0,
            last_stats_push: Instant::now(),
        });
        // 挂载必须在最终地址上做，`Pin<Box<..>>` 保证了这一点。
        win.attach();
        Ok(win)
    }

    #[must_use]
    pub const fn hwnd(&self) -> HWND {
        self.hwnd
    }

    #[must_use]
    pub const fn visible(&self) -> bool {
        self.visible
    }

    /// 把自身指针挂到 `GWLP_USERDATA`，让 `wndproc` 能找回状态。
    ///
    /// 只能由 `new` 在 `Pin<Box<Self>>` 的最终地址上调用 —— 之后再挪动整个值
    /// 就会让这里存的地址失效（见 `new` 的文档）。
    fn attach(&mut self) {
        unsafe {
            SetWindowLongPtrW(
                self.hwnd,
                GWLP_USERDATA,
                std::ptr::from_mut::<Self>(self) as isize,
            );
        }
    }

    /// 显示并激活。已经显示时只把它提到前面：重复 `SetForegroundWindow`
    /// 在同一进程内是被允许的，不会有前台锁问题。
    pub fn show(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            if GetForegroundWindow() != self.hwnd {
                let _ = SetForegroundWindow(self.hwnd);
            }
        }
        self.visible = true;
        self.next_repaint = Some(Instant::now());
    }

    pub fn hide(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.visible = false;
        self.close_requested = false;
        // 窗口藏起来就别再催并重绘了，否则每 16 ms 一帧纯浪费。
        self.next_repaint = None;
    }

    /// 到下一帧的时间。`None` = 没有待办，主循环可以阻塞等消息。
    #[must_use]
    pub fn due(&self) -> bool {
        self.visible && self.next_repaint.is_some_and(|t| t <= Instant::now())
    }

    /// 画一帧并呈现。返回 egui 想要的光标与下一次重绘延时。
    ///
    /// `cfg` 是 Dock 用的同一份；`settings_ui` 在里面改的东西下一帧就生效，
    /// 不需要任何复制或通知。
    pub fn render(&mut self, gpu: &Gpu, cfg: &mut Config) -> Result<(), GpuError> {
        let started = Instant::now();
        self.apply_theme(cfg.get_i32(Key::UiTheme));

        let raw = self.input.take(self.renderer.size(), self.ppp);
        if trace() && !raw.events.is_empty() {
            eprintln!("[settings] 交给 egui 的输入事件：{:?}", raw.events);
        }
        let pos = self.pos;
        let hwnd = self.hwnd;
        let state = &mut self.state;
        let mut outcome = Outcome::default();
        let out = self.renderer.render(gpu, raw, hwnd, pos, |ui| {
            shell(ui, state, cfg, &mut outcome);
        })?;

        self.outcome = outcome;
        if trace() && (self.outcome.changed || self.outcome.command.is_some()) {
            eprintln!(
                "[settings] UI 输出 changed={} command={:?}",
                self.outcome.changed, self.outcome.command
            );
        }
        // 只有"关闭"会让窗口消失。其余命令（增删条目、打开文件夹……）让窗口
        // 留在原地：用户加完一项就想看到栏体的变化。
        if self.outcome.command == Some(Command::Close) {
            self.close_requested = true;
        }
        self.cursor_to_system(out.cursor);
        self.update_stats(started.elapsed().as_secs_f32() * 1000.0);

        // egui 自己知道下一次该在什么时候画（`Duration::MAX` = 没有待办）。
        // "越快越好"给 0 —— Dock 的目标就是 60 fps，按 16 ms 排既不掉帧也不空转。
        self.next_repaint = if out.repaint_delay == Duration::MAX {
            None
        } else if out.repaint_delay.is_zero() {
            Some(Instant::now() + Duration::from_millis(16))
        } else {
            Some(Instant::now() + out.repaint_delay)
        };
        Ok(())
    }

    /// 设置窗口有自己的主题设置（`UiTheme`），与栏体主题（`DockTheme`）无关。
    ///
    /// 0 = 浅色 / 1 = 深色 / 2 = 跟随系统。只对第一次和变化时的帧调用：
    /// `set_visuals` 会让 egui 认为样式变了，每帧调用等于每帧都重绘。
    fn apply_theme(&mut self, mode: i32) {
        if self.ui_theme == Some(mode) {
            return;
        }
        self.ui_theme = Some(mode);
        let dark = match mode {
            1 => true,
            2 => minidock_platform::theme::is_dark(),
            _ => false,
        };
        let visuals = if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        self.renderer.ctx().set_visuals(visuals);
    }

    fn update_stats(&mut self, dt: f32) {
        self.frame_ms = if self.frame_ms <= 0.0 {
            dt
        } else {
            // 指数平滑：单帧毛刺不该把读数打成异常值。
            self.frame_ms * 0.9 + dt * 0.1
        };
        self.samples += 1;
        if self.last_stats_push.elapsed() >= Duration::from_millis(400) {
            self.last_stats_push = Instant::now();
            self.state.stats = format!(
                "设置窗口 {:.2} ms/帧 · {:.0} fps · {:.1} 万次 Present",
                self.frame_ms,
                1000.0 / self.frame_ms.max(0.01),
                self.samples as f32 / 10000.0
            );
        }
    }

    fn cursor_to_system(&self, icon: egui::CursorIcon) {
        let id = match icon {
            egui::CursorIcon::PointingHand => IDC_HAND,
            egui::CursorIcon::Text => IDC_IBEAM,
            _ => IDC_ARROW,
        };
        if let Ok(handle) = unsafe { LoadCursorW(None, id) } {
            unsafe { SetCursor(Some(handle)) };
        }
    }

    fn on_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        if trace() {
            eprintln!("[settings-proc] msg=0x{msg:04X} wparam=0x{:X}", wparam.0);
        }
        match msg {
            WM_MOUSEMOVE => {
                let (x, y) = client_pos(lparam, self.ppp);
                let first = !self.input.hovering();
                self.input.pointer_moved(x, y);
                if first {
                    // 不订阅 TME_LEAVE 就永远收不到 WM_MOUSELEAVE，悬停状态会卡住。
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: self.hwnd,
                        dwHoverTime: 0,
                    };
                    unsafe {
                        let _ = TrackMouseEvent(&mut tme);
                    };
                }
                self.next_repaint = Some(Instant::now());
                None
            }
            WM_MOUSELEAVE => {
                self.input.pointer_gone();
                self.next_repaint = Some(Instant::now());
                None
            }
            WM_LBUTTONDOWN | WM_RBUTTONDOWN => {
                self.input.pointer_button(
                    true,
                    if msg == WM_LBUTTONDOWN {
                        egui::PointerButton::Primary
                    } else {
                        egui::PointerButton::Secondary
                    },
                );
                self.next_repaint = Some(Instant::now());
                None
            }
            WM_LBUTTONUP | WM_RBUTTONUP => {
                self.input.pointer_button(
                    false,
                    if msg == WM_LBUTTONUP {
                        egui::PointerButton::Primary
                    } else {
                        egui::PointerButton::Secondary
                    },
                );
                self.next_repaint = Some(Instant::now());
                None
            }
            WM_MOUSEWHEEL => {
                let delta = ((wparam.0 >> 16) as u16 as i16) as f32 / 120.0;
                self.input.wheel(delta);
                self.next_repaint = Some(Instant::now());
                None
            }
            WM_KEYDOWN | WM_KEYUP => {
                self.input.set_modifiers(current_modifiers());
                let vk = wparam.0 as u32;
                if let Some(key) = map_vk(vk) {
                    self.input.key(key, msg == WM_KEYDOWN);
                    if key == egui::Key::Escape && msg == WM_KEYDOWN {
                        self.close_requested = true;
                    }
                }
                self.next_repaint = Some(Instant::now());
                None
            }
            WM_SETCURSOR => {
                // 返回 TRUE：告诉系统"光标我自己管"，否则会被类光标覆盖。
                self.cursor_to_system(egui::CursorIcon::Default);
                Some(LRESULT(1))
            }
            WM_PAINT => {
                // 内容全部由分层窗口的合成器给，`BeginPaint/EndPaint` 只是
                // 为了让系统认为这块区域已经画过了，不再发 WM_PAINT。
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

/// 窗口外壳：背景 + 标题栏 + 关闭按钮，中间留给 `settings_ui`。
///
/// 放在这一层而不是改 `pages.rs`：那边的 kittest 会遍历控件，多一个按钮就要
/// 改好几条断言；而对"设置窗口要不要有关闭按钮"这个问题，答案是肯定的、
/// 且与页面无关。
fn shell(ui: &mut egui::Ui, state: &mut SettingsState, cfg: &mut Config, out: &mut Outcome) {
    let rect = ui.max_rect();
    let dark = ui.visuals().dark_mode;
    // 250/255：圆角外侧留一点透明让边缘融进桌面，又不至于让壁纸干扰阅读。
    let bg = if dark {
        Color32::from_rgba_unmultiplied(24, 25, 30, 250)
    } else {
        Color32::from_rgba_unmultiplied(246, 247, 250, 250)
    };
    ui.painter().rect_filled(rect, CornerRadius::same(20), bg);

    let inner = rect.shrink(18.0);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::top_down(Align::Min)),
    );

    child.horizontal(|ui| {
        ui.heading(RichText::new("MiniDock 设置").size(20.0));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("关闭").clicked() {
                out.command = Some(Command::Close);
            }
        });
    });
    child.separator();

    let mut page_out = settings_ui(&mut child, cfg, state);
    out.changed |= page_out.changed;
    if out.command.is_none() {
        out.command = page_out.command.take();
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut SettingsWindow;
    if ptr.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    // SAFETY: 指针由 `attach` 挂上，指向调用方持有的 `Box<SettingsWindow>`；
    // 窗口过程只在 UI 线程调用，且不与主循环的渲染阶段重叠。
    let win = unsafe { &mut *ptr };
    if let Some(result) = win.on_message(msg, wparam, lparam) {
        return result;
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// 窗口类名必须是 UTF-16 宽串，且缓冲要在进程整个存活期内有效。
///
/// 直接在 `new()` 里 encode 成一个临时 `Vec<u16>` 再传给 `CreateWindowExW`
/// 是错误的：函数返回后那块内存就释放了，之后谁再用这个类名都是用野指针
/// 访问已释放的堆（本次实测就是窗口压根没创建出来）。
/// 要不要打印诊断：窗口消息流 + 交给 egui 的输入事件。
///
/// 按项目惯例挂在环境变量下（`MINIDOCK_SETTINGS_TRACE=1`）：这类输出每帧
/// 都有，常开着会把真正的错误信息淹掉。
fn trace() -> bool {
    std::env::var_os("MINIDOCK_SETTINGS_TRACE").is_some()
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

fn physical_size(ppp: f32) -> (i32, i32) {
    (
        (LOGICAL_SIZE.0 * ppp).round() as i32,
        (LOGICAL_SIZE.1 * ppp).round() as i32,
    )
}

/// 窗口居中。用工作区而不是屏幕矩形：Dock 预留了边时，居中于屏幕会偏。
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

fn current_modifiers() -> egui::Modifiers {
    let down = |vk: u16| unsafe { (GetKeyState(i32::from(vk)) as u16 & 0x8000) != 0 };
    egui::Modifiers {
        alt: down(VK_MENU.0),
        ctrl: down(VK_CONTROL.0),
        shift: down(VK_SHIFT.0),
        mac_cmd: false,
        command: down(VK_CONTROL.0),
    }
}
