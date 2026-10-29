//! 透明 Overlay 窗口：建窗、渲染、呈现、输入、统计。
//!
//! M0 的结论已经落进代码：
//! - 透明不走交换链（wgpu 没有 alpha 表面），改**离屏渲染 + 分层窗口**；
//! - 命中不再需要 `WM_NCHITTEST` —— 分层窗口天生按 alpha 判定；
//! - 只登记 `needs_frame`，真正的绘制在消息循环里做（`WndProc` 里画会重入）；
//! - 每帧记账（帧耗时 / RSS），性能页直接显示，尖刺本身就是测量工具。

use std::time::{Duration, Instant};

use egui::{Color32, CornerRadius};
use minidock_core::config::Config;
use minidock_settings::{settings_ui, Command, Outcome, SettingsState};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, UpdateWindow, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::INFINITE;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_CONTROL, VK_MENU, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetWindowLongPtrW,
    LoadCursorW, MsgWaitForMultipleObjectsEx, PeekMessageW, PostQuitMessage, RegisterClassW,
    SetCursor, SetForegroundWindow, SetWindowLongPtrW, ShowWindow, TranslateMessage, GWLP_USERDATA,
    IDC_ARROW, IDC_HAND, IDC_IBEAM, MSG, MWMO_INPUTAVAILABLE, PM_REMOVE, QS_ALLINPUT, SW_SHOW,
    WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_PAINT, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR, WNDCLASSW,
    WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::frame::FrameRenderer;
use crate::gpu::{Gpu, GpuError};
use crate::input::{map_vk, Input};
use crate::mem::MemStats;

pub const CLASS_NAME: &str = "MiniDockOverlay";

/// egui 说"尽快"时的最小帧间隔。
///
/// egui 返回 `Duration::ZERO` 只表示"这一帧别等"，不是"忽略成本地连画"。
/// Dock 的动画目标是 60 fps，按 16 ms 排既不掉帧也不空转；
/// 鼠标移动这类输入不走这条路 —— 它们直接置 `needs_frame`，立即出帧。
const FRAME_PACING: Duration = Duration::from_millis(16);

fn class_pcwstr() -> PCWSTR {
    static BUF: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    let buf = BUF.get_or_init(|| {
        CLASS_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect()
    });
    PCWSTR(buf.as_ptr())
}

#[derive(Clone, Copy, Debug)]
struct Layout {
    width: f32,
    height: f32,
}

/// 每帧记账。性能页显示的就是这些数字。
#[derive(Debug, Default)]
struct Stats {
    frame_ms: f32,
    rss_mb: f32,
    samples: u64,
}

impl Stats {
    fn fps(&self) -> f32 {
        if self.frame_ms > 0.0 {
            1000.0 / self.frame_ms
        } else {
            0.0
        }
    }
}

pub struct Overlay {
    hwnd: HWND,
    pos: (i32, i32),
    gpu: Gpu,
    frame: FrameRenderer,
    input: Input,
    layout: Layout,
    pixels_per_point: f32,
    cfg: Config,
    settings: SettingsState,
    commands: Vec<Command>,
    needs_frame: bool,
    cursor: egui::CursorIcon,
    stats: Stats,
    last_stats_push: Instant,
    /// 下一次该画的时间；`None` = 没有待办，阻塞等消息。
    next_repaint_at: Option<Instant>,
    /// 上一次打印追踪的时间（仅 `MINIDOCK_TRACE` 时使用）。
    last_trace: Instant,
    /// 上一次打印时的帧数，用来算每秒帧数。
    last_trace_frames: u64,
}

impl std::fmt::Debug for Overlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Overlay")
            .field("hwnd", &(self.hwnd.0 as isize))
            .field("size", &(self.layout.width, self.layout.height))
            .finish_non_exhaustive()
    }
}

impl Overlay {
    pub fn open(
        width: f32,
        height: f32,
        cfg: Config,
        mut settings: SettingsState,
    ) -> Result<Self, GpuError> {
        // 必须在建窗之前声明：否则进程被当成 DPI 不感知，Windows 会整体拉伸
        // 窗口位图，表面尺寸与客户区错位（M0 第一版实测 1320×1080 vs 880×720）。
        dpi_awareness();

        let hinstance =
            unsafe { GetModuleHandleW(None) }.map_err(|_| GpuError::Win32("GetModuleHandleW"))?;

        let wc = WNDCLASSW {
            hInstance: windows::Win32::Foundation::HINSTANCE(hinstance.0),
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
            lpfnWndProc: Some(wndproc),
            lpszClassName: class_pcwstr(),
            ..Default::default()
        };
        unsafe { RegisterClassW(&wc) };

        // 逻辑尺寸 → 物理尺寸。CreateWindowExW 收物理像素，egui 收逻辑点。
        let ppp = minidock_platform::display::system_dpi() as f32 / 96.0;
        let win_w = (width * ppp).round() as i32;
        let win_h = (height * ppp).round() as i32;
        let (sw, _sh) = primary_work_area();
        let x = sw - win_w - 80;
        let y = 80;

        let hwnd = unsafe {
            CreateWindowExW(
                // WS_EX_LAYERED 是 UpdateLayeredWindow 的前提。
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                class_pcwstr(),
                w!("MiniDock Spike"),
                WS_POPUP,
                x,
                y,
                win_w,
                win_h,
                None,
                None,
                Some(windows::Win32::Foundation::HINSTANCE(hinstance.0)),
                None,
            )
        }
        .map_err(|_| GpuError::Win32("CreateWindowExW"))?;

        // 适配器在建设备时定下来，之后改配置要重启才生效（ADR-012）。
        let gpu = Gpu::new(&cfg.adapter)?;
        // 把真值回填给界面：配置与实际上机结果可能不同（名字对不上会回退），
        // 只显示配置值等于骗用户。
        settings.active_adapter = gpu.info.name.clone();
        if gpu.selection.fell_back() {
            settings.adapter_note = gpu.selection.describe();
        }
        let frame = FrameRenderer::new(&gpu, win_w as u32, win_h as u32, ppp)?;

        let mut client = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut client);
        };
        eprintln!(
            "GPU: {:?} / {} / {:?} / 选择={} / 呈现=离屏+UpdateLayeredWindow / 格式={:?}",
            gpu.info.backend,
            gpu.info.name,
            gpu.info.device_type,
            gpu.selection.describe(),
            Gpu::COLOR_FORMAT
        );
        eprintln!(
            "窗口: 逻辑 {}x{} / dpi 系数 {:.2} / 表面 {}x{} / 客户区 {}x{}",
            width,
            height,
            ppp,
            win_w,
            win_h,
            client.right - client.left,
            client.bottom - client.top
        );

        Ok(Self {
            hwnd,
            pos: (x, y),
            gpu,
            frame,
            input: Input::default(),
            layout: Layout { width, height },
            pixels_per_point: ppp,
            cfg,
            settings,
            commands: Vec::new(),
            needs_frame: true,
            cursor: egui::CursorIcon::Default,
            stats: Stats::default(),
            last_stats_push: Instant::now(),
            next_repaint_at: None,
            last_trace: Instant::now(),
            last_trace_frames: 0,
        })
    }

    pub fn run(self) -> Result<(), GpuError> {
        let ptr = Box::into_raw(Box::new(self));
        unsafe { SetWindowLongPtrW(ptr.as_ref().unwrap().hwnd, GWLP_USERDATA, ptr as isize) };

        unsafe {
            let overlay = &*ptr;
            let _ = ShowWindow(overlay.hwnd, SW_SHOW);
            let _ = SetForegroundWindow(overlay.hwnd);
            let _ = UpdateWindow(overlay.hwnd);
        }
        unsafe { (*ptr).render()? };

        // 循环结构：抽干消息 → 需要就画 → 睡到"有消息"或"到下一帧"。
        //
        // 之前的写法是 `if has_requested_repaint { render; sleep(8ms) }`：
        // 只要 egui 想动画，就以 8 ms 为间隔空转，既不精确又费电，
        // 而且没有利用 egui 给出的 `repaint_delay`（ADR-014）。
        let mut msg = MSG::default();
        // 唯一的退出路径是 WM_QUIT，用带标签的 break 统一收尾，
        // 避免在循环里 `return` 掉 Box 的所有权。
        'pump: loop {
            // ① 把队列里的消息一次处理完：逐条唤醒会造成"一帧一消息"的抖动。
            loop {
                let has = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) };
                if !has.as_bool() {
                    break;
                }
                if msg.message == WM_QUIT {
                    break 'pump;
                }
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }

            // ② 该画就画：输入驱动的（needs_frame）优先，其次看是否到点。
            let overlay = unsafe { &mut *ptr };
            // `None` 表示"没有待办"，不是"立刻要画" —— 写反了就会变成
            // 每来一条消息都重绘一次（实测空闲 CPU 从 0% 涨到 1.5%）。
            let due = overlay
                .next_repaint_at
                .is_some_and(|at| Instant::now() >= at);
            if overlay.needs_frame || due {
                overlay.needs_frame = false;
                overlay.render()?;
            }

            // ③ 阻塞等待：消息来了立刻醒，否则睡到下一帧截止时刻。
            //    `None` 用 INFINITE —— 完全空闲时线程真睡着，CPU 为 0。
            let timeout = match unsafe { (*ptr).next_repaint_at } {
                Some(at) => at
                    .saturating_duration_since(Instant::now())
                    .as_millis()
                    .min(u128::from(INFINITE - 1)) as u32,
                None => INFINITE,
            };
            unsafe {
                MsgWaitForMultipleObjectsEx(None, timeout, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
            }
        }

        drop(unsafe { Box::from_raw(ptr) });
        Ok(())
    }

    fn render(&mut self) -> Result<(), GpuError> {
        let t0 = Instant::now();
        let raw = self.input.take(self.frame.size(), self.pixels_per_point);

        let mut outcome = Outcome::default();
        let frame_out = self
            .frame
            .render(&self.gpu, raw, self.hwnd, self.pos, |ui| {
                let rect = ui.max_rect();
                let dark = ui.visuals().dark_mode;
                // 设置窗口不需要"透出桌面"的观感，只有圆角外侧留透明；
                // 250/255 足够看出边缘融合，又不至于让壁纸干扰阅读。
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
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                outcome = settings_ui(&mut child, &mut self.cfg, &mut self.settings);
            })?;

        if let Some(cmd) = outcome.command {
            self.commands.push(cmd.clone());
            if cmd == Command::Close {
                unsafe { PostQuitMessage(0) };
            }
        }
        if outcome.changed {
            self.settings.status = "已修改（M1 接入自动保存）".to_owned();
        }
        self.cursor = frame_out.cursor;
        // egui 自己知道下一次该在什么时候画：动画会给一个非零延时，
        // "越快越好"给 0，完全不需重绘给 `Duration::MAX`。
        // 用它来驱动消息循环，比固定 `sleep(8ms)` 空转省电得多（ADR-014）。
        self.next_repaint_at = if frame_out.repaint_delay == Duration::MAX {
            // 没有待办：阻塞到下一个窗口消息为止。
            None
        } else if frame_out.repaint_delay.is_zero() {
            // 0 表示"尽快"。Dock 的动画按 60 fps 排就够，不必贴着渲染速度空转。
            Some(Instant::now() + FRAME_PACING)
        } else {
            Some(Instant::now() + frame_out.repaint_delay)
        };

        // 可观测性：排查"空闲还在重绘"这类问题时，`MINIDOCK_TRACE=1` 会
        // 每秒打印一次 egui 给的理由。默认关闭，不影响正常运行。
        if std::env::var_os("MINIDOCK_TRACE").is_some()
            && self.last_trace.elapsed() >= Duration::from_secs(1)
        {
            let ctx = self.frame.ctx();
            let frames = self.stats.samples;
            let per_sec = frames.saturating_sub(self.last_trace_frames);
            self.last_trace_frames = frames;
            self.last_trace = Instant::now();
            let hover = ctx.input(|i| i.pointer.hover_pos());
            eprintln!(
                "[trace] {frames} 帧 (+{per_sec}/s) hover={hover:?} delay={:?} 原因 {:?}",
                frame_out.repaint_delay,
                ctx.repaint_causes()
            );
        }

        self.update_stats(t0);
        apply_cursor(self.cursor);
        Ok(())
    }

    fn update_stats(&mut self, started: Instant) {
        let dt = started.elapsed().as_secs_f32() * 1000.0;
        self.stats.frame_ms = if self.stats.frame_ms <= 0.0 {
            dt
        } else {
            // 指数平滑：单帧毛刺不该把读数打成异常值。
            self.stats.frame_ms * 0.9 + dt * 0.1
        };
        self.stats.samples += 1;
        if self.stats.samples.is_multiple_of(30) {
            self.stats.rss_mb = MemStats::now().ws_mb();
        }
        if self.last_stats_push.elapsed() >= Duration::from_millis(400) {
            self.last_stats_push = Instant::now();
            self.settings.stats = format!(
                "{:.2} ms/帧 · {:.0} fps · 常驻 {:.1} MB · 离屏+分层窗口",
                self.stats.frame_ms,
                self.stats.fps(),
                self.stats.rss_mb
            );
        }
    }

    fn on_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_MOUSEMOVE => {
                let (x, y) = client_pos(lparam, self.pixels_per_point);
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
                None
            }
            WM_MOUSELEAVE => {
                self.input.pointer_gone();
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
                None
            }
            WM_MOUSEWHEEL => {
                let delta = ((wparam.0 >> 16) as u16 as i16) as f32 / 120.0;
                self.input.wheel(delta);
                None
            }
            WM_KEYDOWN | WM_KEYUP => {
                self.input.set_modifiers(current_modifiers());
                let vk = wparam.0 as u32;
                if let Some(key) = map_vk(vk) {
                    self.input.key(key, msg == WM_KEYDOWN);
                    if key == egui::Key::Escape && msg == WM_KEYDOWN {
                        unsafe { PostQuitMessage(0) };
                    }
                }
                None
            }
            WM_SETCURSOR => {
                // 返回 TRUE：告诉系统"光标我自己管"，否则会被类光标覆盖。
                apply_cursor(self.cursor);
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
            WM_DESTROY => {
                unsafe { PostQuitMessage(0) };
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Overlay;
    if ptr.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    // SAFETY: 指针由 `Overlay::run` 用 Box::into_raw 产生，消息循环结束后才回收；
    // 窗口过程只在 UI 线程调用。
    let overlay = unsafe { &mut *ptr };
    if let Some(result) = overlay.on_message(msg, wparam, lparam) {
        return result;
    }
    overlay.needs_frame = true;
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
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

/// egui 的光标意图 → 系统光标。
fn apply_cursor(icon: egui::CursorIcon) {
    let id = match icon {
        egui::CursorIcon::PointingHand => IDC_HAND,
        egui::CursorIcon::Text => IDC_IBEAM,
        _ => IDC_ARROW,
    };
    if let Ok(handle) = unsafe { LoadCursorW(None, id) } {
        unsafe { SetCursor(Some(handle)) };
    }
}

/// 声明 per-monitor v2 DPI 感知。失败无害（老系统返回错误，退回不感知）。
fn dpi_awareness() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    };
}

fn primary_work_area() -> (i32, i32) {
    let mut rc = RECT::default();
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::SystemParametersInfoW(
            windows::Win32::UI::WindowsAndMessaging::SPI_GETWORKAREA,
            0,
            Some((&mut rc as *mut RECT).cast()),
            windows::Win32::UI::WindowsAndMessaging::SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    (rc.right, rc.bottom)
}
