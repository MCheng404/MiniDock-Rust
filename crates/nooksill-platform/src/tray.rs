//! 托盘图标：Dock 被误关之后的唯一恢复入口。
//!
//! # 为什么需要它
//!
//! Dock 窗口是 `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`，不进任务栏也不进
//! Alt+Tab。此前一旦从右键菜单「退出 Dock」，或者 Ctrl+Alt+D 收起后忘了
//! 快捷键，用户只剩"重新找到 exe 双击"一条路。托盘图标是这类常驻工具
//! 的标准兜底。
//!
//! # 只做图标，不做菜单
//!
//! 托盘右键菜单复用 [`nooksill_gpu::menu`] 的自绘模态窗口，而不是 Win32
//! `TrackPopupMenu`：后者要求调用线程拥有前台窗口，Dock 从不去抢前台，
//! 实测会出现"菜单点不消失"。这里只负责注册图标、把点击转成消息。
//!
//! `WM_APP_TRAY` 的 `lParam` 低字是鼠标消息（`WM_LBUTTONUP` /
//! `WM_RBUTTONUP`），由主循环取走后决定弹菜单还是切换显隐。

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    LoadIconW, HICON, IDI_APPLICATION, WM_APP, WM_LBUTTONUP, WM_RBUTTONUP,
};

/// 托盘回调消息。
///
/// **必须是 `WM_APP + 2`**：`worker::WM_APP_WAKE` 已经占了 `WM_APP + 1`，
/// 而托盘回调正是发给 Dock 那个 hwnd 的。撞号会把"托盘被点"读成
/// "后台线程唤醒"（`pump` 里据此决定是否出帧），两边都失效。
/// `appbar::WM_APP_AB` 用的是 `0x8000 + 0x4142` 的高位偏移，不冲突。
pub const WM_APP_TRAY: u32 = WM_APP + 2;

/// 托盘图标 ID（一个进程只有一个）。
const TRAY_ID: u32 = 1;

/// 创建托盘图标失败的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayError {
    /// `Shell_NotifyIconW(NIM_ADD)` 返回失败（Explorer 未就绪 / 策略拦截）。
    Add,
    /// 连系统的默认应用图标都取不到。
    NoIcon,
}

impl std::fmt::Display for TrayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Add => write!(f, "系统托盘不可用"),
            Self::NoIcon => write!(f, "取不到应用图标"),
        }
    }
}

impl std::error::Error for TrayError {}

/// 托盘图标。RAII：析构时注销，不会在通知区留一个死图标。
#[derive(Debug)]
pub struct Tray {
    hwnd: HWND,
    added: bool,
}

impl Tray {
    /// 注册图标。`icon` 可以无效（rc.exe 没跑时），此时退回系统默认图标。
    pub fn add(hwnd: HWND, icon: HICON) -> Result<Self, TrayError> {
        let icon = if icon.is_invalid() {
            // SAFETY: 加载的是系统共享图标，不需要 DestroyIcon。
            unsafe { LoadIconW(None, IDI_APPLICATION) }.map_err(|_| TrayError::NoIcon)?
        } else {
            icon
        };
        let mut data = base_data(hwnd);
        data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        data.uCallbackMessage = WM_APP_TRAY;
        data.hIcon = icon;
        // 提示文字：鼠标停在图标上时显示。UTF-16 + NUL，长度按数组算。
        let tip: Vec<u16> = "NookSill".encode_utf16().collect();
        for (i, c) in tip.into_iter().enumerate().take(data.szTip.len() - 1) {
            data.szTip[i] = c;
        }
        // SAFETY: `data` 在本栈帧上，`cbSize` 由 `base_data` 用 size_of 填好。
        let ok = unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool();
        if !ok {
            return Err(TrayError::Add);
        }
        Ok(Self { hwnd, added: true })
    }

    /// 注销图标。重复调用是安全的。
    pub fn remove(&mut self) {
        if !self.added {
            return;
        }
        self.added = false;
        let data = base_data(self.hwnd);
        // SAFETY: 同上。删除只需要 cbSize / hWnd / uID 三项。
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        }
    }

    /// 把一条 `WM_APP_TRAY` 的 `lParam` 翻译成动作。
    ///
    /// 返回 `Some(true)` = 左键（切换显隐），`Some(false)` = 右键（弹菜单），
    /// `None` = 不关心（鼠标移动、气泡关闭等）。
    #[must_use]
    pub fn parse(lparam: isize) -> Option<bool> {
        match (lparam & 0xFFFF) as u32 {
            WM_LBUTTONUP => Some(true),
            WM_RBUTTONUP => Some(false),
            _ => None,
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.remove();
    }
}

fn base_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: u32::try_from(std::mem::size_of::<NOTIFYICONDATAW>()).unwrap_or(0),
        hWnd: hwnd,
        uID: TRAY_ID,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_message_does_not_collide_with_worker_wake() {
        // 这两个消息都会发到 Dock 的 hwnd；撞号会让托盘点击被 pump
        // 当成后台唤醒而丢掉事件。
        assert_ne!(WM_APP_TRAY, WM_APP + 1);
        assert_eq!(WM_APP_TRAY, WM_APP + 2);
    }

    #[test]
    fn parses_left_and_right_click_only() {
        assert_eq!(Tray::parse(WM_LBUTTONUP as isize), Some(true));
        assert_eq!(Tray::parse(WM_RBUTTONUP as isize), Some(false));
        assert_eq!(Tray::parse(0x0200), None, "鼠标移动不算");
        assert_eq!(Tray::parse(0x0202), Some(true), "WM_LBUTTONUP");
    }

    #[test]
    fn base_data_has_a_valid_size_and_id() {
        let d = base_data(HWND::default());
        assert_eq!(d.cbSize as usize, std::mem::size_of::<NOTIFYICONDATAW>());
        assert_eq!(d.uID, TRAY_ID);
    }
}
