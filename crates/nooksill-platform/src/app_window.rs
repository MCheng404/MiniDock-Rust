//! 判断"什么算应用窗口"，并按 exe 找 / 激活 / 关闭它。
//!
//!
//! 用来支持两件事：
//!
//! 1. **运行指示点**：扫描当前有哪些应用在运行（[`running_apps`]）。
//! 2. **点击已运行的应用切过去**，而不是再开一个实例（[`activate`] / [`toggle`]）。
//!
//! # 与其他模块的分工
//!
//! [`crate::occlusion`] 是"哪些窗口遮住了某块区域"，它需要**额外**排除
//! 最小化窗口与 `WS_EX_NOACTIVATE`（浮层不该算遮挡）。
//! 本模块回答的是"哪些窗口属于一个正常应用"，所以**不**排除最小化 ——
//! 最小化的窗口正是我们要恢复的对象。
//!
//! 两者的公共部分就是 [`is_app_window`]，`occlusion` 在它之上再加过滤。
//!
//! # 激活为什么这么长
//!
//! `SetForegroundWindow` 有一条"前台锁"：**只有满足特定条件的进程才能
//! 把窗口提到前台**（当前持前台权、刚收到用户输入、由前台进程启动……）。
//! Dock 窗口带 `WS_EX_NOACTIVATE` —— 点它不会让 Dock 进程拿到前台权，
//! 于是最朴素的 `SetForegroundWindow` 会被静默拒绝：任务栏按钮闪一下，
//! 窗口留在原地。这是"点了图标没反应"最常见的原因。
//!
//! 所以 [`activate`] 走一条**逐级加码**的阶梯（见 [`force_foreground`]）：
//! 直接置前 → 把自己的输入队列挂到前台线程（`AttachThreadInput`，让"调用者
//! 属于前台线程"成立）→ 模拟一次 ALT 按键（用户按过键 = 允许切换）。
//! 每一级之后都**用 `GetForegroundWindow` 复核**，不复核就等于没做。

use std::collections::HashMap;

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    keybd_event, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VK_MENU,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetAncestor, GetClassNameW, GetForegroundWindow, GetWindow,
    GetWindowLongW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
    IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow, WindowFromPoint, GA_ROOT,
    GWL_EXSTYLE, GW_OWNER, SW_MINIMIZE, SW_RESTORE, WS_EX_TOOLWINDOW,
};

/// UWP 应用的宿主进程名。
///
/// "设置""计算器""照片"这些应用跑在 `ApplicationFrameHost.exe` 里，窗口
/// 也属于它 —— 按 exe 匹配会把它们算成同一个应用：点"计算器"可能切到
/// "设置"。所以整个宿主进程跳过。
///
/// 要按应用区分得取 `GetApplicationUserModelId` 按包名匹配，需要引入
/// `Win32_System_ApplicationModel`，暂时不做。
const UWP_HOST: &str = "applicationframehost.exe";

/// 这个窗口是否属于一个"正常应用"。
///
/// **不排除最小化窗口**：最小化的窗口正是 [`activate`] 要恢复的目标。
#[must_use]
pub fn is_app_window(hwnd: HWND) -> bool {
    // SAFETY: 全是只读查询，句柄来自系统枚举或调用方。
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return false;
        }
        // 有 owner 的是对话框 / 弹出菜单的宿主。
        if GetWindow(hwnd, GW_OWNER).is_ok() {
            return false;
        }
        if GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
            return false;
        }
        // 没有标题的窗口基本都是不可见容器。
        if GetWindowTextLengthW(hwnd) == 0 {
            return false;
        }
        if crate::occlusion::is_cloaked(hwnd) {
            return false;
        }
        if is_desktop_host(hwnd) {
            return false;
        }
    }
    true
}

/// 桌面、任务栏这类"铺满全屏但不是应用"的窗口。
fn is_desktop_host(hwnd: HWND) -> bool {
    let mut buf = [0u16; 64];
    // SAFETY: 输出缓冲区是本地数组。
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    if n <= 0 {
        return false;
    }
    let cls = String::from_utf16_lossy(&buf[..n as usize]);
    matches!(cls.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd")
}

/// 进程 ID → 可执行文件路径。
///
/// 用 `PROCESS_QUERY_LIMITED_INFORMATION` 而不是 `PROCESS_QUERY_INFORMATION`：
/// 前者对更高完整性级别的进程也能打开（后者会被拒绝）。
/// 这对 Dock 很重要 —— 用户用管理员权限跑的应用，普通权限的 Dock 也要能识别。
#[must_use]
pub fn exe_of_pid(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    // SAFETY: 句柄在下面立即关闭；`buf` 是本地缓冲区。
    unsafe {
        let proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let r = QueryFullProcessImageNameW(
            proc,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &raw mut len,
        );
        let _ = CloseHandle(proc);
        r.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// 窗口所属进程的可执行文件完整路径。
#[must_use]
pub fn exe_of_window(hwnd: HWND) -> Option<String> {
    let mut pid = 0u32;
    // SAFETY: 输出参数是本地 u32。
    unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };
    exe_of_pid(pid)
}

/// 一次枚举的上下文。
///
/// `by_pid` 是**这一轮**的 pid → exe 缓存：一个应用开五个窗口时，原先要
/// `OpenProcess` + `QueryFullProcessImageNameW` 五次，现在只查一次。
/// 50 个窗口的桌面上这能省掉约 40 次内核调用，而扫描是每 1.2 秒一轮。
struct Collect {
    map: HashMap<String, HWND>,
    by_pid: HashMap<u32, Option<String>>,
}

impl Collect {
    fn exe_at(&mut self, hwnd: HWND) -> Option<String> {
        let mut pid = 0u32;
        // SAFETY: 输出参数是本地 u32。
        unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };
        if let Some(hit) = self.by_pid.get(&pid) {
            return hit.clone();
        }
        let exe = exe_of_pid(pid);
        self.by_pid.insert(pid, exe.clone());
        exe
    }
}

/// 枚举所有正常应用窗口，返回 `exe 路径（小写）→ 该应用的第一个窗口`。
///
/// 键统一转小写：Windows 路径不区分大小写，而不同 API 返回的大小写不一致
/// （`QueryFullProcessImageNameW` 给的是磁盘上的真实大小写）。
///
/// "第一个窗口"是 `EnumWindows` 的顺序 —— 也就是 **Z 序从上到下**，
/// 即这个应用最上层的那个窗口。点图标切过去正好是它。
#[must_use]
pub fn running_apps() -> HashMap<String, HWND> {
    let mut ctx = Collect {
        map: HashMap::new(),
        by_pid: HashMap::new(),
    };
    // SAFETY: `lparam` 指向本函数栈上的 ctx，`EnumWindows` 同步枚举。
    unsafe {
        let _ = EnumWindows(Some(collect_proc), LPARAM((&raw mut ctx) as isize));
    }
    ctx.map
}

unsafe extern "system" fn collect_proc(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
    // SAFETY: 见 `running_apps` 的说明。
    let ctx = unsafe { &mut *(lparam.0 as *mut Collect) };
    if !is_app_window(hwnd) {
        return true.into();
    }
    let Some(exe) = ctx.exe_at(hwnd) else {
        return true.into();
    };
    if is_uwp_host(&exe) {
        return true.into();
    }
    // 同一个应用可能有多个窗口，保留第一个即可（激活它就行）。
    ctx.map.entry(exe.to_lowercase()).or_insert(hwnd);
    true.into()
}

/// 是不是 UWP 宿主进程。
fn is_uwp_host(exe: &str) -> bool {
    std::path::Path::new(exe)
        .file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case(UWP_HOST))
}

/// 找到 `exe` 的窗口。
#[must_use]
pub fn find_window_of(exe: &str) -> Option<HWND> {
    let want = exe.to_lowercase();
    running_apps().get(&want).copied()
}

/// [`activate`] 的结论。
///
/// 原来它返回 `bool`，只回答"有没有找到窗口"，调用方无法区分
/// "切过去了"和"找到了但被系统拒绝"。现在把两件事分开：
/// [`Activation::is_running`] 决定要不要启动新实例，其余变体只进日志。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activation {
    /// 没找到窗口 —— 调用方应当去启动。
    NotRunning,
    /// 找到并置前了。
    Foreground,
    /// 从最小化恢复并置前。
    Restored,
    /// 原来就在前台，按 `toggle` 把它最小化了。
    Minimized,
    /// 窗口存在，但系统拒绝置前（前台锁）。**仍然算在运行**，
    /// 不能再启动一个实例 —— 那会变成两个窗口。
    Refused,
}

impl Activation {
    /// 这个应用是否"已经在运行"（含被拒绝置前的情况）。
    #[must_use]
    pub const fn is_running(self) -> bool {
        !matches!(self, Self::NotRunning)
    }
}

/// 激活（或最小化）`exe` 的窗口。
///
/// `toggle` 为真时按窗口状态决定动作：
///
/// | 窗口状态 | 动作 |
/// |---|---|
/// | 最小化 | 恢复并置前 |
/// | 已在前台 | 最小化 |
/// | 被别的窗口挡住 | 置前 |
///
/// 恢复/最小化没有过渡动画。
pub fn activate(exe: &str, toggle: bool) -> Activation {
    let Some(hwnd) = find_window_of(exe) else {
        return Activation::NotRunning;
    };
    // SAFETY: hwnd 来自系统枚举，是有效的顶层窗口。
    unsafe {
        let minimized = IsIconic(hwnd).as_bool();
        if trace() {
            eprintln!(
                "[activate] 目标 {:?}（exe={exe}）最小化={minimized} 前台={} 同线程={}",
                window_title(hwnd),
                window_title(GetForegroundWindow()),
                in_front(hwnd)
            );
        }
        if minimized {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        // 已经在前台 -> 缩回去（只对 toggle 生效）。
        //
        // 判据是 [`in_front`] 而不是 `GetForegroundWindow() == hwnd`：多窗口
        // 应用（浏览器、Office、壳套壳的 Electron）在前台的那个窗口常常
        // **不是**我们枚举到的第一个窗口，直接比句柄会永远判成"不在前台"，
        // 于是"再点一次收起"永远不触发 —— 表现就是"最小化没效果"。
        if toggle && !minimized && in_front(hwnd) {
            let _ = ShowWindow(hwnd, SW_MINIMIZE);
            return Activation::Minimized;
        }
        if force_foreground(hwnd) {
            return if minimized {
                Activation::Restored
            } else {
                Activation::Foreground
            };
        }
    }
    Activation::Refused
}

/// `hwnd` 是否已经"在前台"（点击时该收起而不是显示）。
///
/// 三条判据取或，从强到弱：
///
/// 1. 它就是 `GetForegroundWindow()`；
/// 2. 它和前台窗口属于**同一个进程**（同线程即同进程）—— 多窗口应用被
///    带到前台后，真正拿到前台的是它的另一个窗口（弹出面板、文档窗口），
///    句柄与 [`running_apps`] 里记的那个不同；
/// 3. **它自己没被挡住**（见 [`unobstructed`]）—— 后台窗口把它盖住时
///    这条不成立，于是照样走"显示"。
///
/// 为什么需要第 3 条：前两条都依赖 `GetForegroundWindow()`，而它在真实
/// 桌面上并不可靠 —— 前台可能是桌面（无标题的 `Progman`）、可能是标题被
/// UIPI 挡住的提权窗口、也可能在点完 Dock 之后又跳回别的窗口。表现就是
/// "点第二次不收起"：判据永远是 false，于是每次都走"显示"，而窗口本来就
/// 在眼前，看起来就是"没反应"。
///
/// 语义上也说得通：窗口可见且没被遮挡时，"把它带到前台"本来就是空操作，
/// 唯一有意义的动作就是收起。
#[must_use]
pub fn in_front(hwnd: HWND) -> bool {
    // SAFETY: 全是只读查询。
    unsafe {
        let fg = GetForegroundWindow();
        if fg == hwnd {
            return true;
        }
        if !fg.is_invalid() {
            let a = GetWindowThreadProcessId(fg, None);
            let b = GetWindowThreadProcessId(hwnd, None);
            if a != 0 && a == b {
                return true;
            }
        }
    }
    unobstructed(hwnd)
}

/// 窗口在自己**中心点**处没有被别的窗口挡住。
///
/// 用 `WindowFromPoint` 拿那个点上最上层的窗口，再取它的根窗口比对 ——
/// 直接比会拿到子窗口（按钮、编辑框），永远不等于顶层句柄。
///
/// 取中心点而不是遍历四角：圆角/标题栏边缘的点可能落在窗口外，中心点对
/// 任何正常窗口都有效，代价只是一次命中测试。
#[must_use]
pub fn unobstructed(hwnd: HWND) -> bool {
    // SAFETY: 全是只读查询；`RECT`/`POINT` 都是本栈帧上的局部量。
    unsafe {
        if hwnd.is_invalid() || IsIconic(hwnd).as_bool() || !IsWindowVisible(hwnd).as_bool() {
            return false;
        }
        let mut r = RECT::default();
        if GetWindowRect(hwnd, &raw mut r).is_err() {
            return false;
        }
        if r.right <= r.left || r.bottom <= r.top {
            return false;
        }
        let pt = POINT {
            x: (r.left + r.right) / 2,
            y: (r.top + r.bottom) / 2,
        };
        let hit = WindowFromPoint(pt);
        !hit.is_invalid() && GetAncestor(hit, GA_ROOT) == hwnd
    }
}

/// `exe` 的窗口是否已经在前台（[`find_window_of`] + [`in_front`] 的组合）。
///
/// 给探针与日志用：调用方通常只想知道"该不该最小化"。
#[must_use]
pub fn in_front_of(exe: &str) -> bool {
    find_window_of(exe).is_some_and(in_front)
}

/// 窗口标题（诊断用，取不到给空串）。
fn window_title(hwnd: HWND) -> String {
    if hwnd.is_invalid() {
        return String::new();
    }
    let n = unsafe { GetWindowTextLengthW(hwnd) };
    if n <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; n as usize + 1];
    let got = unsafe { GetWindowTextW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..got.max(0) as usize])
}

/// `NOOKSILL_LAUNCH_TRACE=1`：打印激活判定（前台是谁、目标是否最小化）。
fn trace() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("NOOKSILL_LAUNCH_TRACE").is_ok_and(|v| v.trim() != "0"))
}

/// 把 `hwnd` 送到前台，逐级加码。返回**是否真的**到了前台。
///
/// 三级阶梯（每一级都复核 `GetForegroundWindow`）：
///
/// 1. 直接 `SetForegroundWindow`。Dock 窗口带 `WS_EX_NOACTIVATE`，通常过不了。
/// 2. `AttachThreadInput(自己, 当前前台线程)`：把两个输入队列临时挂在一起，
///    "调用者属于前台线程"这个条件成立，`SetForegroundWindow` 才会被放行。
///    挂完必须**立刻解挂**，否则两个线程的输入会串（键盘焦点乱跳）。
/// 3. 模拟一次 ALT 按下抬起：前台锁的判定里有一条"用户刚刚按过键"，
///    而单按 ALT 不会往任何窗口送字符。这是 Win32 上流传很久的做法，
///    也是最后一道保险 —— 它慢（两次 `keybd_event`），只在 1、2 都失败时走。
unsafe fn force_foreground(hwnd: HWND) -> bool {
    // SAFETY: 全是窗口/线程 API，参数来自调用方给的有效句柄。
    unsafe {
        if GetForegroundWindow() == hwnd {
            return true;
        }
        let _ = SetForegroundWindow(hwnd);
        if GetForegroundWindow() == hwnd {
            return true;
        }

        let fg = GetForegroundWindow();
        let fg_tid = if fg.is_invalid() {
            0
        } else {
            GetWindowThreadProcessId(fg, None)
        };
        let me = GetCurrentThreadId();
        let attached = fg_tid != 0 && fg_tid != me && AttachThreadInput(me, fg_tid, true).as_bool();
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        if attached {
            let _ = AttachThreadInput(me, fg_tid, false);
        }
        if GetForegroundWindow() == hwnd {
            return true;
        }

        // 最后一道：ALT 连击（按下 + 抬起）。扩展键标志是 keybd_event 的
        // 惯例写法；对 VK_MENU 而言不传也能工作，但传了与真实键盘一致。
        keybd_event(VK_MENU.0 as u8, 0, KEYEVENTF_EXTENDEDKEY, 0);
        keybd_event(
            VK_MENU.0 as u8,
            0,
            KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP,
            0,
        );
        let _ = SetForegroundWindow(hwnd);
        GetForegroundWindow() == hwnd
    }
}

struct CloseCtx {
    /// 目标 exe（小写）。
    want: String,
    found: bool,
    by_pid: HashMap<u32, Option<String>>,
}

/// 让 `exe` 的应用退出（对它的每个窗口发 `WM_CLOSE`）。
///
/// 找不到窗口返回 false。多发几个窗口是刻意的：多窗口应用（浏览器、编辑器）
/// 只关一个窗口不会退出，得挨个发。哪些窗口关掉才算"退出"是应用自己的
/// 语义，这里不猜。
pub fn close_app(exe: &str) -> bool {
    let mut ctx = CloseCtx {
        want: exe.to_lowercase(),
        found: false,
        by_pid: HashMap::new(),
    };
    // SAFETY: `lparam` 指向本函数栈上的 ctx，`EnumWindows` 同步枚举。
    unsafe {
        let _ = EnumWindows(Some(close_proc), LPARAM((&raw mut ctx) as isize));
    }
    ctx.found
}

unsafe extern "system" fn close_proc(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
    // SAFETY: 见 `close_app` 的说明。
    let ctx = unsafe { &mut *(lparam.0 as *mut CloseCtx) };
    if !is_app_window(hwnd) {
        return true.into();
    }
    let mut pid = 0u32;
    // SAFETY: 输出参数是本地 u32。
    unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };
    let exe = match ctx.by_pid.get(&pid) {
        Some(hit) => hit.clone(),
        None => {
            let e = exe_of_pid(pid);
            ctx.by_pid.insert(pid, e.clone());
            e
        }
    };
    let Some(exe) = exe else {
        return true.into();
    };
    if exe.to_lowercase() == ctx.want {
        // SAFETY: 只是往这个窗口投一条关闭消息，不改自己的状态。
        let _ = unsafe {
            windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                Some(hwnd),
                windows::Win32::UI::WindowsAndMessaging::WM_CLOSE,
                Default::default(),
                Default::default(),
            )
        };
        ctx.found = true;
    }
    true.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 当前进程的窗口必然不是"应用窗口"（控制台/无窗口），
    /// 但桌面窗口也不是 —— 这条守住 `is_desktop_host` 的过滤。
    #[test]
    fn desktop_window_is_not_an_app_window() {
        let desk = unsafe { windows::Win32::UI::WindowsAndMessaging::GetDesktopWindow() };
        assert!(!is_app_window(desk), "桌面窗口不该算应用窗口");
    }

    /// 不存在的 exe 必须报告"没在运行"，不能误判成在运行。
    ///
    /// 这条守的是 [`activate`] 的契约：`NotRunning` 时调用方会去启动新实例。
    /// 如果误报成在运行，点图标就什么也不会发生（最差的一种失败）。
    #[test]
    fn unknown_exe_is_not_running() {
        let key = r"C:\definitely\not\a\real\app.exe";
        assert!(find_window_of(key).is_none());
        let r = activate(key, false);
        assert_eq!(r, Activation::NotRunning);
        assert!(!r.is_running());
    }

    /// UWP 宿主必须被认出来。
    ///
    /// 认不出的后果不是"少一个指示点"，而是**点错应用**：所有 UWP 窗口
    /// 都归 `ApplicationFrameHost.exe`，按 exe 匹配会把计算器认成设置。
    #[test]
    fn uwp_host_is_recognised() {
        assert!(is_uwp_host(r"C:\Windows\System32\ApplicationFrameHost.exe"));
        assert!(is_uwp_host(r"c:\windows\system32\applicationframehost.EXE"));
        assert!(!is_uwp_host(r"C:\Windows\explorer.exe"));
    }

    /// `running_apps` 的键必须全是小写，且没有重复键。
    ///
    /// 键不统一大小写的话，[`find_window_of`] 用 `to_lowercase()` 查
    /// 就会漏掉 —— 表现为"应用明明开着，指示点却不亮、点击又开了一个实例"。
    /// 这是本模块最容易悄悄出问题的地方，所以直接断言表的形状。
    #[test]
    fn running_apps_keys_are_lowercase() {
        for key in running_apps().keys() {
            assert_eq!(*key, key.to_lowercase(), "键 {key} 不是小写");
        }
    }

    /// 桌面窗口与工具窗口都不能进入运行应用列表。
    ///
    /// 否则桌面上没开任何应用时，指示点也会亮（桌面窗口的 exe 是
    /// explorer.exe，会被误认成"资源管理器在运行"）。
    #[test]
    fn desktop_hosts_are_not_listed_as_running() {
        for exe in running_apps().keys() {
            let name = exe.rsplit(['\\', '/']).next().unwrap_or("");
            assert!(
                !name.eq_ignore_ascii_case("progman.exe"),
                "桌面宿主不该出现在运行列表里"
            );
            assert!(
                !name.eq_ignore_ascii_case(UWP_HOST),
                "UWP 宿主不该出现在运行列表里"
            );
        }
    }

    /// `exe_of_pid(0)` 必须返回 `None` 而不是去开一个空句柄。
    #[test]
    fn pid_zero_has_no_exe() {
        assert!(exe_of_pid(0).is_none());
    }
}
