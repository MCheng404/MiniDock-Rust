//! 系统光标。
//!
//! 这个模块存在的唯一原因是一个很难查的坑（ADR-045）：
//!
//! **窗口类的 `hCursor` 留空、又不在 `WM_SETCURSOR` 里自己设光标时，
//! Windows 不会退回箭头，而是显示"忙"（沙漏 / 转圈）。**
//!
//! 表现是"鼠标一进 UI 就开始转"，而且因为没人把它设回去，只要光标停在
//! 窗口上就一直转 —— 看起来像程序卡死了。实测（同一个窗口类，只改
//! `hCursor`，进入窗口后读 `GetCursorInfo`）：
//!
//! | 类光标 | 进入窗口后 |
//! |---|---|
//! | `NULL` | `IDC_WAIT` |
//! | `IDC_ARROW` | `IDC_ARROW` |
//!
//! 所以两件事都要做，缺一不可：
//!
//! 1. 注册窗口类时给 `hCursor` 一个真光标（[`class_cursor`]）；
//! 2. 窗口过程处理 `WM_SETCURSOR`，每次都重设（[`apply`] / [`set`]）。
//!
//! 第 1 条管"刚进入窗口"，第 2 条管"进入之后每次移动、点击、Alt+Tab 回来"。
//!
//! **还有第三条，而且是最容易漏的**：`SetCapture` 期间系统**根本不发
//! `WM_SETCURSOR`**。实测（同一个窗口类，`hCursor=IDC_ARROW`）：
//!
//! | 是否持有捕获 | 收到 WM_SETCURSOR | 光标 |
//! |---|---|---|
//! | 否 | 1 次 | `IDC_ARROW` |
//! | 是 | **0 次** | `IDC_WAIT` |
//!
//! 于是"按住拖动 / 弹出模态菜单 / 弹层对话框"这些**一开就 SetCapture**
//! 的窗口，一露头光标就是忙的，直到它关掉都没变过。所以持有捕获的窗口
//! 必须在 `WM_MOUSEMOVE` 里自己设光标（见 [`set_hand_if`]），不能只靠
//! `WM_SETCURSOR`。

use std::sync::atomic::{AtomicUsize, Ordering};

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, LoadCursorW, SetCursor, HCURSOR, IDC_APPSTARTING, IDC_ARROW, IDC_HAND, IDC_IBEAM,
    IDC_SIZEALL, IDC_SIZENS, IDC_WAIT,
};

/// 系统光标的缓存。
///
/// 这些函数会在 `WM_MOUSEMOVE` 里被调用（捕获期间必须每次鼠标移动都设一遍），
/// 一秒钟几十上百次。`LoadCursorW` 本身不贵，但没必要每次都问系统。
///
/// 句柄是进程内的共享资源、永不释放，所以缓存成 `usize` 是安全的；
/// 用原子量而不是 `OnceLock` 是因为 `HCURSOR` 不实现 `Sync`。
static ARROW_HANDLE: AtomicUsize = AtomicUsize::new(0);
static HAND_HANDLE: AtomicUsize = AtomicUsize::new(0);
static IBEAM_HANDLE: AtomicUsize = AtomicUsize::new(0);
static SIZEALL_HANDLE: AtomicUsize = AtomicUsize::new(0);
static SIZENS_HANDLE: AtomicUsize = AtomicUsize::new(0);
static APPSTARTING_HANDLE: AtomicUsize = AtomicUsize::new(0);
static WAIT_HANDLE: AtomicUsize = AtomicUsize::new(0);

/// 取一个系统光标并缓存。取不到时返回空句柄（调用方不该依赖它非空）。
fn cached(slot: &AtomicUsize, id: windows::core::PCWSTR) -> HCURSOR {
    let hit = slot.load(Ordering::Relaxed);
    if hit != 0 {
        return HCURSOR(hit as *mut std::ffi::c_void);
    }
    let loaded = unsafe { LoadCursorW(None, id) }.unwrap_or_default();
    if !loaded.is_invalid() {
        slot.store(loaded.0 as usize, Ordering::Relaxed);
    }
    loaded
}

fn load(id: windows::core::PCWSTR) -> HCURSOR {
    match id {
        IDC_ARROW => cached(&ARROW_HANDLE, IDC_ARROW),
        IDC_HAND => cached(&HAND_HANDLE, IDC_HAND),
        IDC_IBEAM => cached(&IBEAM_HANDLE, IDC_IBEAM),
        IDC_SIZEALL => cached(&SIZEALL_HANDLE, IDC_SIZEALL),
        IDC_SIZENS => cached(&SIZENS_HANDLE, IDC_SIZENS),
        IDC_APPSTARTING => cached(&APPSTARTING_HANDLE, IDC_APPSTARTING),
        IDC_WAIT => cached(&WAIT_HANDLE, IDC_WAIT),
        other => unsafe { LoadCursorW(None, other) }.unwrap_or_default(),
    }
}

/// 注册窗口类时填给 `WNDCLASSW::hCursor`。
///
/// **不要留空**：留空就是本模块开头那个 bug。
pub fn class_cursor() -> HCURSOR {
    load(IDC_ARROW)
}

/// 箭头。
pub fn arrow() -> HCURSOR {
    load(IDC_ARROW)
}

/// 手型（可点的东西）。
pub fn hand() -> HCURSOR {
    load(IDC_HAND)
}

/// 文本插入符。
pub fn i_beam() -> HCURSOR {
    load(IDC_IBEAM)
}

/// 上下双箭头：拖分割线改 Dock 尺寸时用。
pub fn size_ns() -> HCURSOR {
    load(IDC_SIZENS)
}

/// 把光标设成指定形状。空句柄时什么都不做（避免把光标"设没了"）。
pub fn set(cursor: HCURSOR) {
    if !cursor.is_invalid() {
        unsafe {
            SetCursor(Some(cursor));
        }
    }
}

/// 光标在 `hwnd` 客户区里的物理坐标。
///
/// `WM_SETCURSOR` 的 `lparam` **不是**鼠标坐标（低字是命中测试码），
/// 想知道鼠标在哪只能自己取一次。也不要拿上一次 `WM_MOUSEMOVE` 存下的
/// 位置：系统是先发 `WM_SETCURSOR` 再发 `WM_MOUSEMOVE`，用存下的值会慢一格。
pub fn client_pos(hwnd: HWND) -> Option<(f32, f32)> {
    let mut p = POINT::default();
    if unsafe { GetCursorPos(&raw mut p) }.is_err() {
        return None;
    }
    // `ScreenToClient` 的返回类型是 `Result<Infallible, _>` —— 它不会失败，
    // 只有在 hwnd 无效时才可能出问题，那时保持屏幕坐标也无所谓。
    let _ = unsafe { ScreenToClient(hwnd, &raw mut p) };
    Some((p.x as f32, p.y as f32))
}

/// 按"是否可点"设光标：可点给手型，否则给箭头。
///
/// 给不跟踪 egui 光标形状的窗口用（Dock 主窗口、右键菜单）。
pub fn set_hand_if(hand_cursor: bool) {
    set(if hand_cursor { hand() } else { arrow() });
}

/// 把 egui 请求的光标形状映射成系统光标（纯函数，可单测）。
pub fn from_icon(icon: egui::CursorIcon) -> HCURSOR {
    match icon {
        egui::CursorIcon::PointingHand => hand(),
        egui::CursorIcon::Text | egui::CursorIcon::VerticalText => i_beam(),
        egui::CursorIcon::Move | egui::CursorIcon::Grabbing | egui::CursorIcon::AllScroll => {
            load(IDC_SIZEALL)
        }
        // egui 里 `Progress` / `Wait` 是"真的在处理"，与系统语义一致；
        // 直接用系统那两只，别自己画。
        egui::CursorIcon::Progress => load(IDC_APPSTARTING),
        egui::CursorIcon::Wait => load(IDC_WAIT),
        _ => arrow(),
    }
}

/// 把 egui 请求的光标形状设上。
///
/// egui 每帧会给出 `platform_output.cursor_icon`，用它比"按 hover 猜"
/// 准得多：文本框会自己变成 I 型光标，拖拽时会变成四向箭头。
pub fn apply(icon: egui::CursorIcon) {
    set(from_icon(icon));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 三种常用光标都必须拿得到句柄。
    ///
    /// 这条测试防的是"`LoadCursorW` 参数写错"——传错 id 会返回 Err，
    /// 而 `unwrap_or_default()` 会把它变成空句柄，然后 `WM_SETCURSOR`
    /// 什么都没设，光标又回到系统默认的忙状态。
    #[test]
    fn system_cursors_are_available() {
        for (name, h) in [
            ("arrow", arrow()),
            ("hand", hand()),
            ("i_beam", i_beam()),
            ("class", class_cursor()),
        ] {
            assert!(!h.is_invalid(), "{name} 光标取不到");
        }
    }

    /// `from_icon` 不能把任何形状映射成空句柄 —— 空句柄会让光标停在
    /// 上一个状态（正是本 bug 的成因）。
    #[test]
    fn apply_covers_egui_icons() {
        // egui 的光标枚举不算大，全部过一遍。以后 egui 加新形状时这里
        // 只会漏掉新值，不会误报，所以用穷举列表是可以接受的。
        for icon in [
            egui::CursorIcon::Default,
            egui::CursorIcon::None,
            egui::CursorIcon::ContextMenu,
            egui::CursorIcon::Help,
            egui::CursorIcon::PointingHand,
            egui::CursorIcon::Progress,
            egui::CursorIcon::Wait,
            egui::CursorIcon::Cell,
            egui::CursorIcon::Crosshair,
            egui::CursorIcon::Text,
            egui::CursorIcon::VerticalText,
            egui::CursorIcon::Alias,
            egui::CursorIcon::Copy,
            egui::CursorIcon::Move,
            egui::CursorIcon::NoDrop,
            egui::CursorIcon::NotAllowed,
            egui::CursorIcon::Grab,
            egui::CursorIcon::Grabbing,
            egui::CursorIcon::AllScroll,
            egui::CursorIcon::ResizeHorizontal,
            egui::CursorIcon::ResizeVertical,
            egui::CursorIcon::ResizeNeSw,
            egui::CursorIcon::ResizeNwSe,
            egui::CursorIcon::ResizeEast,
            egui::CursorIcon::ResizeSouth,
            egui::CursorIcon::ResizeWest,
            egui::CursorIcon::ResizeNorth,
        ] {
            let h = from_icon(icon);
            assert!(!h.is_invalid(), "{icon:?} 映射出空句柄");
        }
    }
}
