//! 设置窗口：Dock 进程内的第二个分层窗口。
//!
//! 为什么不是独立进程（ADR-036）：
//!
//! - **改设置要即时生效**。同一进程里 `settings_ui` 直接改的是 Dock 用的那份
//!   `Config`，下一帧就在栏体上体现出来；独立进程要么走 IPC，要么让用户重启。
//! - 本来也是进程内对话框（`DialogBox`），行为口径一致。
//! - GPU device 可以共享：第二个窗口只需要自己的 `FrameRenderer`（离屏目标
//!   + DIB），管线与字体仍在第一次付费（ADR-032 已验证过这套做法）。
//!
//! 与菜单窗口（ADR-032）的差别：
//!
//! - 菜单是**模态**的（`SetCapture` + 自己的消息循环），设置窗口是**非模态**
//!   的 —— 用户要能看着栏体的变化调参数。
//! - 因此需要 `WS_EX_APPWINDOW`：让它出现在任务栏与 Alt+Tab 里，否则窗口被
//!   别的窗口盖住之后就再也找不回来了（我们没有托盘可以兜底）。
//! - 也因此**不能**用 `WM_MOUSEACTIVATE → MA_NOACTIVATE`：颜色选择器、未来的
//!   文本输入都需要焦点。菜单不需要焦点，设置窗口需要。

use std::pin::Pin;
use std::time::{Duration, Instant};

use egui::{Align, Color32, CornerRadius, Layout, RichText};
use minidock_core::config::{Config, Key};
use minidock_settings::{settings_ui, Command, Outcome, SettingsState};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_CONTROL, VK_MENU, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetForegroundWindow, GetWindowLongPtrW, RegisterClassW,
    SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWLP_USERDATA,
    SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOW, WM_ERASEBKGND, WM_KEYDOWN, WM_KEYUP,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT, WM_RBUTTONDOWN,
    WM_RBUTTONUP, WM_SETCURSOR, WNDCLASSW, WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_TOPMOST, WS_POPUP,
};

use crate::frame::FrameRenderer;
use crate::gpu::{Gpu, GpuError};
use crate::input::{map_vk, Input};

/// 窗口类名。`minidock-platform::single_instance` 不用它，留着是给今后
/// "已经有设置窗口就激活它"用的。
pub const CLASS_NAME: &str = "MiniDockSettings";

/// 窗口逻辑尺寸（本地坐标）。宽 893 = 导航栏 193 + 内容区 700（雏形同款）。
///
/// 公开是给离屏截图用的：`examples/settings_shot.rs` 要按同一个尺寸出图，
/// 否则截出来的排版不代表真实窗口。雏形 `docs/settings-prototype.html`
/// 里写的也是这两个数。
pub const LOGICAL_SIZE: (f32, f32) = (893.0, 720.0);

/// 设置窗口的有效缩放（pixels per point）。
///
/// `UiScale` 就是“自定义 DPI”：0（以及 80 以下的杂值）= 跟随系统 DPI；
/// 80..=300 按百分比**直接生效** —— 与 Windows 自定义缩放一致，是覆盖
/// 而不是叠加：系统 150% 下选 125% 得到的是 125%，不是 187%。
///
/// 字体、按钮、行高、卡片间距全部跟着走（egui 的一切尺寸都以 point
/// 计，ppp 是唯一的缩放入口），所以一个键就能整体调大小。
#[must_use]
pub fn effective_ppp(cfg: &Config) -> f32 {
    let pct = cfg.get_i32(Key::UI_SCALE);
    if (80..=300).contains(&pct) {
        pct as f32 / 100.0
    } else {
        minidock_platform::display::system_dpi() as f32 / 96.0
    }
}

/// 设置窗口及其专属渲染器。
pub struct SettingsWindow {
    hwnd: HWND,
    renderer: FrameRenderer,
    input: Input,
    /// 屏幕左上角的物理坐标，`UpdateLayeredWindow` 需要。
    pos: (i32, i32),
    ppp: f32,
    /// UI 私有状态（当前页、显示器名、统计文本……），不属于 `Config`。
    pub state: SettingsState,
    /// 上一帧的输出。主循环取走并处理命令。
    pub outcome: Outcome,
    /// egui 最后想要的光标形状。
    ///
    /// 存下来是因为 `WM_SETCURSOR` 会在每次鼠标移动时问一遍——那条消息
    /// 里没有"egui 想要什么"的信息，只能记住上一帧的结论。原先这里
    /// 一律塞 `Default`，于是文本框上永远见不到 I 型光标。
    cursor: egui::CursorIcon,
    /// egui 说"尽快"时的最小帧间隔。
    next_repaint: Option<Instant>,
    /// Esc / 关闭按钮请求关闭。由主循环消费。
    pub close_requested: bool,
    visible: bool,
    /// 上一次设置的主题码。变了才写 visuals，避免每帧都动样式。
    ui_theme: Option<i32>,
    /// 帧耗时统计（性能页显示）。
    frame_ms: f32,
    samples: u64,
    last_stats_push: Instant,
}

impl std::fmt::Debug for SettingsWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsWindow")
            .field("hwnd", &(self.hwnd.0 as isize))
            .field("visible", &self.visible)
            .finish_non_exhaustive()
    }
}

/// 窗口销毁后清掉 `GWLP_USERDATA`。
///
/// 不清的话，窗口虽已销毁但假如还有排队的消息落到默认窗口过程——那倒没事；
/// 真正危险的是**再建一个同名的响应式窗口**时地址复用，旧指针还会看着像有效。
impl Drop for SettingsWindow {
    fn drop(&mut self) {
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
        }
    }
}

impl SettingsWindow {
    /// 建窗（默认隐藏）并准备好渲染器。
    ///
    /// 返回 `Pin<Box<Self>>` 而不是裸值：窗口过程要通过 `GWLP_USERDATA`
    /// 找回它，**地址必须稳定**。裸值从这儿返回后被 `Box`/`Option` 挪一次，
    /// 窗口过程手里就是失效地址，下一次消息就是野指针读 —— 实测表现为
    /// "点一下设置窗口进程就崩"，且没有任何 Rust panic 输出。
    ///
    /// 第一次构造会慢几十毫秒：装 CJK 字体 + 编译 wgpu 管线。
    pub fn new(gpu: &Gpu, state: SettingsState, ppp: f32) -> Result<Pin<Box<Self>>, GpuError> {
        let hinst =
            unsafe { GetModuleHandleW(None) }.map_err(|_| GpuError::Win32("GetModuleHandleW"))?;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: windows::Win32::Foundation::HINSTANCE(hinst.0),
            lpszClassName: class_pcwstr(),
            // 类光标留空 = 进入窗口时显示忙光标（ADR-045）。
            hCursor: crate::cursor::class_cursor(),
            hIcon: crate::app_icon().unwrap_or_default(),
            ..Default::default()
        };
        unsafe {
            RegisterClassW(&raw const wc);
        }

        let (pw, ph) = physical_size(ppp);
        let (x, y) = centered_origin(pw, ph);
        let hwnd = unsafe {
            CreateWindowExW(
                // WS_EX_LAYERED 是 UpdateLayeredWindow 的前提；
                // WS_EX_APPWINDOW 让它进任务栏（见模块文档）；
                // WS_EX_TOPMOST 跟随 Dock 的层级 —— 不置顶的话别的窗口一盖上来，
                // 用户既点不到它、也没有托盘图标能把它唤回来。
                WS_EX_LAYERED | WS_EX_APPWINDOW | WS_EX_TOPMOST,
                class_pcwstr(),
                windows::core::w!("MiniDock 设置"),
                WS_POPUP,
                x,
                y,
                pw,
                ph,
                None,
                None,
                Some(windows::Win32::Foundation::HINSTANCE(hinst.0)),
                None,
            )
        }
        .map_err(|_| GpuError::Win32("CreateWindowExW（设置窗口）"))?;

        let renderer = FrameRenderer::new(gpu, pw as u32, ph as u32, ppp)?;
        let mut win = Box::pin(Self {
            hwnd,
            renderer,
            input: Input::default(),
            pos: (x, y),
            ppp,
            state,
            outcome: Outcome::default(),
            cursor: egui::CursorIcon::Default,
            next_repaint: None,
            close_requested: false,
            visible: false,
            ui_theme: None,
            frame_ms: 0.0,
            samples: 0,
            last_stats_push: Instant::now(),
        });
        // 挂载必须在最终地址上做，`Pin<Box<..>>` 保证了这一点。
        win.attach();
        Ok(win)
    }

    #[must_use]
    pub const fn hwnd(&self) -> HWND {
        self.hwnd
    }

    #[must_use]
    pub const fn visible(&self) -> bool {
        self.visible
    }

    /// 把自身指针挂到 `GWLP_USERDATA`，让 `wndproc` 能找回状态。
    ///
    /// 只能由 `new` 在 `Pin<Box<Self>>` 的最终地址上调用 —— 之后再挪动整个值
    /// 就会让这里存的地址失效（见 `new` 的文档）。
    fn attach(&mut self) {
        unsafe {
            SetWindowLongPtrW(
                self.hwnd,
                GWLP_USERDATA,
                std::ptr::from_mut::<Self>(self) as isize,
            );
        }
    }

    /// 显示并激活。已经显示时只把它提到前面：重复 `SetForegroundWindow`
    /// 在同一进程内是被允许的，不会有前台锁问题。
    /// 当前生效的缩放。主循环拿它和配置算出的目标值比较，变了就 rescale。
    #[must_use]
    pub fn ppp(&self) -> f32 {
        self.ppp
    }

    /// 就地把窗口重建到新的缩放（自定义 DPI 改动时调用）。
    ///
    /// 渲染器整个换新：离屏缓冲、DIB 与 egui 上下文都按新 ppp 重建
    /// （字体图集重新上传，几十毫秒，设置改动可以接受）。窗口矩形同步
    /// 调整并重新居中 —— 逻辑尺寸不变，变的是物理尺寸。
    pub fn rescale(&mut self, gpu: &Gpu, ppp: f32) -> Result<(), GpuError> {
        if (self.ppp - ppp).abs() < f32::EPSILON {
            return Ok(());
        }
        let (pw, ph) = physical_size(ppp);
        self.renderer = FrameRenderer::new(gpu, pw as u32, ph as u32, ppp)?;
        self.ppp = ppp;
        // 渲染器换新 = egui 上下文换新（默认深色 visuals）。`ui_theme` 里
        // 还留着旧上下文的主题标记，`apply_theme` 会以为"已经应用过"而
        // 跳过 —— 浅色用户一调缩放就跳回深色。置回 None 强制下一帧重套。
        self.ui_theme = None;
        let (x, y) = centered_origin(pw, ph);
        self.pos = (x, y);
        unsafe { SetWindowPos(self.hwnd, None, x, y, pw, ph, SWP_NOACTIVATE | SWP_NOZORDER) }
            .map_err(|_| GpuError::Win32("SetWindowPos（设置窗口缩放）"))?;
        Ok(())
    }

    pub fn show(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            if GetForegroundWindow() != self.hwnd {
                let _ = SetForegroundWindow(self.hwnd);
            }
        }
        self.visible = true;
        self.next_repaint = Some(Instant::now());
    }

    pub fn hide(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.visible = false;
        self.close_requested = false;
        // 窗口藏起来就别再催并重绘了，否则每 16 ms 一帧纯浪费。
        self.next_repaint = None;
    }

    /// 到下一帧的时间。`None` = 没有待办，主循环可以阻塞等消息。
    #[must_use]
    pub fn due(&self) -> bool {
        self.visible && self.next_repaint.is_some_and(|t| t <= Instant::now())
    }

    /// 画一帧并呈现。返回 egui 想要的光标与下一次重绘延时。
    ///
    /// `cfg` 是 Dock 用的同一份；`settings_ui` 在里面改的东西下一帧就生效，
    /// 不需要任何复制或通知。
    pub fn render(&mut self, gpu: &Gpu, cfg: &mut Config) -> Result<(), GpuError> {
        let started = Instant::now();
        self.apply_theme(cfg.get_i32(Key::UiTheme));

        let raw = self.input.take(self.renderer.size(), self.ppp);
        if trace() && !raw.events.is_empty() {
            eprintln!("[settings] 交给 egui 的输入事件：{:?}", raw.events);
        }
        let pos = self.pos;
        let hwnd = self.hwnd;
        let state = &mut self.state;
        let mut outcome = Outcome::default();
        let out = self.renderer.render(gpu, raw, hwnd, pos, |ui| {
            shell(ui, state, cfg, &mut outcome);
        })?;

        self.outcome = outcome;
        if trace() && (self.outcome.changed || self.outcome.command.is_some()) {
            eprintln!(
                "[settings] UI 输出 changed={} command={:?}",
                self.outcome.changed, self.outcome.command
            );
        }
        // 只有"关闭"会让窗口消失。其余命令（增删条目、打开文件夹……）让窗口
        // 留在原地：用户加完一项就想看到栏体的变化。
        if self.outcome.command == Some(Command::Close) {
            self.close_requested = true;
        }
        self.cursor = out.cursor;
        self.cursor_to_system(out.cursor);
        self.update_stats(started.elapsed().as_secs_f32() * 1000.0);

        // egui 自己知道下一次该在什么时候画（`Duration::MAX` = 没有待办）。
        // "越快越好"给 0 —— Dock 的目标就是 60 fps，按 16 ms 排既不掉帧也不空转。
        self.next_repaint = if out.repaint_delay == Duration::MAX {
            None
        } else if out.repaint_delay.is_zero() {
            Some(Instant::now() + Duration::from_millis(16))
        } else {
            Some(Instant::now() + out.repaint_delay)
        };
        Ok(())
    }

    /// 设置窗口有自己的主题设置（`UiTheme`），与栏体主题（`DockTheme`）无关。
    ///
    /// 0 = 浅色 / 1 = 深色 / 2 = 跟随系统。只在第一次和变化时的帧调用。
    ///
    /// 用官方的 `set_theme` 切换 `ThemePreference`，而不是 `set_visuals`
    /// 覆盖当前主题的样式：后者只改 style、不改 preference，之后任何一次
    /// `set_theme`（截图工具也在用）都会突然加载另一套默认样式，主题和
    /// 视觉效果各说各话。`set_theme` 只动一个 preference 字段，比重建
    /// 整套 `Visuals` 也轻。
    fn apply_theme(&mut self, mode: i32) {
        if self.ui_theme == Some(mode) {
            return;
        }
        self.ui_theme = Some(mode);
        let dark = match mode {
            1 => true,
            2 => minidock_platform::theme::is_dark(),
            _ => false,
        };
        self.renderer.ctx().set_theme(if dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
    }

    fn update_stats(&mut self, dt: f32) {
        self.frame_ms = if self.frame_ms <= 0.0 {
            dt
        } else {
            // 指数平滑：单帧毛刺不该把读数打成异常值。
            self.frame_ms * 0.9 + dt * 0.1
        };
        self.samples += 1;
        if self.last_stats_push.elapsed() >= Duration::from_millis(400) {
            self.last_stats_push = Instant::now();
            self.state.stats = format!(
                "设置窗口 {:.2} ms/帧 · {:.0} fps · {:.1} 万次 Present",
                self.frame_ms,
                1000.0 / self.frame_ms.max(0.01),
                self.samples as f32 / 10000.0
            );
        }
    }

    /// 把 egui 的光标形状设到系统上。
    ///
    /// 映射表在 [`crate::cursor`]：这个 bug（ADR-045）就是"各处各自
    /// 维护一份映射、漏掉一个分支"的后果，不要再抄第二份。
    fn cursor_to_system(&self, icon: egui::CursorIcon) {
        crate::cursor::apply(icon);
    }

    fn on_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        if trace() {
            eprintln!("[settings-proc] msg=0x{msg:04X} wparam=0x{:X}", wparam.0);
        }
        match msg {
            WM_MOUSEMOVE => {
                let (x, y) = client_pos(lparam, self.ppp);
                let first = !self.input.hovering();
                self.input.pointer_moved(x, y);
                if first {
                    // 不订阅 TME_LEAVE 就永远收不到 WM_MOUSELEAVE，悬停状态会卡住。
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: self.hwnd,
                        dwHoverTime: 0,
                    };
                    unsafe {
                        let _ = TrackMouseEvent(&mut tme);
                    };
                }
                self.next_repaint = Some(Instant::now());
                None
            }
            WM_MOUSELEAVE => {
                self.input.pointer_gone();
                self.next_repaint = Some(Instant::now());
                None
            }
            WM_LBUTTONDOWN | WM_RBUTTONDOWN => {
                self.input.pointer_button(
                    true,
                    if msg == WM_LBUTTONDOWN {
                        egui::PointerButton::Primary
                    } else {
                        egui::PointerButton::Secondary
                    },
                );
                self.next_repaint = Some(Instant::now());
                None
            }
            WM_LBUTTONUP | WM_RBUTTONUP => {
                self.input.pointer_button(
                    false,
                    if msg == WM_LBUTTONUP {
                        egui::PointerButton::Primary
                    } else {
                        egui::PointerButton::Secondary
                    },
                );
                self.next_repaint = Some(Instant::now());
                None
            }
            WM_MOUSEWHEEL => {
                let delta = ((wparam.0 >> 16) as u16 as i16) as f32 / 120.0;
                self.input.wheel(delta);
                self.next_repaint = Some(Instant::now());
                None
            }
            WM_KEYDOWN | WM_KEYUP => {
                self.input.set_modifiers(current_modifiers());
                let vk = wparam.0 as u32;
                if let Some(key) = map_vk(vk) {
                    self.input.key(key, msg == WM_KEYDOWN);
                    if key == egui::Key::Escape && msg == WM_KEYDOWN {
                        self.close_requested = true;
                    }
                }
                self.next_repaint = Some(Instant::now());
                None
            }
            WM_SETCURSOR => {
                // 返回 TRUE：告诉系统"光标我自己管"，否则会被类光标覆盖。
                // 用上一帧 egui 的结论，不要一律箭头 —— 文本框要 I 型。
                self.cursor_to_system(self.cursor);
                Some(LRESULT(1))
            }
            WM_PAINT => {
                // 内容全部由分层窗口的合成器给，`BeginPaint/EndPaint` 只是
                // 为了让系统认为这块区域已经画过了，不再发 WM_PAINT。
                let mut ps = PAINTSTRUCT::default();
                unsafe {
                    let _ = BeginPaint(self.hwnd, &mut ps);
                    let _ = EndPaint(self.hwnd, &ps);
                }
                Some(LRESULT(0))
            }
            WM_ERASEBKGND => Some(LRESULT(1)),
            _ => None,
        }
    }
}

/// 窗口外壳：背景 + 标题栏 + 关闭按钮，中间留给 `settings_ui`。
///
/// 放在这一层而不是改 `pages.rs`：那边的 kittest 会遍历控件，多一个按钮就要
/// 改好几条断言；而对"设置窗口要不要有关闭按钮"这个问题，答案是肯定的、
/// 且与页面无关。
/// 把「标题栏 + 两列设置界面」画到任意 `Ui` 上。
///
/// 公开是因为离屏截图（`examples/settings_shot.rs`）要画**同一份**外壳 ——
/// 自己再拼一个近似版，截出来的图就不能代表真实窗口了。
pub fn shell(ui: &mut egui::Ui, state: &mut SettingsState, cfg: &mut Config, out: &mut Outcome) {
    // 主题色必须在建任何子 Ui 之前灌进 Visuals：子 Ui 继承创建时的样式。
    crate::accent::apply_visuals(ui, cfg);
    let rect = ui.max_rect();
    let dark = ui.visuals().dark_mode;
    // 250/255：圆角外侧留一点透明让边缘融进桌面，又不至于让壁纸干扰阅读。
    let bg = if dark {
        Color32::from_rgba_unmultiplied(24, 25, 30, 250)
    } else {
        Color32::from_rgba_unmultiplied(246, 247, 250, 250)
    };
    ui.painter().rect_filled(rect, CornerRadius::same(20), bg);

    let inner = rect.shrink(18.0);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::top_down(Align::Min)),
    );

    // ---- 顶栏：品牌 / 搜索（居中）/ 状态 + 图标关闭 ----
    //
    // 三块都用绝对矩形定位：搜索框要几何居中，品牌要和下方导航栏同宽
    // 并在其中居中，都不能靠 flex + margin（会随文字宽度偏）。
    let bar_h = 56.0;
    let bar = egui::Rect::from_min_size(
        child.max_rect().min,
        egui::vec2(child.max_rect().width(), bar_h),
    );
    let nav_w = minidock_settings::NAV_W;

    // Ctrl + K：聚焦搜索框（雏形的交互定义）。
    let ctrl_k = child
        .ctx()
        .input(|i| i.key_pressed(egui::Key::K) && i.modifiers.ctrl);
    // 固定 id 是刻意的：Ctrl+K 要跨帧请求同一个输入框的焦点。
    let search_id = egui::Id::unique("nav-search");
    if ctrl_k {
        child.memory_mut(|m| m.request_focus(search_id));
    }

    // ① 品牌：占导航栏同宽的一格，文字在这格正中 —— 与下方导航项对齐。
    let mut brand = child.new_child(
        egui::UiBuilder::new()
            .max_rect(egui::Rect::from_min_size(bar.min, egui::vec2(nav_w, bar_h)))
            .layout(Layout::top_down(Align::Center)),
    );
    brand.centered_and_justified(|ui| {
        ui.label(RichText::new("MiniDock").size(20.0).strong());
    });

    // ② 搜索：所有输入框共用 `minidock_settings::text_field`。
    //
    // 外壳（圆角/底色/焦点光晕/图标/快捷键/清除按钮）在共享控件里统一画，
    // 文字对齐用官方 `align(Align2::LEFT_CENTER)` —— 之前把 TextEdit 强塞
    // 进 34px 高、又用零内边距的 Frame，文字按内容高度顶格排，明显偏上。
    let search_rect = egui::Rect::from_center_size(
        bar.center(),
        egui::vec2(260.0, minidock_settings::text_field::FIELD_H),
    );
    minidock_settings::text_field::text_field(
        &mut child,
        search_rect,
        search_id,
        &mut state.nav_filter,
        &minidock_settings::text_field::FieldOpts {
            hint: "搜索设置…",
            icon: minidock_settings::text_field::FieldIcon::Search,
            trailing: Some("Ctrl K"),
            clearable: true,
        },
    );

    // ③ 右栏：状态 + 自绘关闭图标按钮（主题色 ×）。
    // right_to_left，先分配的在最右 —— 图标要贴窗口右缘。
    let right = egui::Rect::from_min_max(
        egui::pos2(bar.left() + nav_w, bar.top()),
        egui::pos2(bar.right(), bar.bottom()),
    );
    let mut side = child.new_child(
        egui::UiBuilder::new()
            .max_rect(right)
            .layout(Layout::right_to_left(Align::Center)),
    );
    let (btn, resp) = side.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::click());
    let (icon_color, hover_bg) = {
        let v = side.visuals();
        (
            v.selection.stroke.color,
            v.selection.bg_fill.linear_multiply(0.85),
        )
    };
    if resp.hovered() {
        side.painter()
            .rect_filled(btn, egui::CornerRadius::same(10), hover_bg);
    }
    let c = btn.center();
    let d = 5.0;
    let stroke = egui::Stroke::new(1.8, icon_color);
    side.painter().line_segment(
        [egui::pos2(c.x - d, c.y - d), egui::pos2(c.x + d, c.y + d)],
        stroke,
    );
    side.painter().line_segment(
        [egui::pos2(c.x + d, c.y - d), egui::pos2(c.x - d, c.y + d)],
        stroke,
    );
    // 自绘控件必须自报节点，读屏与 kittest 才查得到。
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "关闭"));
    if resp.clicked() {
        out.command = Some(Command::Close);
    }
    if !state.status.is_empty() {
        side.add_space(8.0);
        side.label(RichText::new(&state.status).small().weak());
    }

    // 子 Ui 不推进父 Ui 的光标：手动把正文起点压到顶栏之下
    child.add_space(bar_h);

    let mut page_out = settings_ui(&mut child, cfg, state);
    out.changed |= page_out.changed;
    if out.command.is_none() {
        out.command = page_out.command.take();
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut SettingsWindow;
    if ptr.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    // SAFETY: 指针由 `attach` 挂上，指向调用方持有的 `Box<SettingsWindow>`；
    // 窗口过程只在 UI 线程调用，且不与主循环的渲染阶段重叠。
    let win = unsafe { &mut *ptr };
    if let Some(result) = win.on_message(msg, wparam, lparam) {
        return result;
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// 窗口类名必须是 UTF-16 宽串，且缓冲要在进程整个存活期内有效。
///
/// 直接在 `new` 里 encode 成一个临时 `Vec<u16>` 再传给 `CreateWindowExW`
/// 是错误的：函数返回后那块内存就释放了，之后谁再用这个类名都是用野指针
/// 访问已释放的堆（本次实测就是窗口压根没创建出来）。
/// 要不要打印诊断：窗口消息流 + 交给 egui 的输入事件。
///
/// 按项目惯例挂在环境变量下（`MINIDOCK_SETTINGS_TRACE=1`）：这类输出每帧
/// 都有，常开着会把真正的错误信息淹掉。
fn trace() -> bool {
    std::env::var_os("MINIDOCK_SETTINGS_TRACE").is_some()
}

fn class_pcwstr() -> windows::core::PCWSTR {
    static BUF: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    let buf = BUF.get_or_init(|| {
        CLASS_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect()
    });
    windows::core::PCWSTR(buf.as_ptr())
}

fn physical_size(ppp: f32) -> (i32, i32) {
    (
        (LOGICAL_SIZE.0 * ppp).round() as i32,
        (LOGICAL_SIZE.1 * ppp).round() as i32,
    )
}

/// 窗口居中。用工作区而不是屏幕矩形：Dock 预留了边时，居中于屏幕会偏。
fn centered_origin(pw: i32, ph: i32) -> (i32, i32) {
    let work = minidock_platform::display::work_area(0);
    let cx = (work.left + work.right) / 2;
    let cy = (work.top + work.bottom) / 2;
    (cx - pw / 2, cy - ph / 2)
}

fn client_pos(lparam: LPARAM, ppp: f32) -> (f32, f32) {
    let x = (lparam.0 & 0xFFFF) as i16 as f32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
    (x / ppp, y / ppp)
}

fn current_modifiers() -> egui::Modifiers {
    let down = |vk: u16| unsafe { (GetKeyState(i32::from(vk)) as u16 & 0x8000) != 0 };
    egui::Modifiers {
        alt: down(VK_MENU.0),
        ctrl: down(VK_CONTROL.0),
        shift: down(VK_SHIFT.0),
        mac_cmd: false,
        command: down(VK_CONTROL.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 自定义缩放的取值规则：0 = 跟随系统；80..=300 生效；80 以下的杂值
    /// 回落系统（clamp 兜着 ini，但规则上也要兜住）。
    #[test]
    fn 自定义缩放的取值规则() {
        let mut cfg = Config::defaults(96);
        let sys = minidock_platform::display::system_dpi() as f32 / 96.0;

        // 默认 0：跟随系统
        assert!((effective_ppp(&cfg) - sys).abs() < 1e-6);

        // 生效区间：覆盖而不是叠加 —— 125% 就是 1.25
        cfg.apply_i32(Key::UI_SCALE, 125);
        assert!((effective_ppp(&cfg) - 1.25).abs() < 1e-6);
        cfg.apply_i32(Key::UI_SCALE, 300);
        assert!((effective_ppp(&cfg) - 3.0).abs() < 1e-6);

        // 80 以下：按跟随系统处理
        cfg.apply_i32(Key::UI_SCALE, 50);
        assert!((effective_ppp(&cfg) - sys).abs() < 1e-6);
    }
}
