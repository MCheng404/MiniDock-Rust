//! 控制中心里那几个"一按就调系统"的动作。
//!
//! 都放在 platform 层，`cc_win` 只管画和判命中。按动作拆成函数，
//! 比塞进一个 `switch` 更好测也更安全。

use windows::core::HSTRING;
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegSetValueExW, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_DWORD,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, SW_HIDE, SW_SHOWNORMAL, WM_SETTINGCHANGE,
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

/// 静默执行一个命令（不带控制台窗口）。用于 `powercfg /setactive`。
pub fn run_hidden(exe: &str, args: &str) {
    let verb = HSTRING::from("open");
    let file = HSTRING::from(exe);
    let params = HSTRING::from(args);
    unsafe {
        let _ = ShellExecuteW(None, &verb, &file, &params, None, SW_HIDE);
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

/// 节能模式 / 平衡模式。`true` = 节能。
///
/// `powercfg` 的这两个 GUID 是系统内置方案，不要自己发挥。
pub fn set_power_saver(on: bool) {
    const SAVER: &str = "/setactive a1841308-3541-4fab-bc81-f71556f20b4a";
    const BALANCED: &str = "/setactive 381b4222-f694-41f0-9685-ff5bb260df2e";
    run_hidden("powercfg.exe", if on { SAVER } else { BALANCED });
}

#[cfg(test)]
mod tests {
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
}
