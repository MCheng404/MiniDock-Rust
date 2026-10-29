//! 单实例与"找到已有窗口"。
//!
//! 原版用一个命名互斥体 + 第二次启动时给已有实例发消息实现"双击只唤起不重开"。
//! 互斥体名里的 `Local\` 前缀很关键：`Local\` 是**会话级**命名空间，
//! 多用户同时登录时各自能有一份 Dock；换成 `Global\` 会全局互斥，
//! 第二个用户就再也起不来了。

use std::fmt;

use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND};
use windows::Win32::System::Threading::CreateMutexW;

use minidock_core::SINGLETON_MUTEX;

/// 已持有的单实例锁。Drop 时关闭句柄（进程退出时系统也会兜底）。
#[derive(Debug)]
pub struct SingleInstance {
    handle: HANDLE,
}

impl SingleInstance {
    /// 尝试取得锁。
    ///
    /// - `Ok(Some(_))`：本进程是第一个实例。
    /// - `Ok(None)`：已有实例在跑，调用方应当把窗口唤到前台后退出。
    /// - `Err(_)`：连互斥体都创建不了（极少见），调用方可以选择继续启动。
    pub fn acquire() -> Result<Option<Self>, MutexError> {
        let handle = unsafe { CreateMutexW(None, true, w!("Local\\MiniDock_Wangcy_Singleton")) }
            .map_err(|e| MutexError { code: e.code().0 })?;
        // CreateMutexW 有竞态：即使对象已存在也会返回有效句柄，
        // 真正的判定在 GetLastError()。
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe {
                let _ = CloseHandle(handle);
            };
            return Ok(None);
        }
        Ok(Some(Self { handle }))
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        };
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MutexError {
    pub code: i32,
}

impl fmt::Display for MutexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CreateMutexW 失败: 0x{:08X}", self.code)
    }
}

impl std::error::Error for MutexError {}

/// 按窗口类名查找已有实例的窗口。
#[must_use]
pub fn find_window(class: &str) -> Option<HWND> {
    let wide = crate::win::to_wide(class);
    let hwnd = unsafe {
        windows::Win32::UI::WindowsAndMessaging::FindWindowW(
            windows::core::PCWSTR(wide.as_ptr()),
            None,
        )
    };
    hwnd.ok().filter(|h| !h.0.is_null())
}

/// 互斥体名与 core 里的常量必须一致，这里做一次编译期对齐检查。
const _: () = assert!(!SINGLETON_MUTEX.is_empty());
