//! M1 的 Dock 窗口：真实 Dock 形态（不是设置窗）。
//!
//! 复用 M0 的呈现管线（离屏渲染 → 回读 → `UpdateLayeredWindow`），
//! 把画面换成 [`minidock_gpu::dock::Dock`] 的绘制。
//!
//! 用法：
//!
//! ```text
//! cargo run --release -p minidock-gpu --bin minidock-dock
//! ```
//!
//! 按 Esc 退出。鼠标移上去会触发悬停放大与标签气泡。

use std::time::Instant;

use minidock_core::config::Config;
use minidock_core::ini::Ini;
use minidock_core::layout::Rect as LRect;
use minidock_core::store;
use minidock_gpu::dock::{Dock, DockItem};
use minidock_gpu::gpu::Gpu;
use minidock_gpu::offscreen::Offscreen;
use minidock_gpu::present::LayeredPresenter;
use minidock_platform::display;
use minidock_platform::Paths;
use windows::core::w;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, UpdateWindow, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::INFINITE;
use windows::Win32::UI::HiDpi::{
    GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetWindowLongPtrW,
    MsgWaitForMultipleObjectsEx, PeekMessageW, RegisterClassW, SetForegroundWindow, ShowWindow,
    TranslateMessage, GWLP_USERDATA, MWMO_INPUTAVAILABLE, PM_REMOVE, QS_ALLINPUT, SW_SHOW,
    WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT,
    WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP, WNDCLASSW, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 循环只在 WM_QUIT 时返回。
    // DPI 感知必须在建窗之前（M0 的教训）。
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let paths = Paths::resolve()?;
    let dpi = display::system_dpi();
    let ini = Ini::load(&paths.ini)?;
    let cfg = Config::from_ini(&ini, dpi);
    let items: Vec<DockItem> = store::load_items(&ini)
        .iter()
        .map(DockItem::from_item)
        .collect();

    // 工作区（物理像素）：C 版用 `g_mi.rcWork`。
    let work: LRect = display::work_area(0);
    let mut dock = Box::new(Dock::new(cfg, items, work));
    dock.update(dpi);

    let (lw, lh) = dock.logical_size();
    let ppp = dpi as f32 / 96.0;
    let win_w = lw.round() as i32;
    let win_h = lh.round() as i32;

    let hinstance = unsafe { GetModuleHandleW(None) }?;
    let wc = WNDCLASSW {
        hInstance: HINSTANCE(hinstance.0),
        lpfnWndProc: Some(wndproc),
        lpszClassName: w!("MiniDockWindow"),
        ..Default::default()
    };
    unsafe {
        RegisterClassW(&wc);
    }

    // 位置：工作区底部居中（与 C 版一致）。
    let x = work.left + (work.width() - win_w) / 2;
    let y = work.bottom - win_h;
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            w!("MiniDockWindow"),
            w!("MiniDock"),
            WS_POPUP,
            x,
            y,
            win_w,
            win_h,
            None,
            None,
            Some(hinstance.into()),
            None,
        )?
    };

    let gpu = Gpu::new("")?;
    let offscreen = Offscreen::new(&gpu.device, win_w as u32, win_h as u32, Gpu::COLOR_FORMAT);
    let mut presenter = LayeredPresenter::new(win_w, win_h)?;
    let mut renderer = egui_wgpu::Renderer::new(
        &gpu.device,
        Gpu::COLOR_FORMAT,
        egui_wgpu::RendererOptions {
            msaa_samples: 1,
            depth_stencil_format: None,
            dithering: false,
            ..Default::default()
        },
    );
    let mut ctx = egui::Context::default();
    if !minidock_settings::fonts::install_cjk(&ctx) {
        eprintln!("警告：未找到中文字体");
    }
    let last = Instant::now();

    // Dock 指针挂到窗口上，供 wndproc 回传鼠标位置。
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(
            hwnd,
            GWLP_USERDATA,
            (&raw mut *dock) as isize,
        );
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = UpdateWindow(hwnd);
    }

    let mut msg = windows::Win32::UI::WindowsAndMessaging::MSG::default();
    loop {
        // ① 抽干消息队列：一条消息一帧会造成抖动。
        loop {
            let has = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) };
            if !has.as_bool() {
                break;
            }
            if msg.message == WM_QUIT {
                return Ok(());
            }
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        // ② 推进动画。**必须主动推进**：消息驱动的循环在没消息时不会醒，
        //    放大曲线会卡在半路（第一版就因为用了阻塞的 GetMessage 而出错）。
        if dock.anim {
            dock.update(dpi);
        }

        // ③ 出帧。

        render(
            &dock,
            &mut ctx,
            &mut renderer,
            &gpu,
            &offscreen,
            &mut presenter,
            hwnd,
            (x, y),
            ppp,
            last,
        )?;

        // ④ 等待：有消息就醒；动画中按 16 ms 醒一次；完全空闲时真睡着。
        let timeout = if dock.anim { 16 } else { INFINITE };
        unsafe {
            MsgWaitForMultipleObjectsEx(None, timeout, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
        }
    }
}

/// 画一帧：egui 画 Dock → 离屏 → 回读 → 分层窗口。
#[allow(clippy::too_many_arguments)]
fn render(
    dock: &Dock,
    ctx: &mut egui::Context,
    renderer: &mut egui_wgpu::Renderer,
    gpu: &Gpu,
    offscreen: &Offscreen,
    presenter: &mut LayeredPresenter,
    hwnd: HWND,
    pos: (i32, i32),
    ppp: f32,
    start: Instant,
) -> Result<(), Box<dyn std::error::Error>> {
    let dark = dock.cfg.ui_is_dark(true);
    let (time, date) = clock_text();
    let clock = if dock.cfg.clock {
        Some((time, date))
    } else {
        None
    };

    // `screen_rect` 必须给对：egui 用它决定根 Ui 的尺寸。
    // 不给的话画布是默认大小，栏体就会被画在错误的位置（M1 第一版就踩了）。
    let mut raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(
                offscreen.width() as f32 / ppp,
                offscreen.height() as f32 / ppp,
            ),
        )),
        time: Some(start.elapsed().as_secs_f64()),
        ..Default::default()
    };
    // egui 0.37 通过 `ViewportInfo::native_pixels_per_point` 接收原生缩放。
    // 不设的话 egui 按 1.0 缩放：画面只占表面的 1/ppp（偏左上、尺寸偏小）。
    raw.viewports.insert(
        egui::ViewportId::ROOT,
        egui::ViewportInfo {
            native_pixels_per_point: Some(ppp),
            ..Default::default()
        },
    );
    let full = ctx.run_ui(raw, |ui| {
        dock.paint(
            ui,
            ppp,
            dark,
            clock.as_ref().map(|(t, d)| (t.as_str(), d.as_str())),
        );
    });

    let egui::FullOutput {
        shapes,
        pixels_per_point,
        mut textures_delta,
        ..
    } = full;
    let jobs = ctx.tessellate(shapes, pixels_per_point);
    for (id, deltas) in &textures_delta.set {
        for delta in deltas {
            renderer.update_texture(&gpu.device, &gpu.queue, *id, delta);
        }
    }
    let screen = egui_wgpu::ScreenDescriptor {
        size_in_pixels: [offscreen.width(), offscreen.height()],
        pixels_per_point,
    };
    let mut encoder =
        gpu.device
            .create_command_encoder(&egui_wgpu::wgpu::CommandEncoderDescriptor {
                label: Some("dock-frame"),
            });
    let buffers = renderer.update_buffers(&gpu.device, &gpu.queue, &mut encoder, &jobs, &screen);
    {
        let mut pass = encoder
            .begin_render_pass(&egui_wgpu::wgpu::RenderPassDescriptor {
                label: Some("dock"),
                color_attachments: &[Some(egui_wgpu::wgpu::RenderPassColorAttachment {
                    view: &offscreen.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: egui_wgpu::wgpu::Operations {
                        load: egui_wgpu::wgpu::LoadOp::Clear(egui_wgpu::wgpu::Color::TRANSPARENT),
                        store: egui_wgpu::wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        renderer.render(&mut pass, &jobs, &screen);
    }
    offscreen.copy_to_buffer(&mut encoder);
    gpu.queue
        .submit(buffers.into_iter().chain([encoder.finish()]));

    let pixels = offscreen.read_pixels(&gpu.device)?;
    presenter.upload_rgba(&pixels);
    presenter.present(hwnd, pos.0, pos.1)?;

    for id in &textures_delta.free {
        renderer.free_texture(id);
    }
    textures_delta.clear();
    Ok(())
}

/// 当前时间与日期文本（上游格式：`12:34` 与 `10月22日 周三`）。
fn clock_text() -> (String, String) {
    unsafe {
        let st = windows::Win32::System::SystemInformation::GetLocalTime();
        let week = ["日", "一", "二", "三", "四", "五", "六"]
            .get(st.wDayOfWeek as usize)
            .copied()
            .unwrap_or("日");
        (
            format!("{:02}:{:02}", st.wHour, st.wMinute),
            format!("{}月{}日 周{}", st.wMonth, st.wDay, week),
        )
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let _ = BeginPaint(hwnd, &mut ps);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_MOUSEMOVE => {
            // 逻辑坐标：物理 / ppp。
            let x = (lparam.0 & 0xFFFF) as f32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as f32;
            let ppp = {
                let dpi = GetDpiForWindow(hwnd);
                if dpi == 0 {
                    1.0
                } else {
                    dpi as f32 / 96.0
                }
            };
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Dock;
            if !ptr.is_null() {
                unsafe {
                    (*ptr).mouse = (x / ppp, y / ppp);
                    (*ptr).hover = true;
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONUP => LRESULT(0),
        WM_KEYDOWN => {
            if wparam.0 as u32 == 0x1B {
                unsafe {
                    windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0);
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe {
                windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
