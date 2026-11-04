//! 弹层面板的"点外面关闭"。
//!
//! 四块面板（日历 / 控制中心 / 资源监视器 / 计时器）都由 Dock 创建，而
//! Dock 是 `WS_EX_NOACTIVATE`，从不去抢前台。**Windows 只把鼠标捕获交给
//! 前台窗口**，所以 `SetCapture` 这条路在这里是死的：面板开着、光标在别的
//! 应用上按一下，面板收不到任何消息，会一直挂在那儿。
//!
//! 于是改成**轮询**：面板可见时主循环每 [`POLL_MS`] 读一次全局按键状态，
//! 发现按下且落点不在任何面板里就收起。
//!
//! 轮询不改变事件流向 —— 那一下点击照常落到它该去的地方。点别的应用会
//! 顺带把面板收起，这正是想要的行为；顺带的好处是不需要全局钩子，也不
//! 会有"点一下只关面板、应用没反应"的憋屈感。
//!
//! 关闭行为由 `[widgets] PanelDismiss` 控制；关掉之后只能再点一次卡片
//! （或按 Esc）收起。

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetWindowRect};

/// 轮询间隔。24 ms 是"人感觉不到延迟"与"不白烧 CPU"的折中：
/// 面板开着时每秒醒 42 次，每次只读两个按键状态，量级是微秒。
pub const POLL_MS: u32 = 24;

/// 按键监视：把"按下瞬间"从持续按住里分出来。
#[derive(Debug, Default)]
pub struct Watcher {
    left_down: bool,
    right_down: bool,
}

impl Watcher {
    /// 丢掉此刻之前累积的边沿，只记"现在按着没有"。
    ///
    /// 面板刚打开时必须调一次：打开它的那一下点击还在 `GetAsyncKeyState`
    /// 的最低位里躺着，不清掉的话，下一次轮询（24 ms 后）会把"鼠标已经
    /// 离开 Dock"读成一次新的点击，面板刚出来就自己关了。
    pub fn reset(&mut self) {
        // SAFETY: 只读全局状态。
        let left = unsafe { GetAsyncKeyState(i32::from(VK_LBUTTON.0)) };
        let right = unsafe { GetAsyncKeyState(i32::from(VK_RBUTTON.0)) };
        self.left_down = left < 0;
        self.right_down = right < 0;
    }

    /// 自上次调用以来按下过鼠标左右键，返回那一刻的光标位置。
    ///
    /// 同时看两个信号：`GetAsyncKeyState` 的最低位（"自上次调用以来按下
    /// 过"）与当前是否按住。只看前者，别的线程动过这个 API 就会把边沿吃掉；
    /// 只看后者，两次轮询之间的快速点击会整个漏掉。
    pub fn poll(&mut self) -> Option<(i32, i32)> {
        // SAFETY: `GetAsyncKeyState` 只读全局状态，没有前置条件。
        let left = unsafe { GetAsyncKeyState(i32::from(VK_LBUTTON.0)) };
        let right = unsafe { GetAsyncKeyState(i32::from(VK_RBUTTON.0)) };
        let left_hit = (left & 1) != 0 || (left < 0 && !self.left_down);
        let right_hit = (right & 1) != 0 || (right < 0 && !self.right_down);
        self.left_down = left < 0;
        self.right_down = right < 0;
        if left_hit || right_hit {
            Some(cursor())
        } else {
            None
        }
    }
}

/// 光标的屏幕坐标（物理像素）。
fn cursor() -> (i32, i32) {
    let mut p = POINT::default();
    // SAFETY: 输出结构在本栈帧上。
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

/// 窗口的屏幕矩形 `(left, top, right, bottom)`，物理像素。
#[must_use]
pub fn screen_rect(hwnd: HWND) -> Option<(i32, i32, i32, i32)> {
    let mut rc = RECT::default();
    // SAFETY: `rc` 在本栈帧上；句柄由调用方保证有效。
    if unsafe { GetWindowRect(hwnd, &mut rc) }.is_err() {
        return None;
    }
    Some((rc.left, rc.top, rc.right, rc.bottom))
}

/// 点 `(x, y)` 是否落在该窗口里。
#[must_use]
pub fn hits(hwnd: HWND, x: i32, y: i32) -> bool {
    screen_rect(hwnd).is_some_and(|(l, t, r, b)| x >= l && y >= t && x < r && y < b)
}
