//! 中文输入法（IME）。
//!
//! 之前的做法是只收 `WM_CHAR` —— 英文能打，拼音打不出来：输入法把按键吃进
//! 自己的组合窗口，只有"上屏"那一刻才会以 `WM_CHAR` 的形式交给我们，而用户
//! 在组合过程中看到的是输入法的候选框、不是我们的输入框。实测在起动台里敲
//! "we" 会弹出一个孤零零的候选窗，搜索框毫无反应。
//!
//! 这里做三件事：
//!
//! 1. **把默认输入法上下文挂回窗口**（`ImmAssociateContextEx`）。有些窗口
//!    （尤其带 `WS_EX_NOACTIVATE`、或由宿主程序创建的）默认是没有 IME 上下文的。
//! 2. **收组合过程**：`WM_IME_COMPOSITION` 的 `GCS_COMPSTR` 是"还在打"的
//!    那串（拼音/候选），`GCS_RESULTSTR` 是"已上屏"的字。前者我们画在输入框
//!    里（带下划线），后者追加到文本里。
//! 3. **把候选窗摆到光标旁**（`ImmSetCompositionWindow`），否则它可能出现在
//!    屏幕左上角，用户根本不知道自己在哪打字。
//!
//! 注意：这是 **窗口级** 的 IME 支持，不是自绘候选列表。候选框仍然是系统的
//! 那一套（用户熟悉、词库一致），我们只负责"文字进来"和"位置正确"。

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::UI::Input::Ime::{
    ImmAssociateContextEx, ImmGetCompositionStringW, ImmGetContext, ImmReleaseContext,
    ImmSetCompositionWindow, CFS_POINT, COMPOSITIONFORM, GCS_COMPSTR, GCS_RESULTSTR, HIMC,
    IACE_DEFAULT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, WM_IME_COMPOSITION, WM_IME_ENDCOMPOSITION, WM_IME_SETCONTEXT,
    WM_IME_STARTCOMPOSITION,
};

/// 一个窗口的 IME 状态。
///
/// 只存"正在组合的那串字"，其余都是即时查询。每个要用输入法的窗口持有一个。
#[derive(Debug, Default)]
pub struct Ime {
    /// 正在组合、还没上屏的文字（拼音或候选）。
    composing: String,
}

impl Ime {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 窗口建好后调一次：确保它有输入法上下文。
    ///
    /// `IACE_DEFAULT` 的意思是"恢复默认上下文"，对绝大多数窗口是空操作；
    /// 但对被宿主程序改过上下文的窗口，这是唯一能把它救回来的调用。
    pub fn enable(hwnd: HWND) {
        unsafe {
            // 传空 HIMC + `IACE_DEFAULT` = 恢复系统默认上下文。
            let _ = ImmAssociateContextEx(hwnd, HIMC(std::ptr::null_mut()), IACE_DEFAULT);
        }
    }

    /// 正在组合的文字。UI 把它画在输入框里（建议加下划线）。
    #[must_use]
    pub fn composing(&self) -> &str {
        &self.composing
    }

    /// 有没有正在组合的文字。
    #[must_use]
    pub fn is_composing(&self) -> bool {
        !self.composing.is_empty()
    }

    /// 清掉组合态（比如输入框失焦、或按了 Esc）。
    pub fn cancel(&mut self) {
        self.composing.clear();
    }

    /// 处理一条 `WM_IME_*` 消息。
    ///
    /// 返回 `true` 表示这条消息已经处理完，调用方不要再走 `DefWindowProc`。
    /// 上屏的文字追加到 `committed`。
    pub fn handle(
        &mut self,
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        committed: &mut String,
    ) -> bool {
        match msg {
            WM_IME_STARTCOMPOSITION => {
                self.composing.clear();
                true
            }
            WM_IME_COMPOSITION => {
                // lparam 是位掩码：同一帧可能既有 RESULTSTR 又有 COMPSTR。
                let flags = lparam.0 as u32;
                if flags & GCS_RESULTSTR.0 != 0 {
                    if let Some(s) = composition_string(hwnd, GCS_RESULTSTR) {
                        committed.push_str(&s);
                    }
                    // 上屏之后组合串就空了。
                    self.composing.clear();
                }
                if flags & GCS_COMPSTR.0 != 0 {
                    self.composing = composition_string(hwnd, GCS_COMPSTR).unwrap_or_default();
                }
                true
            }
            WM_IME_ENDCOMPOSITION => {
                self.composing.clear();
                true
            }
            WM_IME_SETCONTEXT => {
                // 让系统自己画候选窗（我们不自绘候选列表）。`wparam` 表示
                // 窗口是否获得焦点，失去焦点时要把组合态清掉，否则会留下
                // 一个永远不消失的"半截拼音"。
                if wparam.0 == 0 {
                    self.composing.clear();
                }
                false
            }
            _ => false,
        }
    }

    /// 把候选窗摆到光标处（逻辑坐标）。
    ///
    /// `x` / `y` 是窗口客户区坐标（逻辑点），内部按 `ppp` 换成物理像素 ——
    /// `ImmSetCompositionWindow` 要的是物理像素。
    pub fn set_caret(&self, hwnd: HWND, x: f32, y: f32, ppp: f32) {
        let himc = unsafe { ImmGetContext(hwnd) };
        if himc.0.is_null() {
            return;
        }
        let form = COMPOSITIONFORM {
            dwStyle: CFS_POINT,
            ptCurrentPos: POINT {
                x: (x * ppp).round() as i32,
                y: (y * ppp).round() as i32,
            },
            rcArea: RECT::default(),
        };
        unsafe {
            let _ = ImmSetCompositionWindow(himc, &raw const form);
            let _ = ImmReleaseContext(hwnd, himc);
        }
    }

    /// 没被我们吃掉的消息交回系统。调用方在 `handle` 返回 `false` 时用它。
    ///
    /// 单独抽出来是因为 `WM_IME_SETCONTEXT` 必须回调默认处理，否则系统
    /// 不会显示候选窗。
    pub fn default_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> isize {
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }.0
    }
}

/// 取一段组合字符串。
///
/// `ImmGetCompositionStringW` 的协议是"先问长度、再取内容"，而且返回的是
/// **字节数**（UTF-16 所以是字符数 × 2）。长度为 0 或负数都表示没有。
fn composition_string(
    hwnd: HWND,
    kind: windows::Win32::UI::Input::Ime::IME_COMPOSITION_STRING,
) -> Option<String> {
    let himc = unsafe { ImmGetContext(hwnd) };
    if himc.0.is_null() {
        return None;
    }
    let result = (|| {
        let bytes = unsafe { ImmGetCompositionStringW(himc, kind, None, 0) };
        if bytes <= 0 {
            return None;
        }
        let mut buf = vec![0_u16; bytes as usize / 2 + 1];
        let got = unsafe {
            ImmGetCompositionStringW(
                himc,
                kind,
                Some(buf.as_mut_ptr().cast()),
                (buf.len() * 2) as u32,
            )
        };
        if got <= 0 {
            return None;
        }
        buf.truncate(got as usize / 2);
        Some(String::from_utf16_lossy(&buf))
    })();
    unsafe {
        let _ = ImmReleaseContext(hwnd, himc);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 新建时没有组合串。
    #[test]
    fn starts_empty() {
        let ime = Ime::new();
        assert!(!ime.is_composing());
        assert_eq!(ime.composing(), "");
    }

    /// 组合开始 / 结束的消息要吃掉，并且清空组合串。
    ///
    /// 用空 HWND 跑：这两条分支不碰 HIMC，正好可以脱离真实输入法验证状态机。
    #[test]
    fn start_and_end_composition_are_handled() {
        let mut ime = Ime::new();
        let mut committed = String::new();
        let null = HWND(std::ptr::null_mut());
        assert!(ime.handle(
            null,
            WM_IME_STARTCOMPOSITION,
            WPARAM(0),
            LPARAM(0),
            &mut committed
        ));
        assert!(!ime.is_composing());
        assert!(ime.handle(
            null,
            WM_IME_ENDCOMPOSITION,
            WPARAM(0),
            LPARAM(0),
            &mut committed
        ));
        assert!(!ime.is_composing());
        assert!(committed.is_empty(), "组合消息不该产生上屏文字");
    }

    /// `WM_IME_SETCONTEXT` 必须交回默认处理（否则系统不画候选窗），
    /// 但失焦时要顺手清掉组合串 —— 否则会留下一条永远不消失的拼音。
    #[test]
    fn setcontext_is_passed_through_and_clears_on_blur() {
        let mut ime = Ime::new();
        ime.composing = "nihao".to_owned();
        let mut committed = String::new();
        let null = HWND(std::ptr::null_mut());
        let handled = ime.handle(
            null,
            WM_IME_SETCONTEXT,
            WPARAM(0),
            LPARAM(0),
            &mut committed,
        );
        assert!(!handled, "要交回 DefWindowProc");
        assert!(!ime.is_composing(), "失焦应当清掉组合串");
    }

    /// 取消会清空组合串。
    #[test]
    fn cancel_clears_composition() {
        let mut ime = Ime::new();
        // 直接写字段：`composing` 是私有的，测试在同模块里可以碰。
        ime.composing = "nihao".to_owned();
        assert!(ime.is_composing());
        ime.cancel();
        assert!(!ime.is_composing());
    }
}
