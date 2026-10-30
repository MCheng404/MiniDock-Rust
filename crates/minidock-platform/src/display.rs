//! 显示器枚举与工作区。
//!
//! 原版 `GetMon()` 的两个细节必须保留：
//! 1. **主显示器排第一**，`Monitor` 配置项的下标才有稳定含义；
//! 2. 枚举失败时回落到"包含 (0,0) 的显示器"，而不是报错 ——
//!    Dock 是常驻程序，宁可退化到默认屏也不能起不来。

use minidock_core::layout::Rect;
use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint, HDC, HMONITOR, MONITORINFO,
    MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;

/// 一个显示器。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Monitor {
    /// 原生句柄，仅用于比较/调试。
    pub handle: isize,
    /// 完整区域（含任务栏）。
    pub rect: Rect,
    /// 工作区（不含任务栏）。
    pub work: Rect,
    pub primary: bool,
}

unsafe extern "system" fn enum_proc(
    monitor: HMONITOR,
    _dc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    // SAFETY: `data` 由 `monitors()` 传入，指向一个存活的 Vec<HMONITOR>，
    // 且回调在 `EnumDisplayMonitors` 返回前同步执行。
    let list = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
    list.push(monitor);
    BOOL(1)
}

fn info_of(monitor: HMONITOR) -> Option<(Rect, Rect, bool)> {
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut mi) }.as_bool() {
        return None;
    }
    let r = mi.rcMonitor;
    let w = mi.rcWork;
    Some((
        Rect::new(r.left, r.top, r.right, r.bottom),
        Rect::new(w.left, w.top, w.right, w.bottom),
        mi.dwFlags & MONITORINFOF_PRIMARY != 0,
    ))
}

/// 枚举所有显示器，主显示器排第一。
#[must_use]
pub fn monitors() -> Vec<Monitor> {
    let mut handles: Vec<HMONITOR> = Vec::new();
    let ok = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(enum_proc),
            LPARAM(std::ptr::addr_of_mut!(handles) as isize),
        )
    };
    if !ok.as_bool() && handles.is_empty() {
        // 极端情况（会话 0 / 远程桌面）：退回主显示器。
        let fallback = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
        if let Some((rect, work, _)) = info_of(fallback) {
            return vec![Monitor {
                handle: fallback.0 as isize,
                rect,
                work,
                primary: true,
            }];
        }
        return Vec::new();
    }

    let mut out: Vec<Monitor> = handles
        .into_iter()
        .filter_map(|h| {
            info_of(h).map(|(rect, work, primary)| Monitor {
                handle: h.0 as isize,
                rect,
                work,
                primary,
            })
        })
        .collect();

    // 主显示器前置，其余保持系统顺序。
    if let Some(pos) = out.iter().position(|m| m.primary) {
        out.swap(0, pos);
    }
    out
}

/// 取第 `index` 个显示器的工作区；越界回落到主显示器，再不行回落到 1080p。
#[must_use]
pub fn work_area(index: i32) -> Rect {
    let list = monitors();
    if list.is_empty() {
        return Rect::new(0, 0, 1920, 1040);
    }
    let idx = if index >= 0 && (index as usize) < list.len() {
        index as usize
    } else {
        0
    };
    list[idx].work
}

/// 取第 `index` 个显示器的**完整**区域（含任务栏）。
///
/// 与 [`work_area`] 的区别只在 AppBar 上体现：`SHAppBarMessage` 是在整个
/// 屏幕上切一条，不是在 `rcWork` 里切。传错会算出一条"两次都扣掉任务栏高度"
/// 的窄条，表现为 Dock 比预期矮一半。
#[must_use]
pub fn monitor_rect(index: i32) -> Rect {
    let list = monitors();
    list.first().map_or_else(
        || Rect::new(0, 0, 1920, 1080),
        |_| {
            let idx = if index >= 0 && (index as usize) < list.len() {
                index as usize
            } else {
                0
            };
            list[idx].rect
        },
    )
}

/// 系统 DPI（见 [`crate::win::system_dpi`] 的说明，为什么要单列一个导出）。
#[must_use]
pub fn system_dpi() -> i32 {
    crate::win::system_dpi()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_least_one_monitor_on_a_desktop() {
        // 这条测试只能在有桌面的会话里跑（CI 的 Windows runner 满足）。
        let list = monitors();
        assert!(!list.is_empty(), "至少应枚举到一个显示器");
        assert!(list[0].primary || list.len() == 1);
        assert!(list[0].work.width() > 0);
    }

    #[test]
    fn work_area_is_within_monitor() {
        let list = monitors();
        if list.is_empty() {
            return;
        }
        let m = list[0];
        assert!(m.work.left >= m.rect.left);
        assert!(m.work.top >= m.rect.top);
        assert!(m.work.right <= m.rect.right);
        assert!(m.work.bottom <= m.rect.bottom);
    }

    #[test]
    fn out_of_range_index_falls_back_to_primary() {
        let a = work_area(0);
        let b = work_area(99);
        assert_eq!(a, b);
    }
}
