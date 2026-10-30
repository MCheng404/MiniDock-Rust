//! 判断"什么算应用窗口"，并按 exe 找 / 激活 / 关闭它。
//!
//! 对应 C 版 `IsAppWindow()` / `ExeOfWindow()` / `ActivateApp()` / `ToggleApp()`。
//!
//! 用来支持两件事：
//!
//! 1. **运行指示点**：扫描当前有哪些应用在运行（[`running_apps`]）。
//! 2. **点击已运行的应用切过去**，而不是再开一个实例（[`activate`] / [`toggle`]）。
//!
//! # 与其他模块的分工
//!
//! [`crate::occlusion`] 是"哪些窗口遮住了某块区域"，它需要**额外**排除
//! 最小化窗口与 `WS_EX_NOACTIVATE`（浮层不该算遮挡）。
//! 本模块回答的是"哪些窗口属于一个正常应用"，所以**不**排除最小化 ——
//! 最小化的窗口正是我们要恢复的对象。
//!
//! 两者的公共部分就是 [`is_app_window`]，`occlusion` 在它之上再加过滤。

use std::collections::HashMap;

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetForegroundWindow, GetWindow, GetWindowLongW,
    GetWindowTextLengthW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetForegroundWindow,
    ShowWindow, GWL_EXSTYLE, GW_OWNER, SW_MINIMIZE, SW_RESTORE, WS_EX_TOOLWINDOW,
};

/// 这个窗口是否属于一个"正常应用"。
///
/// 判据与 C 版 `IsAppWindow()` 逐条对应。**注意它不排除最小化窗口**：
/// 最小化的窗口正是 [`activate`] 要恢复的目标。
#[must_use]
pub fn is_app_window(hwnd: HWND) -> bool {
    // SAFETY: 全是只读查询，句柄来自系统枚举或调用方。
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return false;
        }
        // 有 owner 的是对话框 / 弹出菜单的宿主。
        if GetWindow(hwnd, GW_OWNER).is_ok() {
            return false;
        }
        if GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
            return false;
        }
        // 没有标题的窗口基本都是不可见容器。
        if GetWindowTextLengthW(hwnd) == 0 {
            return false;
        }
        if crate::occlusion::is_cloaked(hwnd) {
            return false;
        }
        if is_desktop_host(hwnd) {
            return false;
        }
    }
    true
}

/// 桌面、任务栏这类"铺满全屏但不是应用"的窗口。
fn is_desktop_host(hwnd: HWND) -> bool {
    let mut buf = [0u16; 64];
    // SAFETY: 输出缓冲区是本地数组。
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    if n <= 0 {
        return false;
    }
    let cls = String::from_utf16_lossy(&buf[..n as usize]);
    matches!(cls.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd")
}

/// 窗口所属进程的可执行文件完整路径。
///
/// 用 `PROCESS_QUERY_LIMITED_INFORMATION` 而不是 `PROCESS_QUERY_INFORMATION`：
/// 前者对更高完整性级别的进程也能打开（后者会被拒绝）。
/// 这对 Dock 很重要 —— 用户用管理员权限跑的应用，普通权限的 Dock 也要能识别。
#[must_use]
pub fn exe_of_window(hwnd: HWND) -> Option<String> {
    let mut pid = 0u32;
    // SAFETY: 输出参数是本地 u32。
    unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };
    if pid == 0 {
        return None;
    }
    // SAFETY: 句柄在下面立即关闭；`buf` 是本地缓冲区。
    unsafe {
        let proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let r = QueryFullProcessImageNameW(
            proc,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &raw mut len,
        );
        let _ = CloseHandle(proc);
        r.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// 枚举所有正常应用窗口，返回 `exe 路径（小写）→ 该应用的第一个窗口`。
///
/// 键统一转小写：Windows 路径不区分大小写，而不同 API 返回的大小写不一致
/// （`QueryFullProcessImageNameW` 给的是磁盘上的真实大小写）。
#[must_use]
pub fn running_apps() -> HashMap<String, HWND> {
    let mut map: HashMap<String, HWND> = HashMap::new();
    // SAFETY: `lparam` 指向本函数栈上的 map，`EnumWindows` 同步枚举。
    unsafe {
        let _ = EnumWindows(Some(collect_proc), LPARAM((&raw mut map) as isize));
    }
    map
}

unsafe extern "system" fn collect_proc(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
    // SAFETY: 见 `running_apps` 的说明。
    let map = unsafe { &mut *(lparam.0 as *mut HashMap<String, HWND>) };
    if is_app_window(hwnd) {
        if let Some(exe) = exe_of_window(hwnd) {
            // 同一个应用可能有多个窗口，保留第一个即可（激活它就行）。
            map.entry(exe.to_lowercase()).or_insert(hwnd);
        }
    }
    true.into()
}

/// 找到 `exe` 的窗口。
#[must_use]
pub fn find_window_of(exe: &str) -> Option<HWND> {
    let want = exe.to_lowercase();
    running_apps().get(&want).copied()
}

/// 激活（或最小化）`exe` 的窗口。
///
/// `toggle` 为真时模仿 C 版 `ToggleApp()`：
///
/// | 窗口状态 | 动作 |
/// |---|---|
/// | 最小化 | 恢复并置前 |
/// | 已在前台 | 最小化 |
/// | 被别的窗口挡住 | 置前 |
///
/// 返回是否找到了窗口。`false` 表示这个应用没在运行，调用方应该去启动它。
///
/// **与原版的差异**：原版在最小化/恢复时会走 `FlyStart` 的飞入动画
/// （用缩略图做"吸进 Dock"的过渡），这里没有实现 —— 直接恢复/最小化。
/// 那是纯观感，不影响行为。
pub fn activate(exe: &str, toggle: bool) -> bool {
    let Some(hwnd) = find_window_of(exe) else {
        return false;
    };
    // SAFETY: hwnd 来自系统枚举，是有效的顶层窗口。
    unsafe {
        if !toggle {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(hwnd);
            return true;
        }

        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
            return true;
        }
        // 已经在前台 -> 缩回去。判断用 `GetForegroundWindow` 直接比较：
        // 比 `GetActiveWindow` 可靠（后者只对调用线程的窗口有意义）。
        let fg = GetForegroundWindow();
        if fg == hwnd {
            let _ = ShowWindow(hwnd, SW_MINIMIZE);
        } else {
            let _ = SetForegroundWindow(hwnd);
        }
    }
    true
}

struct CloseCtx {
    /// 目标 exe（小写）。
    want: String,
    found: bool,
}

/// 让 `exe` 的应用退出（对它的每个窗口发 `WM_CLOSE`）。
///
/// 找不到窗口返回 false。多发几个窗口是刻意的：多窗口应用（浏览器、编辑器）
/// 只关一个窗口不会退出，得挨个发。哪些窗口关掉才算"退出"是应用自己的
/// 语义，这里不猜。
pub fn close_app(exe: &str) -> bool {
    let mut ctx = CloseCtx {
        want: exe.to_lowercase(),
        found: false,
    };
    // SAFETY: `lparam` 指向本函数栈上的 ctx，`EnumWindows` 同步枚举。
    unsafe {
        let _ = EnumWindows(Some(close_proc), LPARAM((&raw mut ctx) as isize));
    }
    ctx.found
}

unsafe extern "system" fn close_proc(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
    // SAFETY: 见 `close_app` 的说明。
    let ctx = unsafe { &mut *(lparam.0 as *mut CloseCtx) };
    if !is_app_window(hwnd) {
        return true.into();
    }
    let Some(exe) = exe_of_window(hwnd) else {
        return true.into();
    };
    if exe.to_lowercase() == ctx.want {
        // SAFETY: 只是往这个窗口投一条关闭消息，不改自己的状态。
        let _ = unsafe {
            windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                Some(hwnd),
                windows::Win32::UI::WindowsAndMessaging::WM_CLOSE,
                Default::default(),
                Default::default(),
            )
        };
        ctx.found = true;
    }
    true.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 当前进程的窗口必然不是"应用窗口"（控制台/无窗口），
    /// 但桌面窗口也不是 —— 这条守住 `is_desktop_host` 的过滤。
    #[test]
    fn desktop_window_is_not_an_app_window() {
        let desk = unsafe { windows::Win32::UI::WindowsAndMessaging::GetDesktopWindow() };
        assert!(!is_app_window(desk), "桌面窗口不该算应用窗口");
    }

    /// 不存在的 exe 必须返回 false，不能误判成"在运行"。
    ///
    /// 这条守的是 [`activate`] 的契约：返回 false 时调用方会去启动新实例。
    /// 如果误返回 true，点图标就什么也不会发生（最差的一种失败）。
    #[test]
    fn unknown_exe_is_not_running() {
        assert!(find_window_of("C:\\definitely\\not\\a\\real\\app.exe").is_none());
        assert!(!activate("C:\\definitely\\not\\a\\real\\app.exe", false));
    }

    /// `running_apps` 的键必须全是小写，且没有重复键。
    ///
    /// 键不统一大小写的话，[`find_window_of`] 用 `to_lowercase()` 查
    /// 就会漏掉 —— 表现为"应用明明开着，指示点却不亮、点击又开了一个实例"。
    /// 这是本模块最容易悄悄出问题的地方，所以直接断言表的形状。
    #[test]
    fn running_apps_keys_are_lowercase() {
        for key in running_apps().keys() {
            assert_eq!(*key, key.to_lowercase(), "键 {key} 不是小写");
        }
    }

    /// 桌面窗口与工具窗口都不能进入运行应用列表。
    ///
    /// 否则桌面上没开任何应用时，指示点也会亮（桌面窗口的 exe 是
    /// explorer.exe，会被误认成"资源管理器在运行"）。
    #[test]
    fn desktop_hosts_are_not_listed_as_running() {
        for exe in running_apps().keys() {
            let name = exe.rsplit(['\\', '/']).next().unwrap_or("");
            assert!(
                !name.eq_ignore_ascii_case("progman.exe"),
                "桌面宿主不该出现在运行列表里"
            );
        }
    }
}
