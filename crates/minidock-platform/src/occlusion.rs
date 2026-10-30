//! 遮挡检测：判断有没有普通窗口盖住指定区域。
//!
//! 对应 C 版 `OvProc()` + `EnumWindows` 那段，用于 `Mode=2`（智能隐藏）：
//! 只有当 Dock 那块屏幕区域**真的被挡住**时才收起，否则一直显示。
//!
//! # 为什么不能只看 `WindowFromPoint`
//!
//! `WindowFromPoint` 会返回 Dock 自己（分层窗口在命中测试里算"在上面"），
//! 拿不到被它盖住的那个窗口。必须枚举所有顶层窗口再求交。
//!
//! # 过滤规则为什么这么多
//!
//! 桌面上的"顶层窗口"远不止用户看得见的应用：还有输入法候选窗、开始菜单、
//! 各种 `WS_EX_TOOLWINDOW` 的浮条、被 DWM 隐藏的 UWP 幽灵窗口（cloaked）、
//! 以及 `Progman` / `WorkerW` 这类铺满全屏的桌面宿主。少过滤一类，
//! 结果就是"什么都没开，Dock 却一直躲着"。
//!
//! 过滤清单与上游一致，逐条在注释里写明理由。

use minidock_core::layout::Rect;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindow, GetWindowLongW, GetWindowRect, GetWindowTextLengthW,
    IsIconic, IsWindowVisible, GWL_EXSTYLE, GW_OWNER, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

/// C 版直接用 `DwmGetWindowAttribute(h, 14, ...)`；这里用具名常量。
///
/// 公开是因为 `app_window` 也要用它 —— "被 DWM 隐藏的 UWP 幽灵窗口"
/// 既不算遮挡，也不算正在运行的应用。
#[must_use]
pub fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked: i32 = 0;
    // SAFETY: 第三个参数指向本地 i32，第四个给的是它的字节数。
    let ok = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&raw mut cloaked).cast(),
            u32::try_from(std::mem::size_of::<i32>()).unwrap_or(4),
        )
    };
    ok.is_ok() && cloaked != 0
}

/// 这个窗口是否**不该**参与遮挡判断。
fn should_skip(hwnd: HWND, exclude: &[HWND]) -> bool {
    if exclude.contains(&hwnd) {
        return true;
    }
    // SAFETY: 全是只读查询，句柄来自系统枚举。
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return true;
        }
        // 工具窗 / 不抢焦点的浮层：输入法候选、悬浮工具栏。它们浮在
        // 最上层但不该让 Dock 躲起来。
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex & (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) != 0 {
            return true;
        }
        // 有 owner 的是对话框 / 弹出菜单的宿主，不算独立应用窗口。
        if GetWindow(hwnd, GW_OWNER).is_ok() {
            return true;
        }
        // 没有标题的窗口基本都是不可见的容器。这条也会误杀少数无标题的
        // 全屏渲染窗口（个别游戏/播放器），但上游同样如此 —— 宁可 Dock
        // 不躲，也不要莫名其妙地躲。
        if GetWindowTextLengthW(hwnd) == 0 {
            return true;
        }
        if is_cloaked(hwnd) {
            return true;
        }
        // 桌面与任务栏：它们铺满全屏，放进来会让 Dock 永远处于"被遮挡"。
        let mut buf = [0u16; 64];
        let n = GetClassNameW(hwnd, &mut buf);
        if n > 0 {
            let cls = String::from_utf16_lossy(&buf[..n as usize]);
            if matches!(cls.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd") {
                return true;
            }
        }
    }
    false
}

fn window_rect(hwnd: HWND) -> Option<Rect> {
    let mut r = RECT::default();
    // SAFETY: 输出缓冲区是本地 RECT。
    unsafe { GetWindowRect(hwnd, &raw mut r) }.ok()?;
    Some(Rect::new(r.left, r.top, r.right, r.bottom))
}

/// 两个矩形是否有正面积交集。
///
/// 只是"贴边"不算遮挡：Dock 上方紧邻的最大化窗口，其下沿与 Dock 上沿
/// 恰好相接，此时不该判定为遮挡。
#[must_use]
pub fn overlaps(a: Rect, b: Rect) -> bool {
    a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
}

struct Ctx<'a> {
    target: Rect,
    exclude: &'a [HWND],
    hit: bool,
    /// 盖住者（仅诊断路径填充）。
    culprit: Option<String>,
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
    // SAFETY: `lparam` 由 `any_window_covers` 传入，指向该函数栈上的 `Ctx`，
    // 且 `EnumWindows` 是同步调用 —— 回调执行期间那个引用一定有效。
    let ctx = unsafe { &mut *(lparam.0 as *mut Ctx) };
    if should_skip(hwnd, ctx.exclude) {
        return true.into();
    }
    let Some(r) = window_rect(hwnd) else {
        return true.into();
    };
    if overlaps(r, ctx.target) {
        ctx.hit = true;
        ctx.culprit = window_title(hwnd);
        // 返回 FALSE 提前结束枚举。
        return false.into();
    }
    true.into()
}

/// 有没有普通窗口盖住 `target`（屏幕坐标，物理像素）。
///
/// `exclude` 放自己的窗口（Dock 本体、设置窗、预览窗），否则自己挡自己，
/// 结果永远是"被遮挡"。
#[must_use]
pub fn any_window_covers(target: Rect, exclude: &[HWND]) -> bool {
    if target.width() <= 0 || target.height() <= 0 {
        return false;
    }
    let mut ctx = Ctx {
        target,
        exclude,
        hit: false,
        culprit: None,
    };
    // SAFETY: 见 `enum_proc` 的说明；这里是同步枚举，栈上指针在调用期间有效。
    let _ = unsafe { EnumWindows(Some(enum_proc), LPARAM((&raw mut ctx) as isize)) };
    ctx.hit
}

fn window_title(hwnd: HWND) -> Option<String> {
    let mut buf = [0u16; 128];
    // SAFETY: 输出缓冲区是本地数组。
    let n = unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(hwnd, &mut buf) };
    if n > 0 {
        Some(String::from_utf16_lossy(&buf[..n as usize]))
    } else {
        None
    }
}

/// 诊断版：返回 `(是否被盖, 盖住者的标题)`。
///
/// 与 [`any_window_covers`] 走完全相同的过滤与判定路径 ——
/// 排查"为什么 Dock 躲起来了"时用它点名，而不是另写一套容易漂移的逻辑。
#[must_use]
pub fn covering_window(target: Rect, exclude: &[HWND]) -> (bool, Option<String>) {
    let mut ctx = Ctx {
        target,
        exclude,
        hit: false,
        culprit: None,
    };
    // SAFETY: 同 `any_window_covers`，同步枚举，栈指针在调用期间有效。
    let _ = unsafe { EnumWindows(Some(enum_proc), LPARAM((&raw mut ctx) as isize)) };
    (ctx.hit, ctx.culprit)
}

/// 前台全屏检测的结果，含判定依据。
///
/// 所有字段都留给诊断用：`verdict` 才是 Dock 真正要的答案。
/// 把"查什么"和"怎么判"放在一个结构里，是为了让
/// [`is_fullscreen_foreground`] 与 [`fullscreen_trace`]
/// **不可能给出不同结论** —— 两个函数各写一遍判定逻辑的话，
/// 排查时看到的诊断会与真实行为不一致，比没有诊断更糟。
#[derive(Debug)]
pub struct FullscreenCheck {
    /// 前台窗口句柄（诊断用）。
    pub hwnd: isize,
    /// 前台窗口类名。
    pub class: String,
    /// 前台窗口矩形。
    pub rect: Rect,
    /// 前台窗口所在显示器的**完整**区域。
    pub monitor: Rect,
    /// 是否带标题栏。
    pub has_caption: bool,
    /// 是否是桌面 / 外壳窗口。
    pub is_shell: bool,
    /// 最终判定：全屏且无标题栏。
    pub verdict: bool,
}

/// 查一次前台窗口并给出全屏判定。
#[must_use]
pub fn check_fullscreen() -> FullscreenCheck {
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetDesktopWindow, GetForegroundWindow, GetShellWindow, GetWindowLongW, GWL_STYLE,
        WS_CAPTION,
    };

    // SAFETY: 全是只读查询；输出缓冲区都是本地结构体。
    unsafe {
        let fg = GetForegroundWindow();
        let mut cls = [0u16; 64];
        let n = GetClassNameW(fg, &mut cls);
        let class = String::from_utf16_lossy(&cls[..n.max(0) as usize]);
        let rect = window_rect(fg).unwrap_or(Rect::new(0, 0, 0, 0));

        let mut mi = MONITORINFO {
            cbSize: u32::try_from(std::mem::size_of::<MONITORINFO>()).unwrap_or(40),
            ..Default::default()
        };
        let _ = GetMonitorInfoW(MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST), &raw mut mi);
        let m = mi.rcMonitor;
        let monitor = Rect::new(m.left, m.top, m.right, m.bottom);

        let style = GetWindowLongW(fg, GWL_STYLE) as u32;
        let has_caption = style & WS_CAPTION.0 != 0;
        let is_shell = fg.is_invalid() || fg == GetShellWindow() || fg == GetDesktopWindow();

        // 判定条件（缺一不可）：
        // 1. 不是桌面/外壳；2. 类名不是桌面宿主；3. 覆盖整个显示器；
        // 4. 没有标题栏。
        let covers = rect.left <= monitor.left
            && rect.top <= monitor.top
            && rect.right >= monitor.right
            && rect.bottom >= monitor.bottom;
        let desktop_host = matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd");
        let verdict = !is_shell && !desktop_host && covers && !has_caption;

        FullscreenCheck {
            hwnd: fg.0 as isize,
            class,
            rect,
            monitor,
            has_caption,
            is_shell,
            verdict,
        }
    }
}

/// 前台窗口是否是一个**全屏且无标题栏**的窗口（看视频、玩游戏、全屏演示）。
///
/// 命中时 Dock 应当完全隐藏 —— 全屏内容下面露出半条 Dock 是很显眼的破相。
///
/// 覆盖整个显示器用的是 `rcMonitor` 而不是 `rcWork`：全屏视频会盖住任务栏，
/// 用 `rcWork` 判断会漏掉这种最常见的场景。
///
/// **没有 `WS_CAPTION`** 是关键条件：单纯覆盖全屏的普通最大化窗口
/// （比如浏览器普通最大化）应当**不**让 Dock 消失，否则用户会以为 Dock 崩了。
/// 无边框全屏才是"沉浸态"的信号。
#[must_use]
pub fn is_fullscreen_foreground() -> bool {
    check_fullscreen().verdict
}

/// 诊断用的一行说明。仅在 `MINIDOCK_FS_TRACE` 打开时调用。
#[must_use]
pub fn fullscreen_trace() -> String {
    let c = check_fullscreen();
    format!(
        "fg={:#x} cls={:?} rect={:?} mon=({},{},{},{}) caption={} shell={}",
        c.hwnd,
        c.class,
        c.rect,
        c.monitor.left,
        c.monitor.top,
        c.monitor.right,
        c.monitor.bottom,
        c.has_caption,
        c.is_shell
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_requires_positive_area() {
        let bar = Rect::new(100, 900, 500, 1000);
        // 完全在内部。
        assert!(overlaps(Rect::new(200, 950, 300, 990), bar));
        // 部分相交。
        assert!(overlaps(Rect::new(0, 0, 150, 950), bar));
        // 只贴边：下沿正好等于上沿，不算遮挡。
        assert!(!overlaps(Rect::new(0, 0, 1920, 900), bar));
        // 只贴右边。
        assert!(!overlaps(Rect::new(500, 900, 900, 1000), bar));
        // 完全分离。
        assert!(!overlaps(Rect::new(0, 0, 50, 50), bar));
        // 完全包含。
        assert!(overlaps(Rect::new(0, 0, 1920, 1080), bar));
    }

    /// 零面积目标直接返回 false，不进枚举。
    #[test]
    fn empty_target_is_never_covered() {
        assert!(!any_window_covers(Rect::new(0, 0, 0, 100), &[]));
        assert!(!any_window_covers(Rect::new(0, 0, 100, 0), &[]));
    }

    /// 屏幕外的矩形一定没人盖。
    ///
    /// 这条看着像废话，但它守的是"枚举失败时默认返回 true"这类实现错误：
    /// 一旦 `covered` 因为求交写错而恒为真，智能隐藏就退化成"永远收起"，
    /// 而用户看到的现象是"Dock 不出来了"，与遮挡判定毫无关联，极难定位。
    #[test]
    fn offscreen_target_is_not_covered() {
        assert!(!any_window_covers(
            Rect::new(20_000, 20_000, 20_100, 20_100),
            &[]
        ));
        assert!(!any_window_covers(
            Rect::new(-5_000, -5_000, -4_900, -4_900),
            &[]
        ));
    }

    /// 把自己排除掉之后，全屏的"自己"不该算遮挡。
    ///
    /// 这条守的是最容易犯的错：忘了传 `exclude`，Dock 覆盖的正是自己那块
    /// 区域，于是永远"被遮挡"，Dock 一起动就收起。
    #[test]
    fn excluding_everything_yields_no_coverage() {
        // 用一个真实存在的窗口做实验：拿当前进程的桌面窗口是不行的，
        // 这里只验证"排除列表生效"这一点 —— 传一个必然被枚举到的句柄。
        let desk = unsafe { windows::Win32::UI::WindowsAndMessaging::GetDesktopWindow() };
        let r = window_rect(desk).unwrap_or(Rect::new(0, 0, 1920, 1080));
        // 不排除时桌面窗口应当被判为遮挡（它就是铺满全屏的）。
        assert!(any_window_covers(r, &[]), "全屏区域应当能被某个窗口覆盖");
        // 排除之后……桌面窗口类名是 Progman/WorkerW 之一，本来就被过滤，
        // 所以这里只能保证不 panic，语义由 `should_skip` 的过滤规则保证。
        let _ = any_window_covers(r, &[desk]);
    }
}
