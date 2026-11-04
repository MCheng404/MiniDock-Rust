//! 系统深浅色 / 强调色 / 开机自启动（注册表）。
//!
//! 全部是"读一次、失败就用默认值"的语义：注册表被策略锁死时 Dock 也要能
//! 起得来。

use windows::core::w;
use windows::Win32::Foundation::{ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegQueryValueExW,
    RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE,
    REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_DWORD,
};

use crate::win::{last_error, to_wide};

/// 系统是否为深色模式（`AppsUseLightTheme == 0`）。
#[must_use]
pub fn is_dark() -> bool {
    let mut value: u32 = 1;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(std::ptr::addr_of_mut!(value).cast()),
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        // 读不到就按浅色处理：与"系统默认浅色"的观感一致。
        return false;
    }
    value == 0
}

/// Windows 强调色（DWM 的着色色）原始 RGB，未做明度归一化。
///
/// `None` = 取不到（远程会话、组策略禁用等）。界面主题色要用这个原始值，
/// 因为自定义色与系统色走的是同一条 COLORREF 路径；归一化版见 [`accent`]。
#[must_use]
pub fn accent_raw() -> Option<[u8; 3]> {
    use windows::core::BOOL;
    use windows::Win32::Graphics::Dwm::DwmGetColorizationColor;

    let mut color: u32 = 0;
    let mut opaque = BOOL(0);
    // SAFETY: 两个参数都是本栈帧上的有效指针，调用期间不会被移动。
    unsafe { DwmGetColorizationColor(&mut color, &mut opaque) }.ok()?;
    Some([
        ((color >> 16) & 0xFF) as u8,
        ((color >> 8) & 0xFF) as u8,
        (color & 0xFF) as u8,
    ])
}

/// Windows 强调色，归一化到**最大分量 170**：DWM 给的是用户自己调的颜色，
/// 明度差异很大 —— 冰蓝的 (200,230,255) 与深蓝的 (0,60,120) 直接拿去当
/// 烟雾色，前者会亮到糊成白团、后者几乎看不见。压到同一上限之后只有色相
/// 在起作用，这正是烟雾"跟随强调色"想要的语义。
#[must_use]
pub fn accent() -> Option<[f32; 3]> {
    let raw = accent_raw()?;
    let rgb = [raw[0] as f32, raw[1] as f32, raw[2] as f32];
    let max = rgb[0].max(rgb[1]).max(rgb[2]);
    if max < 1.0 {
        return None;
    }
    Some([
        rgb[0] * 170.0 / max,
        rgb[1] * 170.0 / max,
        rgb[2] * 170.0 / max,
    ])
}

/// 开机自启动是否已注册。
///
/// 判定方式是"值存在"，不是"值等于当前路径"。
/// 用户挪动过 exe 之后旧路径仍在，这里会如实返回 true；`set_autostart(true)`
/// 会把它改写成当前路径。
#[must_use]
pub fn get_autostart() -> bool {
    let mut key = HKEY::default();
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Run"),
            None,
            KEY_QUERY_VALUE,
            &mut key,
        )
    };
    if status != ERROR_SUCCESS {
        return false;
    }
    let mut size = 0u32;
    let status =
        unsafe { RegQueryValueExW(key, w!("MiniDock"), None, None, None, Some(&mut size)) };
    unsafe {
        let _ = RegCloseKey(key);
    };
    status == ERROR_SUCCESS
}

/// 注册 / 取消开机自启动。返回值表示是否成功写入。
pub fn set_autostart(on: bool) -> bool {
    let mut key = HKEY::default();
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Run"),
            None,
            w!(""),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
    };
    if status != ERROR_SUCCESS {
        return false;
    }

    let ok = if on {
        let cmd = format!(
            "\"{}\"",
            std::env::current_exe().unwrap_or_default().display()
        );
        let wide = to_wide(&cmd);
        let bytes =
            unsafe { std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2) };
        unsafe { RegSetValueExW(key, w!("MiniDock"), None, REG_SZ, Some(bytes)) == ERROR_SUCCESS }
    } else {
        // 值不存在也算成功。
        let s = unsafe { RegDeleteValueW(key, w!("MiniDock")) };
        s == ERROR_SUCCESS || s == WIN32_ERROR(2) // ERROR_FILE_NOT_FOUND
    };

    unsafe {
        let _ = RegCloseKey(key);
    };
    ok
}

/// 最后一次注册表操作的错误码，便于日志（`set_autostart` 失败时用）。
#[must_use]
pub fn last_registry_error() -> u32 {
    last_error()
}
