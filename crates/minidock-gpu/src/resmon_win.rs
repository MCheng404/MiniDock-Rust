//! 资源监视器详情面板：点条带上的「CPU xx%」卡片弹出来。
//!
//! 对应上游 v1.6.0 的 `ShowResMon()` / `ResDraw()`（minidock.c:6533-6590）。
//! 上游画三条进度条：处理器 / 内存 / 磁盘（C:）。
//!
//! 与设置窗口（ADR-036）同一套做法：进程内第二个分层窗口，共用 GPU 设备，
//! 自己的 `FrameRenderer`。但这个面板是**即时刷新**的 —— 数据每秒变一次，
//! 所以它自己按 1 秒排帧，不依赖 egui 的 repaint 提示。

use std::pin::Pin;
use std::time::{Duration, Instant};

use egui::{Align2, Color32, CornerRadius, FontId, Rect as EguiRect, Stroke};
use minidock_platform::{CpuSampler, SysInfo};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetWindowLongPtrW, RegisterClassW, SetWindowLongPtrW,
    ShowWindow, GWLP_USERDATA, SW_HIDE, SW_SHOW, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_RBUTTONDOWN, WM_RBUTTONUP, WNDCLASSW, WS_EX_APPWINDOW,
    WS_EX_LAYERED, WS_EX_TOPMOST, WS_POPUP,
};

use crate::frame::FrameRenderer;
use crate::gpu::{Gpu, GpuError};

/// 窗口类名。
pub const CLASS_NAME: &str = "MiniDockResMon";

/// 面板逻辑尺寸（本地坐标）。上游是 340 宽，这里放宽一点让进度条有地方画。
const LOGICAL_SIZE: (f32, f32) = (340.0, 210.0);

/// 资源监视器详情面板。
pub struct ResMonWindow {
    hwnd: HWND,
    renderer: FrameRenderer,
    /// 输入。面板是只读的，但**仍然需要它**：`Input::take` 会带上
    /// `screen_rect`，没有它 egui 的绘制区只有一小块，
    /// 背景被裁、右侧文字落在裁剪区外（实测过一次）。
    input: crate::input::Input,
    /// 屏幕左上角的物理坐标。
    pos: (i32, i32),
    /// 每逻辑点对应多少物理像素。留着是为了后续按 DPI 调整面板尺寸。
    #[allow(dead_code)]
    ppp: f32,
    /// CPU 采样器（占用率要靠两次采样的差值）。
    cpu: CpuSampler,
    /// 最近一次采样。
    info: SysInfo,
    /// 下一次刷新的时刻。
    next_refresh: Instant,
    visible: bool,
    /// 深色主题。
    dark: bool,
}

impl std::fmt::Debug for ResMonWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResMonWindow")
            .field("hwnd", &(self.hwnd.0 as isize))
            .field("visible", &self.visible)
            .finish_non_exhaustive()
    }
}

impl ResMonWindow {
    /// 建窗（默认隐藏）。
    pub fn new(gpu: &Gpu, ppp: f32, dark: bool) -> Result<Pin<Box<Self>>, GpuError> {
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
                WS_EX_LAYERED | WS_EX_APPWINDOW | WS_EX_TOPMOST,
                class_pcwstr(),
                windows::core::w!("资源监视器"),
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
        .map_err(|_| GpuError::Win32("CreateWindowExW（资源监视器）"))?;

        let renderer = FrameRenderer::new(gpu, pw as u32, ph as u32, ppp)?;
        let mut win = Box::pin(Self {
            hwnd,
            renderer,
            input: crate::input::Input::default(),
            pos: (x, y),
            ppp,
            cpu: CpuSampler::new(),
            info: SysInfo::default(),
            next_refresh: Instant::now(),
            visible: false,
            dark,
        });
        win.attach();
        Ok(win)
    }

    /// 把自身指针挂到 `GWLP_USERDATA`。
    ///
    /// 公开是因为主循环要能单独调（设置窗口那边是 `new` 内部自调，
    /// 这里为了统一"懒建 + 挂载"的顺序，交给调用方）。
    pub fn attach_public(&mut self) {
        self.attach();
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

    /// 显示面板并刷新一次数据。
    pub fn show(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
        }
        self.visible = true;
        self.refresh();
        self.next_refresh = Instant::now() + Duration::from_secs(1);
    }

    pub fn hide(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.visible = false;
    }

    /// 是否到了该刷新的时刻。
    #[must_use]
    pub fn due(&self) -> bool {
        self.visible && self.next_refresh <= Instant::now()
    }

    /// 重新采样一次。
    pub fn refresh(&mut self) {
        self.info = minidock_platform::sysinfo::sample(&mut self.cpu);
    }

    /// 画一帧。
    pub fn render(&mut self, gpu: &Gpu) -> Result<(), GpuError> {
        self.refresh();
        self.next_refresh = Instant::now() + Duration::from_secs(1);

        let pos = self.pos;
        let hwnd = self.hwnd;
        let dark = self.dark;
        let info = self.info;
        let raw = self.input.take(self.renderer.size(), self.ppp);
        self.renderer.render(gpu, raw, hwnd, pos, |ui| {
            draw_panel(ui, dark, info);
        })?;
        Ok(())
    }

    fn on_message(&mut self, msg: u32, _wparam: WPARAM, _lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONUP | WM_MOUSEMOVE => None,
            WM_KEYDOWN => None,
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

/// 画面板：标题 + 三条进度条（处理器 / 内存 / 磁盘）。
///
/// 对应上游的 `ResDraw()`。上游是显式定位（`DrawTextBox` 传矩形），
/// 这里也用 painter 显式画 —— 用 egui 的布局做左右对齐时，
/// 右边的数值会被挤没（实测过：`with_layout(right_to_left)` 在
/// `horizontal` 里拿不到剩余宽度）。
fn draw_panel(ui: &mut egui::Ui, dark: bool, info: SysInfo) {
    let rect = ui.max_rect();
    let bg = if dark {
        Color32::from_rgba_unmultiplied(24, 25, 30, 250)
    } else {
        Color32::from_rgba_unmultiplied(246, 247, 250, 250)
    };
    let painter = ui.painter();
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

    let pad = 16.0;
    let mut y = rect.top() + pad;

    // 标题
    painter.text(
        egui::pos2(rect.left() + pad, y + 10.0),
        Align2::LEFT_TOP,
        "资源监视器",
        FontId::proportional(18.0),
        tc,
    );
    y += 30.0;
    painter.line_segment(
        [
            egui::pos2(rect.left() + pad, y),
            egui::pos2(rect.right() - pad, y),
        ],
        Stroke::new(1.0, sub),
    );
    y += 14.0;

    let rows = [
        ("处理器", info.cpu_percent, format!("{}%", info.cpu_percent)),
        (
            "内存",
            info.mem_percent,
            format!("{} / {} MB", info.mem_used_mb, info.mem_total_mb),
        ),
        (
            "磁盘（C:）",
            info.disk_percent,
            format!("可用 {} / {} GB", info.disk_free_gb, info.disk_total_gb),
        ),
    ];

    for (label, pct, detail) in rows {
        // 左边标签，右边数值（显式定位，不用布局）。
        painter.text(
            egui::pos2(rect.left() + pad, y + 8.0),
            Align2::LEFT_TOP,
            label,
            FontId::proportional(13.0),
            tc,
        );
        painter.text(
            egui::pos2(rect.right() - pad, y + 8.0),
            Align2::RIGHT_TOP,
            detail,
            FontId::proportional(12.0),
            sub,
        );
        y += 22.0;
        // 进度条
        let bar_h = 6.0;
        let track = EguiRect::from_min_size(
            egui::pos2(rect.left() + pad, y),
            egui::vec2(rect.width() - 2.0 * pad, bar_h),
        );
        painter.rect_filled(track, CornerRadius::same(3), sub);
        let pct = f32::from(pct.min(100));
        let filled =
            EguiRect::from_min_size(track.min, egui::vec2(track.width() * pct / 100.0, bar_h));
        if filled.width() > 1.0 {
            painter.rect_filled(filled, CornerRadius::same(3), accent);
        }
        y += bar_h + 18.0;
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut ResMonWindow;
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

impl Drop for ResMonWindow {
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
        (LOGICAL_SIZE.0 * ppp).round() as i32,
        (LOGICAL_SIZE.1 * ppp).round() as i32,
    )
}

/// 面板居中于工作区。
fn centered_origin(pw: i32, ph: i32) -> (i32, i32) {
    let work = minidock_platform::display::work_area(0);
    let cx = (work.left + work.right) / 2;
    let cy = (work.top + work.bottom) / 2;
    (cx - pw / 2, cy - ph / 2)
}

/// 供调用方判断这个窗口是不是资源监视器面板。
#[must_use]
pub fn is_resmon_window(hwnd: HWND) -> bool {
    let mut buf = [0_u16; 64];
    let n = unsafe { windows::Win32::UI::WindowsAndMessaging::GetClassNameW(hwnd, &mut buf) };
    n > 0 && String::from_utf16_lossy(&buf[..n as usize]) == CLASS_NAME
}

/// 面板上要画的文字（测试用）。
#[must_use]
pub fn summary(info: &SysInfo) -> String {
    format!(
        "CPU {}%  内存 {}%  磁盘 {}%",
        info.cpu_percent, info.mem_percent, info.disk_percent
    )
}
