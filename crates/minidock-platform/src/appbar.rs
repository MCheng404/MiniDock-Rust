//! 桌面工作区预留（AppBar 协议）。
//!
//! 对应 C 版 `SetupAppBar()`。作用是把屏幕边缘一条区域登记为"系统级工具栏"，
//! 让最大化的窗口避开 Dock —— 也就是 `Reserve` 这个配置项。
//!
//! # 为什么必须 RAII
//!
//! AppBar 的登记是**系统级**的：登记之后 `rcWork` 会永久少一块，
//! 直到进程调用 `ABM_REMOVE` 或者被系统回收。如果进程异常退出（panic、
//! 被强杀），残留的登记会让用户桌面永远缺一条 —— 而且这个问题在重启前
//! 不会自愈，用户根本无法自己修好。
//!
//! 所以这里做成 `Drop` 自动注销，并且在 `remove()` 上加了幂等保护。
//! **不要**把这个结构体放进 `std::mem::forget` 或裸指针里逃逸生命周期。
//!
//! # 为什么用 `ABM_QUERYPOS` 而不是缓存 key
//!
//! C 版用一个 hash key（边 + 厚度 + 显示器矩形）判断"要不要重新提交"。
//! 问题在于 key 之外的因素也会改变结果：别的 AppBar 退出、任务栏换边、
//! 显示器分辨率变化 —— 这些都不在 key 里，于是 Dock 会卡在旧位置。
//!
//! 这里改成每次都走 `ABM_QUERYPOS`（纯查询，无副作用），只有结果真的变了
//! 才 `ABM_SETPOS`。多一次调用换来的是"上层窗口变化时也能跟上"。

use minidock_core::config::Edge;
use minidock_core::layout::Rect;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::Shell::{
    SHAppBarMessage, ABE_BOTTOM, ABE_LEFT, ABE_RIGHT, ABE_TOP, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE,
    ABM_SETPOS, APPBARDATA,
};

/// AppBar 回调消息。收到的是 `ABN_*` 通知。
///
/// 当前不处理具体通知（几何变化时我们本来就会重查），但**必须**给一个值：
/// `ABM_NEW` 要靠它区分不同 AppBar。取 `WM_APP` 之后一个不常用的偏移，
/// 避免和 Dock 自己的消息撞车。
pub const WM_APP_AB: u32 = 0x8000 + 0x4142;

/// 一个已登记的 AppBar。
#[derive(Debug)]
pub struct AppBar {
    hwnd: HWND,
    /// 是否已 `ABM_NEW`。
    registered: bool,
    /// 上一次成功提交的边与矩形。相同就不再 `ABM_SETPOS`。
    applied: Option<(Edge, Rect)>,
}

impl AppBar {
    /// 绑定窗口，但**不**立刻登记（要等第一次 `apply`）。
    #[must_use]
    pub fn new(hwnd: HWND) -> Self {
        Self {
            hwnd,
            registered: false,
            applied: None,
        }
    }

    /// 请求预留 `thickness` 宽的边条，返回实际拿到的矩形。
    ///
    /// 调用方要把返回的矩形传给 [`minidock_core::layout::window_origin`]，
    /// 因为 `ABM_QUERYPOS` 可能把请求位置调整到"实际能给"的地方。
    ///
    /// `monitor` 是目标显示器的**整块**矩形（`rcMonitor`，不是 `rcWork`）：
    /// AppBar 是在整个屏幕上切，不是在工作区里切。
    pub fn apply(&mut self, edge: Edge, thickness: i32, monitor: Rect) -> Option<Rect> {
        if !self.registered {
            let mut ab = self.base();
            ab.uCallbackMessage = WM_APP_AB;
            // ABM_NEW 失败（返回 FALSE）说明系统拒绝登记，此时不该继续
            // SETPOS —— 那样只会安静地什么也不做，还留下错误的状态。
            if unsafe { SHAppBarMessage(ABM_NEW, &raw mut ab) } == 0 {
                return None;
            }
            self.registered = true;
        }

        let mut ab = self.base();
        let mut want = monitor;
        match edge {
            Edge::Top => {
                ab.uEdge = ABE_TOP;
                want.bottom = want.top + thickness;
            }
            Edge::Bottom => {
                ab.uEdge = ABE_BOTTOM;
                want.top = want.bottom - thickness;
            }
            Edge::Left => {
                ab.uEdge = ABE_LEFT;
                want.right = want.left + thickness;
            }
            Edge::Right => {
                ab.uEdge = ABE_RIGHT;
                want.left = want.right - thickness;
            }
        }
        ab.rc = to_win_rect(want);

        // 系统会把它挪到实际可用的位置（避开任务栏与别的 AppBar）。
        unsafe { SHAppBarMessage(ABM_QUERYPOS, &raw mut ab) };

        // QUERYPOS 可能只调整了坐标，厚度必须按我们自己的要求重新固定，
        // 否则拿回来的可能是一条很窄的缝。
        match edge {
            Edge::Top => ab.rc.bottom = ab.rc.top + thickness,
            Edge::Bottom => ab.rc.top = ab.rc.bottom - thickness,
            Edge::Left => ab.rc.right = ab.rc.left + thickness,
            Edge::Right => ab.rc.left = ab.rc.right - thickness,
        }

        let got = from_win_rect(ab.rc);
        if self.applied == Some((edge, got)) {
            return Some(got);
        }

        unsafe { SHAppBarMessage(ABM_SETPOS, &raw mut ab) };
        self.applied = Some((edge, got));
        Some(got)
    }

    /// 注销并把工作区还给系统。幂等。
    pub fn remove(&mut self) {
        if !self.registered {
            return;
        }
        let mut ab = self.base();
        unsafe { SHAppBarMessage(ABM_REMOVE, &raw mut ab) };
        self.registered = false;
        self.applied = None;
    }

    fn base(&self) -> APPBARDATA {
        APPBARDATA {
            cbSize: u32::try_from(std::mem::size_of::<APPBARDATA>()).unwrap_or(40),
            hWnd: self.hwnd,
            ..Default::default()
        }
    }
}

impl Drop for AppBar {
    fn drop(&mut self) {
        // 关键：不注销的话用户桌面会永久缺一条，重启前无法自愈。
        self.remove();
    }
}

fn to_win_rect(r: Rect) -> RECT {
    RECT {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    }
}

fn from_win_rect(r: RECT) -> Rect {
    Rect::new(r.left, r.top, r.right, r.bottom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_roundtrips_through_win_rect() {
        let r = Rect::new(-100, 20, 300, 1040);
        assert_eq!(from_win_rect(to_win_rect(r)), r);
    }

    /// 未登记时 `remove` 必须是空操作。
    ///
    /// `Drop` 会无条件调用它，如果没做幂等保护，一个从未成功登记的
    /// 实例在析构时会向系统发一次 `ABM_REMOVE` —— 那是另一个 AppBar
    /// 的槽位也可能被误伤（同一 hwnd 重复注销的行为没有明确文档保证）。
    #[test]
    fn remove_is_idempotent_before_new() {
        // 拿一个无效句柄：只要不真的走 SHAppBarMessage 就不会崩。
        let mut bar = AppBar::new(HWND::default());
        bar.remove();
        bar.remove();
        assert!(!bar.registered);
    }
}
