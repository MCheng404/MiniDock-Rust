//! 字符串、DPI 与 COM 初始化的薄封装。

use windows::Win32::Graphics::Gdi::{GetDC, GetDeviceCaps, ReleaseDC, LOGPIXELSX};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};

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

thread_local! {
    /// 本线程的 COM 是否已初始化。**必须按线程**，不能按进程。
    ///
    /// 这里原本是一个 `static Once`：`Once` 保证的是"整个进程只执行一次"，
    /// 所以只有第一个调用它的线程被初始化，其余线程上的
    /// `CoCreateInstance` 会以 `CO_E_NOTINITIALIZED` 失败。单线程时看不出
    /// 来，一旦把取图标挪到工作线程，就变成"图标偶尔取不到"这种
    /// 没有报错、没有崩溃、只是结果缺失的问题。
    ///
    /// 选 STA（`COINIT_APARTMENTTHREADED`）而不是 MTA：shell 的
    /// `IShellLink` / `IImageList` 在 STA 下行为最稳。不配对
    /// `CoUninitialize` —— 线程退出时系统回收；提前反初始化会让之后任何
    /// COM 调用都失败。
    static COM_READY: () = {
        // SAFETY: 每个线程只执行一次（`thread_local!` 的惰性初始化保证），
        // 且没有配对的反初始化（见上）。
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
    };
}

/// 保证**当前线程**已初始化 COM。可重复调用，也可从任意线程调用。
///
/// 若调用方已经把线程初始化成 MTA，`CoInitializeEx` 会返回
/// `RPC_E_CHANGED_MODE`，此时仍然可以用 shell 的对象（它们不要求特定套间），
/// 所以忽略错误。
pub fn ensure_com() {
    COM_READY.with(|()| {});
}
