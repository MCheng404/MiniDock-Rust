//! MiniDock 可执行文件入口。
//!
//! 职责只有四件事，刻意保持极薄：
//! 1. 单实例判定（已有实例就把它的窗口唤起，然后退出）
//! 2. 解析路径 / 读 ini / 得到 `Config` + `Vec<Item>`
//! 3. 启动 GDI+ 运行时
//! 4. 交给 `Dock::run`，把错误显示给用户
//!
//! 注意 `GdiPlus` 必须**先于** `Dock` 声明：Rust 按声明逆序析构，
//! 这样 Dock 里的 Font/Brush 会先释放，再 `GdiplusShutdown`。
//! 反过来写会在退出时崩在 `GdipDeleteFont` 里。

// release 下不要控制台窗口；debug 保留，方便 eprintln! 与调试器。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use minidock_core::config::Config;
use minidock_core::ini::Ini;
use minidock_core::store;
use minidock_gfx::GdiPlus;
use minidock_platform::single_instance::SingleInstance;
use minidock_platform::{display, Paths};
use minidock_ui::dock::{find_existing, ping_existing, Dock};

use windows::core::PCWSTR;
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

fn main() {
    if let Err(msg) = run() {
        fatal(&msg);
    }
}

fn run() -> Result<(), String> {
    // 单实例：`Ok(None)` 表示已有一份在跑。
    match SingleInstance::acquire() {
        Ok(Some(_guard)) => {}
        Ok(None) => {
            if let Some(hwnd) = find_existing() {
                ping_existing(hwnd);
            }
            return Ok(());
        }
        Err(e) => {
            // 互斥体失败不应该阻止启动：宁可两份 Dock，也不要起不来。
            eprintln!("MiniDock: {e}（继续启动）");
        }
    }

    let paths = Paths::resolve().map_err(|e| format!("初始化配置目录失败：{e}"))?;
    let dpi = display::system_dpi();

    let ini =
        Ini::load(&paths.ini).map_err(|e| format!("读取 {} 失败：{e}", paths.ini.display()))?;
    let cfg = Config::from_ini(&ini, dpi);
    let items = store::load_items(&ini);
    eprintln!(
        "MiniDock: {} 个条目，{}x{} 逻辑尺寸，DPI {}",
        items.len(),
        0,
        0,
        dpi
    );

    let _gp = GdiPlus::startup().map_err(|e| format!("GDI+ 初始化失败：{e}"))?;
    let dock = Dock::new(paths, cfg, items, dpi).map_err(|e| format!("创建 Dock 失败：{e}"))?;
    dock.run().map_err(|e| format!("Dock 运行失败：{e}"))
}

/// 用系统消息框报错。GUI 程序没有控制台，这是唯一用户可见的失败通道。
fn fatal(msg: &str) {
    let text: Vec<u16> = msg.encode_utf16().chain(std::iter::once(0)).collect();
    let title: Vec<u16> = "MiniDock"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}
