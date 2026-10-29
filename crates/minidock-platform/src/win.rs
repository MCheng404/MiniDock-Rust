//! 字符串与 DPI 的薄封装。

use windows::Win32::Graphics::Gdi::{GetDC, GetDeviceCaps, ReleaseDC, LOGPIXELSX};

/// `&str` → 以 NUL 结尾的 UTF-16。所有 `*W` API 都吃这个。
#[must_use]
pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// UTF-16 缓冲 → `String`，在第一个 NUL 处截断。
#[must_use]
pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// 系统 DPI（主显示器）。
///
/// 原版用 `GetDeviceCaps(LOGPIXELSX)`，即**系统 DPI**而不是 per-monitor DPI。
/// 这里保持一致：整个程序只按一个 DPI 缩放，进程也不声明 DPI 感知，
/// 由系统统一做位图拉伸。改成 per-monitor v2 是独立议题（见 PORTING.md）。
#[must_use]
pub fn system_dpi() -> i32 {
    let dc = unsafe { GetDC(None) };
    if dc.0.is_null() {
        return 96;
    }
    let dpi = unsafe { GetDeviceCaps(Some(dc), LOGPIXELSX) };
    unsafe { ReleaseDC(None, dc) };
    if dpi <= 0 {
        96
    } else {
        dpi
    }
}

/// 最近一次 Win32 错误的原始码（调试用）。
#[must_use]
pub fn last_error() -> u32 {
    unsafe { windows::Win32::Foundation::GetLastError().0 }
}
