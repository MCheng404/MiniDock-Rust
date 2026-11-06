//! GPU 呈现层：wgpu 表面 + egui 绘制 + Win32 输入/命中。
//!
//! 这一层**只做窗口与提交**，不含任何业务逻辑：
//! - 画什么 → `minidock-settings`（设置界面）/ `dock`（Dock 本体）
//! - 系统信息 → `minidock-platform`
//! - 配置模型与几何 → `minidock-core`
//!
//! 模块分工：
//! - `frame` —— 一帧的提交路径（egui → 离屏 → 回读 → 分层窗口），
//!   Dock 窗口与设置窗口共用；
//! - `settings_win` —— 设置窗口的建窗、输入、呈现（ADR-036，Dock 进程内）；
//! - `dock` —— Dock 的布局状态机与 egui 绘制；
//! - `gpu` / `offscreen` / `present` —— 设备、离屏目标、DIB 呈现；
//! - `input` —— Win32 消息 → `egui::RawInput`；
//! - `menu` / `settings_win` —— 右键菜单与设置窗口（各自一个分层窗口）；
//! - `worker` —— 后台线程：取图标、扫描运行状态/遮挡、写盘、启动应用。
//!
//! 依赖里**没有 eframe / winit**：消息循环由我们自己持有，因为托盘、
//! 全局热键、AppBar 预留、逐像素命中都需要在 `WndProc` 里说话。
//! 代价是输入翻译（`input.rs`）和 IME（M1）要自己写。

#![deny(missing_debug_implementations)]

pub mod accent;
pub mod shaders;

/// Load the icon embedded by `build.rs` (resource id 1).
/// Returns `None` when rc.exe was unavailable; a missing icon must not block startup.
#[must_use]
#[expect(
    clippy::manual_dangling_ptr,
    reason = "MAKEINTRESOURCE passes integer 1 as a resource id; never dereferenced"
)]
pub fn app_icon() -> Option<windows::Win32::UI::WindowsAndMessaging::HICON> {
    use windows::core::PCWSTR;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::LoadIconW;

    let hinst = unsafe { GetModuleHandleW(None) }.ok()?;
    let icon = unsafe {
        LoadIconW(
            Some(windows::Win32::Foundation::HINSTANCE(hinst.0)),
            PCWSTR(1usize as *const u16), // MAKEINTRESOURCE(1)
        )
    }
    .ok()?;
    (!icon.0.is_null()).then_some(icon)
}

pub mod calendar_win;
pub mod cc_win;
pub mod cursor;
pub mod dock;
pub mod frame;
pub mod frost;
pub mod glyph;
pub mod gpu;
pub mod ime;
pub mod input;
pub mod launchpad_win;
pub mod mem;
pub mod menu;
pub mod offscreen;
pub mod perf;
pub mod popup;
pub mod present;
pub mod resmon_win;
pub mod settings_win;
pub mod smoke;
pub mod tool_win;
pub mod worker;

pub use dock::{Dock, DockItem};
pub use frame::{FrameOutcome, FrameRenderer};
pub use gpu::{Gpu, GpuError, Selection};
pub use mem::MemStats;
pub use menu::{Menu, MenuCmd, MenuEntry};
pub use offscreen::Offscreen;
pub use present::LayeredPresenter;
pub use settings_win::SettingsWindow;
pub use worker::{DiskWriter, IconLoader, Opener, Scanner};
