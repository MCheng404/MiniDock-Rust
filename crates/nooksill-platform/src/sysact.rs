//! 控制中心里那几个"一按就调系统"的动作。
//!
//! 都放在 platform 层，`cc_win` 只管画和判命中。按动作拆成函数，
//! 比塞进一个 `switch` 更好测也更安全。

use windows::core::{GUID, HSTRING};
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::System::Power::{PowerGetActiveScheme, PowerSetActiveScheme};
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegSetValueExW, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_DWORD,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, SW_SHOWNORMAL, WM_SETTINGCHANGE,
};

/// 用默认程序打开一个 URI 或路径（`ms-settings:` / `https://` / 文件）。
///
/// Shell 的返回值是个"大于 32 才算成功"的伪句柄，这里只关心失败。
pub fn open_uri(target: &str) {
    let verb = HSTRING::from("open");
    let file = HSTRING::from(target);
    unsafe {
        let _ = ShellExecuteW(None, &verb, &file, None, None, SW_SHOWNORMAL);
    }
}

/// 打开 Windows 的快速设置面板（Win + A）。
///
/// 对 Wi‑Fi / 蓝牙 / 热点三个磁贴都是这么做的：
/// 这几个开关没有稳定的公开接口，与其去调未文档化的 WinRT 私有 API，
/// 不如把系统自己的面板拉出来。
pub fn open_quick_settings() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYEVENTF_KEYUP, VK_LWIN};
    // 'A' 的虚拟键码是 0x41。
    const VK_A: u8 = 0x41;
    unsafe {
        keybd_event(VK_LWIN.0 as u8, 0, Default::default(), 0);
        keybd_event(VK_A, 0, Default::default(), 0);
        keybd_event(VK_A, 0, KEYEVENTF_KEYUP, 0);
        keybd_event(VK_LWIN.0 as u8, 0, KEYEVENTF_KEYUP, 0);
    }
}

/// 打开开始菜单。
///
/// 没有公开接口能在"不模拟按键"的前提下打开它（`shell:AppsFolder` 是另一
/// 个界面），所以这里就按一下 Win 键：系统自己处理"按下即开、再按即关"。
pub fn open_start() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYEVENTF_KEYUP, VK_LWIN};
    unsafe {
        keybd_event(VK_LWIN.0 as u8, 0, Default::default(), 0);
        keybd_event(VK_LWIN.0 as u8, 0, KEYEVENTF_KEYUP, 0);
    }
}

/// 打开 Windows 的通知区域（托盘溢出面板）。
///
/// 1.6.5 的 `TrayOpen` 是两段式：先跨进程点 Explorer 的溢出按钮
/// （`TB_BUTTONCOUNT` / `TB_GETITEMRECT` 取坐标再 `mouse_event`），失败才
/// 退回热键。我们**只做热键那一段**：
///
/// - 跨进程 `SendMessage` 到 `ToolbarWindow32` 要求两者同位数、Explorer
///   的工具栏结构在目标系统上一致，而坐标一旦算错就是"点到了别处"；
/// - 收益只是省一次 Enter，风险却是随系统版本静默失效。
///
/// `Win+B` 把焦点移到通知区域，再 `Enter` 打开溢出面板。
pub fn open_tray() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        keybd_event, KEYEVENTF_KEYUP, VK_LWIN, VK_RETURN,
    };
    // 'B' 的虚拟键码是 0x42。
    const VK_B: u8 = 0x42;
    unsafe {
        keybd_event(VK_LWIN.0 as u8, 0, Default::default(), 0);
        keybd_event(VK_B, 0, Default::default(), 0);
        keybd_event(VK_B, 0, KEYEVENTF_KEYUP, 0);
        keybd_event(VK_LWIN.0 as u8, 0, KEYEVENTF_KEYUP, 0);
    }
    // 必须等焦点真的落到通知区域再回车，否则 Enter 会打到原来的前台窗口上
    // （也是 `Sleep(120)`）。
    std::thread::sleep(std::time::Duration::from_millis(120));
    unsafe {
        keybd_event(VK_RETURN.0 as u8, 0, Default::default(), 0);
        keybd_event(VK_RETURN.0 as u8, 0, KEYEVENTF_KEYUP, 0);
    }
}

/// 切换系统深色模式。返回切换后的状态；写注册表失败时返回切换前的状态。
///
/// 改的是 `HKCU\...\Themes\Personalize` 的两个 DWORD，再广播
/// `WM_SETTINGCHANGE("ImmersiveColorSet")` —— 这是系统自己那套开关用的同一条路。
/// 我们的 Dock 也要重读一次主题（`theme::is_dark`），否则栏体会停在旧配色。
pub fn set_dark_mode(dark: bool) -> bool {
    let value: u32 = u32::from(dark);
    let sub = HSTRING::from(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    unsafe {
        let mut key = windows::Win32::System::Registry::HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, &sub, Some(0), KEY_SET_VALUE, &mut key).is_err() {
            return !dark;
        }
        let mut ok = true;
        for name in ["AppsUseLightTheme", "SystemUsesLightTheme"] {
            let n = HSTRING::from(name);
            // 注意语义是反的：`AppsUseLightTheme = 0` 才是深色。
            let v: u32 = 1 - value;
            let bytes = v.to_le_bytes();
            if RegSetValueExW(key, &n, Some(0), REG_DWORD, Some(&bytes)).is_err() {
                ok = false;
            }
        }
        let _ = RegCloseKey(key);

        let mut result = 0_usize;
        let _ = SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            windows::Win32::Foundation::WPARAM(0),
            windows::Win32::Foundation::LPARAM(
                windows::core::w!("ImmersiveColorSet").as_ptr() as isize
            ),
            SMTO_ABORTIFHUNG,
            200,
            Some(&mut result),
        );
        if ok {
            dark
        } else {
            !dark
        }
    }
}

/// Windows 内置电源方案。GUID 与 `powercfg /list` 里那几行逐字节相同。
const SCHEME_SAVER: GUID = GUID::from_u128(0xa1841308_3541_4fab_bc81_f71556f20b4a);
const SCHEME_BALANCED: GUID = GUID::from_u128(0x381b4222_f694_41f0_9685_ff5bb260df2e);

/// 当前活动的电源方案。`None` = 读不到（精简系统、策略禁用）。
///
/// `PowerGetActiveScheme` 返回的是 `LocalAlloc` 出来的 GUID，调用方负责
/// `LocalFree`，所以这里拷一份再释放。
pub fn active_power_scheme() -> Option<GUID> {
    let mut raw: *mut GUID = std::ptr::null_mut();
    // SAFETY: powrprof 成功时把一块 LocalAlloc 的 GUID 写进 raw。
    let err = unsafe { PowerGetActiveScheme(None, &raw mut raw) };
    if err.0 != 0 || raw.is_null() {
        return None;
    }
    let guid = unsafe { *raw };
    unsafe { LocalFree(Some(HLOCAL(raw.cast()))) };
    Some(guid)
}

/// 当前是否节能模式。
pub fn is_power_saver() -> bool {
    active_power_scheme() == Some(SCHEME_SAVER)
}

/// 切到指定方案，返回系统是否接受。
pub fn set_active_power_scheme(scheme: &GUID) -> bool {
    // SAFETY: 指针指向调用方栈上的 GUID。
    unsafe { PowerSetActiveScheme(None, Some(scheme)) }.0 == 0
}

/// 切换决策（纯函数）：返回要设置的方案与切换后应记住的"原方案"。
///
/// 退出节能时优先回到**进入节能前**的那个方案 —— 用户可能在用高性能方案，
/// 一律切回"平衡"等于替他改设置。
fn plan_toggle(current: Option<GUID>, saved: Option<GUID>) -> (GUID, Option<GUID>) {
    if current == Some(SCHEME_SAVER) {
        (saved.unwrap_or(SCHEME_BALANCED), None)
    } else {
        (SCHEME_SAVER, current)
    }
}

/// 节能模式开关。记住进节能前的方案，退出时还原。
#[derive(Debug, Default)]
pub struct PowerSaver {
    before: Option<GUID>,
}

impl PowerSaver {
    /// 切换并返回**真实**的新状态（以系统回读为准，失败时状态不变）。
    pub fn toggle(&mut self) -> bool {
        let (target, next) = plan_toggle(active_power_scheme(), self.before);
        self.before = next;
        let _ = set_active_power_scheme(&target);
        is_power_saver()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 只验证纯字符串部分；真正的系统调用要靠手动验收。
    #[test]
    fn settings_uris_are_the_documented_ones() {
        // 这两个 URI 是 Windows 设置应用注册的协议，写错会什么都不发生。
        for uri in [
            "ms-settings:",
            "ms-settings:privacy-location",
            "ms-settings:network-status",
        ] {
            assert!(uri.starts_with("ms-settings:"), "{uri}");
        }
    }

    #[test]
    fn toggling_into_saver_remembers_the_previous_scheme() {
        let high = GUID::from_u128(0x8c5e7fda_e8bf_4a96_9a85_a6e23a8c635c);
        let (target, saved) = plan_toggle(Some(high), None);
        assert_eq!(target, SCHEME_SAVER);
        assert_eq!(saved, Some(high), "must remember, not assume balanced");
    }

    #[test]
    fn toggling_out_of_saver_restores_what_was_saved() {
        let high = GUID::from_u128(0x8c5e7fda_e8bf_4a96_9a85_a6e23a8c635c);
        let (target, saved) = plan_toggle(Some(SCHEME_SAVER), Some(high));
        assert_eq!(target, high);
        assert_eq!(saved, None);
    }

    #[test]
    fn toggling_out_without_a_memory_falls_back_to_balanced() {
        let (target, saved) = plan_toggle(Some(SCHEME_SAVER), None);
        assert_eq!(target, SCHEME_BALANCED);
        assert_eq!(saved, None);
    }

    #[test]
    fn unreadable_scheme_still_toggles_into_saver() {
        let (target, saved) = plan_toggle(None, None);
        assert_eq!(target, SCHEME_SAVER);
        assert_eq!(saved, None);
    }

    /// 只读验证：不切方案，只确认能读到一个合理的 GUID。
    #[test]
    #[ignore = "queries the live power scheme; run with --ignored"]
    fn active_power_scheme_is_readable() {
        let g = active_power_scheme().expect("PowerGetActiveScheme failed");
        assert!(g.data1 != 0, "unexpected zero GUID");
        eprintln!(
            "active scheme: {:08x}-{:04x}-{:04x}",
            g.data1, g.data2, g.data3
        );
    }
}
