//! M1 的 Dock 窗口：真实 Dock 形态（不是设置窗）。
//!
//! 复用 M0 的呈现管线（离屏渲染 → 回读 → `UpdateLayeredWindow`），
//! 把画面换成 [`minidock_gpu::dock::Dock`] 的绘制。管线本身在
//! [`minidock_gpu::frame::FrameRenderer`] 里，与设置窗口共用同一份实现。
//!
//! 用法：
//!
//! ```text
//! cargo run --release -p minidock-gpu --bin minidock-dock
//! ```
//!
//! 按 Esc 退出。鼠标移上去会触发悬停放大与标签气泡。

use std::time::{Duration, Instant};

use minidock_core::clock;
use minidock_core::clock::ClockLine;
use minidock_core::config::{Config, DockMode, Edge, Key};
use minidock_core::icon::FitMode;
use minidock_core::ini::Ini;
use minidock_core::item::{Item, ItemKind};
use minidock_core::layout;
use minidock_core::layout::Rect as LRect;
use minidock_core::running;
use minidock_core::store;
use minidock_gpu::dock::{Dock, DockItem};
use minidock_gpu::frame::FrameRenderer;
use minidock_gpu::gpu::Gpu;
use minidock_gpu::menu::{Menu, MenuCmd};
use minidock_gpu::settings_win::SettingsWindow;
use minidock_platform::app_window;
use minidock_platform::appbar::AppBar;
use minidock_platform::display;
use minidock_platform::icon::{self, IconBitmap};
use minidock_platform::launch;
use minidock_platform::occlusion;
use minidock_platform::shortcut;
use minidock_platform::single_instance::SingleInstance;
use minidock_platform::theme;
use minidock_platform::Paths;
use minidock_settings::{Command, SettingsState};
use std::pin::Pin;
use windows::core::w;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, ScreenToClient, UpdateWindow, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, GetKeyState, RegisterHotKey, ReleaseCapture, SetCapture, TrackMouseEvent,
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, TME_LEAVE, TRACKMOUSEEVENT, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetCursorPos, GetWindowLongPtrW,
    GetWindowRect, MsgWaitForMultipleObjectsEx, PeekMessageW, PostQuitMessage, RegisterClassW,
    SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage,
    GWLP_USERDATA, HWND_TOPMOST, MA_NOACTIVATE, MSG, MWMO_INPUTAVAILABLE, PM_REMOVE, QS_ALLINPUT,
    SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOW, SW_SHOWNA, WM_CAPTURECHANGED, WM_DESTROY,
    WM_ERASEBKGND, WM_HOTKEY, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONUP,
    WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_PAINT, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP,
    WM_SETTINGCHANGE, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    match App::boot()? {
        // `None` = 已有实例在跑。这是正常退出，不是错误。
        None => Ok(()),
        Some(mut app) => app.run(),
    }
}

// ============================ 调试开关 ============================

/// `MINIDOCK_*` 调试开关。**启动时读一次**，之后只读缓存。
///
/// 原先这些判断散在主循环里共 16 处 `std::env::var_os("MINIDOCK_GEOM")`，
/// 每帧各锁一次进程环境块再线性扫描一遍字符串。60 fps 下是白花的开销，
/// 而且 16 份重复的魔法字符串里拼错一个，那个开关就静默失效 —— 排查时
/// 最难受的一类问题。
mod trace {
    use std::sync::LazyLock;

    fn flag(name: &'static str) -> bool {
        std::env::var_os(name).is_some()
    }

    /// 几何 / 重排 / 点击 / 扫描的详细日志，用于与 C 版逐项对拍。
    pub fn geom() -> bool {
        static V: LazyLock<bool> = LazyLock::new(|| flag("MINIDOCK_GEOM"));
        *V
    }

    /// 全屏检测的判定过程。
    pub fn fs() -> bool {
        static V: LazyLock<bool> = LazyLock::new(|| flag("MINIDOCK_FS_TRACE"));
        *V
    }

    /// 菜单窗口的消息流。
    pub fn menu() -> bool {
        static V: LazyLock<bool> = LazyLock::new(|| flag("MINIDOCK_MENU_TRACE"));
        *V
    }
}

// ============================ 应用状态 ============================

/// 一次 Dock 会话的全部可变状态。
///
/// 以前这些是 `main` 里的 16 个局部变量，循环体一路写到 500 多行；
/// 后来每加一个功能只能按 `②.52` / `②.535` 这种编号插进去，编号本身
/// 已经重复（两个 `②.1`、两个 `②.4`）。收进结构体之后主循环变成一串
/// 有名字的步骤，每一步都能单独读、单独改、单独加。
struct App {
    hwnd: HWND,
    gpu: Gpu,
    frame: FrameRenderer,
    /// 必须在堆上：`wndproc` 通过 `GWLP_USERDATA` 存的是它的地址，
    /// `Box` 的内容不会因为 `App` 本身移动而搬家。
    dock: Box<Dock>,
    paths: Paths,
    /// 完整条目（含启动路径与 `.lnk` 解析结果）。`dock.items` 只有显示用的
    /// 子集，两份必须同步，否则"点了图标启动的是别人"。
    raw: Vec<Item>,
    dpi: i32,
    ppp: f32,
    /// 目标显示器的工作区（物理像素），C 版用 `g_mi.rcWork`。
    work: LRect,
    appbar: AppBar,
    /// AppBar 申请到的预留矩形；`None` = 没有预留（或申请失败）。
    reserved: Option<LRect>,
    /// 窗口左上角的屏幕坐标（物理像素）。
    pos: (i32, i32),
    /// 窗口尺寸（物理像素）。
    size: (i32, i32),
    menu_slot: MenuSlot,
    settings_slot: Option<Pin<Box<SettingsWindow>>>,
    /// 配置改动到写盘的防抖起点（`None` = 没有待写内容）。
    cfg_dirty_at: Option<Instant>,
    last_fs_check: Instant,
    fs_hidden: bool,
    last_scan: Instant,
    /// `MINIDOCK_FAKE_MOUSE="u,v"`：把光标钉在指定位置（无人值守截图用）。
    fake_mouse: Option<(f32, f32)>,
    /// `MINIDOCK_FAKE_COVERED=0/1`：覆盖遮挡判定结果。
    fake_covered: Option<bool>,
    start: Instant,
    /// 单实例互斥体。**必须是字段**：它是 RAII 锁，放成 `boot` 的局部变量
    /// 会在 `boot` 返回时立刻释放，第二个实例就能起来了。
    _guard: Option<SingleInstance>,
}

/// 消息泵的结论。
enum Pumped {
    /// 处理过消息，需要重绘。
    Frames,
    /// 队列已空。
    Idle,
    /// 收到 `WM_QUIT`。
    Quit,
}

impl App {
    /// 建窗口、装资源，直到可以进消息循环为止。
    ///
    /// `Ok(None)` 表示已有实例在跑 —— 调用方应当正常退出，不是报错。
    fn boot() -> Result<Option<Self>, Box<dyn std::error::Error>> {
        // DPI 感知必须在建窗之前（M0 的教训：未声明时 Windows 会拉伸窗口
        // 位图，表面尺寸与客户区错位）。
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }

        // 单实例。**Dock 也必须接这个**：两个 Dock 会叠出两个分层窗口，
        // 而且 `FindWindow` 只会抓到一个，外部工具/测试会操作错的那个。
        // 互斥体本身失败时不阻止启动 —— 宁可多一份 Dock，也不要起不来。
        let guard = match SingleInstance::acquire() {
            Ok(Some(guard)) => Some(guard),
            Ok(None) => {
                eprintln!("MiniDock 已在运行，本次启动退出。");
                return Ok(None);
            }
            Err(e) => {
                eprintln!("单实例互斥体失败：{e}（继续启动）");
                None
            }
        };

        let paths = Paths::resolve()?;
        let dpi = display::system_dpi();
        let mut ini = Ini::load(&paths.ini)?;
        // 上游格式就地升级：读一次就变成新格式，用户不需要知道有这回事。
        // 只在这一步写盘 —— 迁移失败也不影响本次运行（`from_ini` 两种格式都认）。
        if Config::upgrade_legacy(&mut ini, dpi) {
            match ini.save(&paths.ini) {
                Ok(()) => eprintln!(
                    "配置已从上游格式升级到 v{}（各设置保持不变）。",
                    minidock_core::config::CONFIG_VERSION
                ),
                Err(e) => eprintln!("配置升级后保存失败（本次仍按新格式使用）：{e}"),
            }
        }
        let cfg = Config::from_ini(&ini, dpi);
        // 可变：拖拽重排要跟着 Dock 一起调整，顺序才与屏幕一致。
        let mut raw: Vec<Item> = store::load_items(&ini);
        let items: Vec<DockItem> = raw.iter().map(DockItem::from_item).collect();

        let work: LRect = display::work_area(0);
        let mut dock = Box::new(Dock::new(cfg, items, work));
        dock.mode = dock.cfg.mode;
        dock.update(dpi);

        let (win_w, win_h) = dock.physical_size_i32();
        dump_geometry(&dock, dpi);
        let ppp = dpi as f32 / 96.0;

        let hinstance = unsafe { GetModuleHandleW(None) }?;
        let wc = WNDCLASSW {
            hInstance: HINSTANCE(hinstance.0),
            lpfnWndProc: Some(wndproc),
            lpszClassName: w!("MiniDockWindow"),
            ..Default::default()
        };
        unsafe {
            RegisterClassW(&wc);
        }

        // 先按"没有预留区"的位置建窗：AppBar 的 `ABM_NEW` 需要 hwnd，
        // 而位置又要用到 AppBar 拿回来的矩形 —— 顺序上只能先建后挪。
        // C 版也是这么做的（`UpdateGeometry` 里先建后 `SetWindowPos`）。
        let (mut x, mut y) = layout::window_origin(dock.cfg.edge, work, None, win_w, win_h);
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                w!("MiniDockWindow"),
                w!("MiniDock"),
                WS_POPUP,
                x,
                y,
                win_w,
                win_h,
                None,
                None,
                Some(hinstance.into()),
                None,
            )?
        };

        // 预留桌面空间。**只在常显模式下有意义**：自动隐藏/Dock 会滑走，
        // 留着那块空白反而更怪（C 版同样是 `cfg.reserve && cfg.mode == 0`）。
        let mut appbar = AppBar::new(hwnd);
        let reserved = if dock.cfg.reserve && dock.mode == DockMode::Always {
            appbar.apply(
                dock.cfg.edge,
                dock.reserve_thickness(),
                display::monitor_rect(0),
            )
        } else {
            None
        };
        (x, y) = layout::window_origin(dock.cfg.edge, work, reserved, win_w, win_h);
        if reserved.is_some() {
            // 申请成功才移动窗口：失败时保持原位置，宁可少预留也不要跑到屏幕外。
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    x,
                    y,
                    win_w,
                    win_h,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
        if dock.cfg.reserve && dock.mode != DockMode::Always {
            // 不预留在隐藏模式下是**故意的**（Dock 会滑走，留一块空白更怪），
            // 但用户既然开了开关就该知道它没生效，否则会当成 bug 报上来。
            eprintln!("`Reserve` 只在常显模式（Mode=0）下生效，当前已忽略。");
        }
        if trace::geom() {
            eprintln!(
                "[layout] edge={:?} reserve={} -> pos=({x},{y}) reserved={reserved:?}",
                dock.cfg.edge, dock.cfg.reserve
            );
        }

        let gpu = Gpu::new("")?;
        let frame = FrameRenderer::new(&gpu, win_w as u32, win_h as u32, ppp)?;
        let start = Instant::now();

        // 先把 `.lnk` 解析成真实 exe 路径：判断"是否在运行"和"点击该切过去"
        // 都要靠它。解析有 COM 开销，所以只在启动时做一次，结果缓存在
        // `Item.target` 里（不写回 ini，那是运行时缓存）。
        resolve_targets(&mut raw);
        // 初次扫描：让指示点在第一帧就正确，而不是等 1.2 秒后才亮。
        refresh_running(&mut raw, &mut dock);

        // 图标在 egui 上下文就绪之后装：纹理要 ctx 才能建。
        load_icons(&mut dock, frame.ctx(), &raw);

        // 全局快捷键 `Ctrl+Alt+D`：强制收起 / 恢复。原版用 `cfg.hotkey` 开关。
        // 注册失败（被别的程序占了）只警告，不影响启动。
        if dock.cfg.hotkey {
            let ok = unsafe {
                RegisterHotKey(
                    Some(hwnd),
                    1,
                    HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_ALT.0),
                    u32::from(b'D'),
                )
            };
            if ok.is_err() {
                eprintln!("快捷键 Ctrl+Alt+D 注册失败（可能被占用），强制收起不可用。");
            }
        }

        let mut app = Self {
            hwnd,
            gpu,
            frame,
            dock,
            paths,
            raw,
            dpi,
            ppp,
            work,
            appbar,
            reserved,
            pos: (x, y),
            size: (win_w, win_h),
            menu_slot: None,
            settings_slot: None,
            cfg_dirty_at: None,
            last_fs_check: Instant::now(),
            fs_hidden: false,
            last_scan: Instant::now(),
            fake_mouse: fake_mouse_from_env(),
            fake_covered: fake_covered_from_env(),
            start,
            _guard: guard,
        };

        // Dock 指针挂到窗口上，供 wndproc 回传鼠标位置。
        // 取的是 `Box` 里那块堆内存的地址，与 `App` 自身是否再移动无关。
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, (&raw mut *app.dock) as isize);
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
            let _ = UpdateWindow(hwnd);
        }
        Ok(Some(app))
    }

    /// 消息循环。只在 `WM_QUIT` 时返回。
    ///
    /// 每一步返回"本轮是否需要出帧"。**顺序有意义**：菜单是模态的（会阻塞），
    /// 必须排在点击之后、设置窗口之前；写盘放在设置之后，否则当轮改动还没
    /// 落到 `cfg` 里。
    fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut msg = MSG::default();
        loop {
            let mut need_frame = false;

            // ① 抽干消息队列：一条消息一帧会造成抖动。
            match pump(&mut msg) {
                Pumped::Quit => return Ok(()),
                Pumped::Frames => need_frame = true,
                Pumped::Idle => {}
            }

            // ② 重布局。**两个条件缺一不可**：`dirty`（鼠标进出/按下抬起
            //    改变了输入状态，必须重算 `hot`）与 `anim`（放大曲线还在走）。
            //    早先只写了 `anim`，结果动画一收敛鼠标再进来也不会放大 ——
            //    因为 `hot` 只在 `update` 里算，而 `update` 不再被调用。
            need_frame |= self.step_layout();
            // ③ 窗口尺寸/位置跟着条目数走。
            need_frame |= self.step_relayout()?;
            // ④ 拖拽重排同步到配置副本并写盘。
            self.step_reorder();
            // ⑤ 点击启动（`ShellExecuteW` 可能阻塞，不能放在窗口过程里）。
            self.step_click();
            // ⑥ 右键菜单（模态）。
            need_frame |= self.step_menu()?;
            // ⑦ 设置窗口（非模态）。
            need_frame |= self.step_settings()?;
            // ⑧ 配置落盘（防抖）。
            self.step_save_config();
            // ⑨ 全屏时整体隐藏。
            need_frame |= self.step_fullscreen();
            // ⑩ 运行状态扫描。
            need_frame |= self.step_running_scan();
            // ⑪ 自动隐藏 / 智能隐藏。
            need_frame |= self.step_auto_hide();

            if need_frame || self.dock.anim {
                self.render()?;
            }
            self.wait();
        }
    }

    /// 推进一帧布局。返回是否需要出帧。
    fn step_layout(&mut self) -> bool {
        // 无人值守截图用：`MINIDOCK_FAKE_MOUSE="u,v"` 把鼠标钉在指定位置。
        // 自动化验证时物理光标会被用户挪走，靠它才能稳定复现悬停态。
        if let Some((u, v)) = self.fake_mouse {
            self.dock.mouse = (u, v);
            self.dock.hover = true;
        }
        if !self.dock.take_dirty() && !self.dock.anim {
            return false;
        }
        self.dock.update(self.dpi);
        true
    }

    /// 配置热更新：条目数变了 → 栏体宽度变了 → 窗口尺寸与位置都要改。
    ///
    /// 这条路径的风险在于离屏纹理与 DIB 必须跟着换成新尺寸，否则
    /// `UpdateLayeredWindow` 提交的还是一块旧尺寸的位图 —— 表现为
    /// "只有左上角一部分被画出来"。
    fn step_relayout(&mut self) -> Result<bool, Box<dyn std::error::Error>> {
        let (nw, nh) = self.dock.physical_size_i32();
        if nw == self.size.0 && nh == self.size.1 {
            return Ok(false);
        }
        self.size = (nw, nh);
        self.frame.resize(&self.gpu, nw as u32, nh as u32)?;
        self.reposition();
        if trace::geom() {
            eprintln!(
                "[relayout] 窗口 -> ({},{}) {nw}x{nh}",
                self.pos.0, self.pos.1
            );
        }
        Ok(true)
    }

    /// 把拖拽产生的重排同步到配置副本，并写回 ini。
    ///
    /// 不需要额外出帧：`move_item` 会把 `moved` 置真，下一轮 `update`
    /// 据此补位移动画，而 `update` 自身就会要求出帧。
    fn step_reorder(&mut self) {
        while let Some((from, to)) = self.dock.take_pending_move() {
            if from < self.raw.len() && to < self.raw.len() {
                let moved = self.raw.remove(from);
                self.raw.insert(to, moved);
            }
        }
        if self.dock.take_order_changed() {
            if let Err(e) = save_order(&self.paths, &self.raw) {
                eprintln!("保存条目顺序失败：{e}");
            }
        }
    }

    /// 点击启动。放在主循环而不是窗口过程里：`ShellExecuteW` 可能阻塞
    /// 几十毫秒，不该占着消息处理。
    fn step_click(&mut self) {
        let Some((i, force)) = self.dock.take_click() else {
            return;
        };
        if trace::geom() {
            eprintln!(
                "[click] 第 {i} 项 force={force} mouse={:?} slot_l={:?}",
                self.dock.mouse, self.dock.geom.slot_l
            );
        }
        // 图标数与配置不一致时不要静默吞掉，这类错配要能看见。
        let Some(item) = self.raw.get(i) else {
            eprintln!("点击了第 {i} 项，但配置里没有对应条目");
            return;
        };

        // `force`（Shift+点击 / 中键）= 不管有没有在跑，都新起一个。
        // 原版 `LaunchItem(i, force)` 的同一个开关：开了 force 就连
        // "激活已有窗口"都跳过 —— 否则想开第二个实例永远开不出来。
        let toggle = self.dock.cfg.get_i32(Key::CLICK_TOGGLE) != 0;
        let switched = !force
            && running::running_key(item).is_some_and(|exe| app_window::activate(&exe, toggle));
        if switched {
            if trace::geom() {
                eprintln!("[click] 切换到已运行实例：{}", item.name);
            }
        } else if let Err(e) = launch::launch(item) {
            eprintln!("启动「{}」失败：{e}", item.name);
        }
        // 启动/切换之后弹一下，给一个"点到了"的反馈。
        // 弹跳只在真的发生了动作时播，失败不弹（否则会误导）。
        self.dock.start_bounce(i);
    }

    /// 右键菜单。**模态**：会阻塞到用户选择或取消。
    fn step_menu(&mut self) -> Result<bool, Box<dyn std::error::Error>> {
        let Some((u, v)) = self.dock.take_menu_request() else {
            return Ok(false);
        };
        let hit = self.dock.hit_at(u, v);
        let target = hit.and_then(|i| self.raw.get(i));
        let running = target.is_some_and(|it| it.running);
        let Some(menu) = Menu::build(target, running) else {
            return Ok(false);
        };

        // 光标位置决定菜单弹出点；工作区边界决定往哪边翻。
        let mut pt = POINT::default();
        let _ = unsafe { GetCursorPos(&raw mut pt) };
        let cursor = (pt.x as f32 / self.ppp, pt.y as f32 / self.ppp);
        // 边界用**工作区**而不是整块屏幕：菜单不该压到任务栏上。
        let w = self.work;
        let bounds = (
            w.left as f32 / self.ppp,
            w.top as f32 / self.ppp,
            w.right as f32 / self.ppp,
            w.bottom as f32 / self.ppp,
        );
        let dark = self.dock.cfg.dock_is_dark(theme::is_dark());
        let cmd = match show_menu(
            &self.gpu,
            &mut self.menu_slot,
            menu,
            cursor,
            bounds,
            dark,
            self.ppp,
            self.start,
        ) {
            Ok(cmd) => cmd,
            Err(e) => {
                eprintln!("菜单显示失败：{e}");
                None
            }
        };

        if let Some(cmd) = cmd {
            if trace::geom() {
                eprintln!(
                    "[menu] 命中 hit={hit:?} name={:?} cmd={cmd:?}",
                    hit.and_then(|i| self.raw.get(i)).map(|it| it.name.clone())
                );
            }
            if cmd == MenuCmd::Settings {
                // 设置窗口要激活另一个 HWND，不能放进 `run_menu_cmd`：
                // 那儿没有 `gpu`，也不该有。
                let sw =
                    ensure_settings(&self.gpu, &mut self.settings_slot, &self.paths, self.ppp)?;
                // 每次打开都重读：用户可能刚在任务管理器里改过它。
                sw.state.auto_start = theme::get_autostart();
                sw.show();
            } else {
                run_menu_cmd(cmd, hit, &mut self.raw, &mut self.dock, &self.paths);
            }
        }

        // 菜单期间 Dock 没有重绘，回来后强制出帧并刷新悬停。
        self.dock.mark_dirty();
        // 取消悬停：光标多半已经不在图标上了。
        self.dock.hover = false;
        self.dock.mouse = (-1.0, -1.0);
        Ok(true)
    }

    /// 设置窗口。**非模态**：改参数时要能一边拖动滑块一边看栏体的变化，
    /// 所以不像菜单那样把整条消息循环堵住。
    fn step_settings(&mut self) -> Result<bool, Box<dyn std::error::Error>> {
        let (changed, remount) = tick_settings(
            &self.gpu,
            &mut self.settings_slot,
            &mut self.dock,
            &mut self.raw,
            &self.paths,
            self.dpi,
        )?;
        let mut need_frame = false;

        if changed {
            // 配置是 Dock 与设置窗口共用的同一份：`update()` 在下一轮用它
            // 重算几何，这里只需要要求出帧 + 排一次写盘。
            self.cfg_dirty_at = Some(Instant::now());
            need_frame = true;
            self.dock.mark_dirty();
            if trace::geom() {
                eprintln!("[settings] 配置有改动，排定写盘");
            }
        }

        if remount {
            // 停靠边 / 预留 / 隐藏模式变了：AppBar 得重新注册，窗口位置
            // 得跟着新边算。尺寸若也变了（左右停靠要转置 lw/lh），下一轮
            // `step_relayout` 会 resize —— 这里先按旧尺寸摆到新位置，差的是一帧。
            self.dock.mode = self.dock.cfg.mode;
            self.reserved = if self.dock.cfg.reserve && self.dock.mode == DockMode::Always {
                self.appbar.apply(
                    self.dock.cfg.edge,
                    self.dock.reserve_thickness(),
                    display::monitor_rect(0),
                )
            } else {
                self.appbar.remove();
                None
            };
            self.reposition();
            if trace::geom() {
                eprintln!(
                    "[settings] 重挂 edge={:?} reserve={} mode={:?} -> pos=({},{}) reserved={:?}",
                    self.dock.cfg.edge,
                    self.dock.cfg.reserve,
                    self.dock.mode,
                    self.pos.0,
                    self.pos.1,
                    self.reserved
                );
            }
            need_frame = true;
        }

        Ok(need_frame)
    }

    /// 配置落盘。滑块一次拖动会连着产生几十次 `changed`，每条都写一次
    /// 等于每帧一次 UTF-16 全量回写，所以走 400 ms 防抖。
    fn step_save_config(&mut self) {
        let Some(at) = self.cfg_dirty_at else {
            return;
        };
        if at.elapsed() < Duration::from_millis(CFG_SAVE_DEBOUNCE_MS) {
            return;
        }
        self.cfg_dirty_at = None;
        if let Err(e) = save_config(&self.paths, &self.dock.cfg) {
            eprintln!("保存配置失败（改动仍在本进程内生效）：{e}");
            return;
        }
        settings_status(&mut self.settings_slot, "已保存");
        if trace::geom() {
            eprintln!("[settings] 已写回 {}", self.paths.ini.display());
        }
    }

    /// 前台是沉浸式全屏（视频/游戏）时整体隐藏 Dock。
    ///
    /// 一秒查一次足够 —— 从全屏切回来到 Dock 重新出现，1 秒内感知不到延迟，
    /// 而检查本身只是两个只读查询，不触发任何渲染。
    fn step_fullscreen(&mut self) -> bool {
        if self.last_fs_check.elapsed() < Duration::from_millis(u64::from(FS_CHECK_MS)) {
            return false;
        }
        self.last_fs_check = Instant::now();
        let fs = occlusion::is_fullscreen_foreground();
        if trace::fs() {
            eprintln!("[fs] {} -> {}", occlusion::fullscreen_trace(), fs);
        }
        if fs == self.fs_hidden {
            return false;
        }
        self.fs_hidden = fs;
        unsafe {
            let _ = ShowWindow(self.hwnd, if fs { SW_HIDE } else { SW_SHOWNA });
        }
        if trace::geom() {
            eprintln!(
                "[fullscreen] {}",
                if fs { "隐藏 Dock" } else { "恢复 Dock" }
            );
        }
        // 显示回来时立刻画一帧，否则会白屏到下一次事件。
        !fs
    }

    /// 扫描运行中的应用并刷新指示点。
    ///
    /// 原版只在"要画指示点 / 显示运行应用 / 最小化动画"时才扫；我们把它当作
    /// "点击能否切到已运行实例"的前提，所以只要有条目就扫（一次约 1 ms，
    /// 1.2 s 一轮可以忽略）。
    fn step_running_scan(&mut self) -> bool {
        if self.raw.is_empty()
            || self.last_scan.elapsed() < Duration::from_millis(u64::from(Dock::SCAN_INTERVAL_MS))
        {
            return false;
        }
        self.last_scan = Instant::now();
        refresh_running(&mut self.raw, &mut self.dock)
    }

    /// 自动隐藏 / 智能隐藏。**必须轮询光标**，不能只靠窗口消息：收起后
    /// 窗口内容被推到可视区外，那片区域的 alpha 是 0，鼠标事件会直接穿透，
    /// 永远等不到 `WM_MOUSEMOVE`。
    fn step_auto_hide(&mut self) -> bool {
        if !self.dock.needs_hide_tick() {
            return false;
        }
        let now_ms = u64::try_from(self.start.elapsed().as_millis()).unwrap_or(0);
        let near = cursor_near(&self.dock, self.hwnd);
        let busy = self.dock.dragging().is_some();
        // 智能隐藏才需要枚举窗口。按需调用：`EnumWindows` 要遍历桌面上
        // 所有顶层窗口，100 ms 一次也吃不住白跑的代价。
        // `MINIDOCK_FAKE_COVERED=0/1` 覆盖遮挡判定结果：桌面环境不可控，
        // 要验证"遮挡消失就展开"这条分支不能真去最小化别人的窗口。
        let covered = self.dock.mode == DockMode::SmartHide
            && self
                .fake_covered
                .unwrap_or_else(|| dock_bar_covered(&self.dock, self.hwnd, self.pos.0, self.pos.1));
        let before = self.dock.hide_target;
        self.dock.tick_auto_hide(near, busy, covered, now_ms);
        if before == self.dock.hide_target {
            return false;
        }
        if trace::geom() {
            // 智能隐藏时点名盖住者："为什么躲起来"要能一眼看出，
            // 否则只能对着一个 bool 猜是哪个窗口。
            let by = if self.dock.mode == DockMode::SmartHide {
                let left = self.dock.geom.bar_l.round() as i32;
                let top = self.dock.geom.bar_top.round() as i32;
                let right = (self.dock.geom.bar_l + self.dock.geom.bar_w).round() as i32;
                let bottom = (self.dock.geom.bar_bottom + self.dock.metrics.margin).round() as i32;
                occlusion::covering_window(
                    LRect::new(
                        self.pos.0 + left,
                        self.pos.1 + top,
                        self.pos.0 + right,
                        self.pos.1 + bottom,
                    ),
                    &[self.hwnd],
                )
                .1
                .map_or_else(String::new, |t| format!(" by {t:?}"))
            } else {
                String::new()
            };
            eprintln!(
                "[hide] target={} near={near} busy={busy} covered={covered}{by} cur={:.3}",
                self.dock.hide_target, self.dock.hide_cur
            );
        }
        true
    }

    /// 画一帧：egui 画 Dock → 离屏 → 回读 → 分层窗口。
    fn render(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // **Dock 主题**，不是设置窗口主题：`UiTheme` 与 `DockTheme` 的 0/1
        // 含义相反（见 `config.rs` 里那条防回归测试）。用错会把整个 Dock
        // 画成另一套配色，而且看起来"只是颜色不对"，很难察觉是取值取错了。
        let dark = self.dock.cfg.dock_is_dark(theme::is_dark());
        // 文字由 `core::clock` 生成：日期格式、周几、行数、顺序都在那里，
        // 与 C 版 `ClockLines()` 一一对应（曾经在 bin 里手拼，少了年份）。
        let clock: Option<Vec<ClockLine>> = if self.dock.cfg.clock {
            Some(clock::clock_lines(&self.dock.cfg, &now_parts()))
        } else {
            None
        };

        let ppp = self.ppp;
        let dock = &*self.dock;
        let raw = raw_input(self.frame.size(), ppp, self.start);
        self.frame
            .render(&self.gpu, raw, self.hwnd, self.pos, |ui| {
                dock.paint(ui, ppp, dark, clock.as_deref());
            })?;
        Ok(())
    }

    /// 按当前停靠边 / 预留区 / 尺寸重算窗口位置并移动。
    fn reposition(&mut self) {
        self.pos = layout::window_origin(
            self.dock.cfg.edge,
            self.work,
            self.reserved,
            self.size.0,
            self.size.1,
        );
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                self.pos.0,
                self.pos.1,
                self.size.0,
                self.size.1,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    /// 等下一个事件。有消息就醒；动画中按 16 ms 醒一次；自动隐藏 / 强制
    /// 收起开着时按 100 ms 轮询光标。
    ///
    /// 扫描周期与全屏检测都要参与超时，否则常显模式下会一直睡着 ——
    /// `INFINITE` 等不到"时间过去"这种事件，指示点与全屏状态都不会更新。
    /// 取两者较小值：全屏检测 1 s 一次，扫描间隔通常更长。
    fn wait(&self) {
        let timeout = if self.dock.anim {
            16
        } else if self.dock.needs_hide_tick() {
            100
        } else if self.raw.is_empty() {
            FS_CHECK_MS
        } else {
            Dock::SCAN_INTERVAL_MS.min(FS_CHECK_MS)
        };
        unsafe {
            MsgWaitForMultipleObjectsEx(None, timeout, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
        }
    }
}

/// 把队列里的消息一次处理完：逐条唤醒会造成"一帧一消息"的抖动。
///
/// 做成自由函数而不是 `App` 的方法：处理消息期间不需要碰 `App`，
/// 少一层借用关系。
fn pump(msg: &mut MSG) -> Pumped {
    let mut saw = false;
    loop {
        let has = unsafe { PeekMessageW(msg, None, 0, 0, PM_REMOVE) };
        if !has.as_bool() {
            break;
        }
        saw = true;
        if msg.message == WM_QUIT {
            return Pumped::Quit;
        }
        unsafe {
            let _ = TranslateMessage(msg);
            DispatchMessageW(msg);
        }
    }
    if saw {
        Pumped::Frames
    } else {
        Pumped::Idle
    }
}

/// `MINIDOCK_FAKE_MOUSE="u,v"`：解析成钉住的光标位置（逻辑/物理同单位的
/// Dock 内部坐标）。格式不对就当没设。
fn fake_mouse_from_env() -> Option<(f32, f32)> {
    let s = std::env::var("MINIDOCK_FAKE_MOUSE").ok()?;
    let (a, b) = s.split_once(',')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

/// `MINIDOCK_FAKE_COVERED=0/1`：覆盖智能隐藏的遮挡判定。
fn fake_covered_from_env() -> Option<bool> {
    let s = std::env::var("MINIDOCK_FAKE_COVERED").ok()?;
    match s.trim() {
        "1" | "true" => Some(true),
        "0" | "false" => Some(false),
        _ => None,
    }
}

/// `MINIDOCK_GEOM=1` 时打印度量与槽位，用于与 C 版逐项对拍
/// （验收 ≤1 px 时靠它定位）。
fn dump_geometry(dock: &Dock, dpi: i32) {
    if !trace::geom() {
        return;
    }
    let m = &dock.metrics;
    eprintln!(
        "GEOM dpi={dpi} S={:.1} Z={:.1} g={:.1} sep={:.1} pad={:.1} padX={:.1} barH={:.1} \
         over={:.1} margin={:.1} clockW={} clockExtra={:.1} reflH={:.1} lw={} lh={}",
        m.s,
        m.z,
        m.gap,
        m.sep_w,
        m.pad,
        m.pad_x,
        m.bar_h,
        m.overhang,
        m.margin,
        m.clock_w,
        m.clock_extra,
        m.refl_h,
        m.lw,
        m.lh
    );
    eprintln!(
        "GEOM bar_l={:.1} bar_w={:.1} bar_top={:.1} bar_bottom={:.1} icon_bottom={:.1}",
        dock.geom.bar_l,
        dock.geom.bar_w,
        dock.geom.bar_top,
        dock.geom.bar_bottom,
        dock.geom.icon_bottom
    );
    eprintln!("GEOM slot_l={:?}", dock.geom.slot_l);
}

/// 取本地时间，转成 `core` 用的结构（去掉 Win32 依赖）。
fn now_parts() -> clock::ClockParts {
    let st = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    clock::ClockParts {
        year: st.wYear,
        month: st.wMonth as u8,
        day: st.wDay as u8,
        weekday: st.wDayOfWeek as u8,
        hour: st.wHour as u8,
        minute: st.wMinute as u8,
        second: st.wSecond as u8,
    }
}

/// 取 `Dock` 指针。窗口数据没设好时返回空，调用方各自判空。
fn dock_ptr(hwnd: HWND) -> *mut Dock {
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Dock }
}

/// 客户区坐标（`WM_*MOUSE*` 给的就是物理像素）。
///
/// **不除以 ppp**：Dock 的几何（槽位、栏体、`lh`）全是物理像素，
/// 鼠标也必须同单位，否则命中会整体偏移一个 DPI 缩放因子。
fn physical_xy(lparam: LPARAM) -> (f32, f32) {
    let x = (lparam.0 & 0xFFFF) as f32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as f32;
    (x, y)
}

/// 申请一次 `WM_MOUSELEAVE`。系统只发一次，所以每次移动都要重新申请。
fn arm_mouse_leave(hwnd: HWND) {
    let mut tme = TRACKMOUSEEVENT {
        cbSize: u32::try_from(std::mem::size_of::<TRACKMOUSEEVENT>()).unwrap_or(0),
        dwFlags: TME_LEAVE,
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };
    unsafe {
        let _ = TrackMouseEvent(&raw mut tme);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let _ = BeginPaint(hwnd, &mut ps);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_MOUSEMOVE => {
            let (u, v) = physical_xy(lparam);
            let ptr = dock_ptr(hwnd);
            if !ptr.is_null() {
                unsafe {
                    (*ptr).mouse = (u, v);
                    (*ptr).hover = true;
                    // 按住时把移动喂给拖拽状态机；没按住时它自己会早退。
                    (*ptr).drag_to(u, v);
                    (*ptr).mark_dirty();
                }
                // 让系统在鼠标离开时发 `WM_MOUSELEAVE`：不装这个钩子，
                // `hover` 会永远是 true，Dock 停在放大状态不回去。
                arm_mouse_leave(hwnd);
            }
            LRESULT(0)
        }
        // 鼠标离开窗口：取消悬停。没有这一段，移开鼠标后图标会一直放大。
        WM_MOUSELEAVE => {
            let ptr = dock_ptr(hwnd);
            if !ptr.is_null() {
                unsafe {
                    (*ptr).mouse = (-1.0, -1.0);
                    (*ptr).hover = false;
                    (*ptr).mark_dirty();
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (u, v) = physical_xy(lparam);
            let ptr = dock_ptr(hwnd);
            if !ptr.is_null() {
                // 原版在命中时 `SetCapture`：把后续消息都收到自己这里，
                // 鼠标移出窗口也不会丢抬起事件（否则 Dock 会"卡住"）。
                // Shift 必须在**按下**时读：抬起那一刻用户可能已经松开，
                // 而 Shift+点击的语义是"我要开第二个实例"，读晚了就丢。
                let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
                if unsafe { (*ptr).press_at(u, v, shift) } {
                    unsafe {
                        let _ = SetCapture(hwnd);
                    }
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (u, v) = physical_xy(lparam);
            let ptr = dock_ptr(hwnd);
            if !ptr.is_null() {
                unsafe {
                    if GetCapture() == hwnd {
                        let _ = ReleaseCapture();
                    }
                    // 只记录"点了哪个"，真正的启动放在主循环里做：
                    // `ShellExecuteW` 可能阻塞几十毫秒，不该占着窗口过程。
                    let _ = (*ptr).release_at(u, v);
                    (*ptr).mark_dirty();
                }
            }
            LRESULT(0)
        }
        // **不要抢焦点**。Dock 是常驻工具窗，点图标时如果把焦点从用户
        // 正在用的窗口抢走，接着启动的应用反而拿不到预期的前台位置，
        // 观感是"点一下整个桌面闪一下"。原版同样返回 MA_NOACTIVATE。
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        // 鼠标捕获被抢走（UAC 弹窗、Alt+Tab、别的程序 SetCapture）：
        // 抬起事件永远不会到，必须主动取消拖拽，否则下次点击会带着
        // 上一次的"按下"状态，表现为点一下启动了上一个图标。
        WM_CAPTURECHANGED => {
            let ptr = dock_ptr(hwnd);
            if !ptr.is_null() {
                unsafe {
                    (*ptr).cancel_drag();
                }
            }
            LRESULT(0)
        }
        // 中键 = 强制新实例（原版 `LaunchItem(hi, TRUE)`）。
        WM_MBUTTONUP => {
            let (u, v) = physical_xy(lparam);
            let ptr = dock_ptr(hwnd);
            if !ptr.is_null() {
                unsafe {
                    (*ptr).force_click(u, v);
                    (*ptr).mark_dirty();
                }
            }
            LRESULT(0)
        }
        // `Ctrl+Alt+D` 强制收起 / 恢复。常显模式下也能用。
        WM_HOTKEY => {
            let ptr = dock_ptr(hwnd);
            if !ptr.is_null() {
                let hidden = unsafe { (*ptr).toggle_force_hide() };
                eprintln!("[hotkey] {}", if hidden { "已收起" } else { "已恢复" });
            }
            LRESULT(0)
        }
        // 系统深浅色切换（`ImmersiveColorSet`）要**立即**生效。
        // 只在启动时读一次主题的话，用户切到深色后 Dock 会一直是浅色，
        // 直到重启 —— 这是最容易被当成 bug 报上来的那种问题。
        WM_SETTINGCHANGE => {
            let ptr = dock_ptr(hwnd);
            if !ptr.is_null() {
                unsafe {
                    (*ptr).mark_dirty();
                    (*ptr).anim = true;
                }
            }
            LRESULT(0)
        }
        // 右键：只记位置，菜单由主循环弹（模态，不能在窗口过程里阻塞）。
        WM_RBUTTONDOWN => LRESULT(0),
        WM_RBUTTONUP => {
            let (u, v) = physical_xy(lparam);
            let ptr = dock_ptr(hwnd);
            if !ptr.is_null() {
                unsafe {
                    (*ptr).request_menu(u, v);
                }
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 as u32 == 0x1B {
                unsafe { PostQuitMessage(0) };
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// 全屏检测的最小间隔（毫秒）。
///
/// 只做两个只读查询、不触发渲染，所以下限就是"多久能发现自己该让开"。
/// 1 秒对用户感知足够快，同时把空闲唤醒压到每秒一次。
const FS_CHECK_MS: u32 = 1000;

/// 设置面板改动到写盘的防抖窗口（毫秒）。
///
/// 拖一下滑块会产生几十次 `changed`；ini 是 UTF-16 全量回写，逐次写盘
/// 会让一次拖动变成几十次文件重写。400 ms 是"停手就存、拖着不存"的折中。
const CFG_SAVE_DEBOUNCE_MS: u64 = 400;

/// 给每个条目装图标。
///
/// 来源顺序照抄原版 `LoadItemIcon()` 的主干，但只覆盖"能拿到的"几条：
///
/// | 条目 | 做法 |
/// |---|---|
/// | 分割线 | 不装（画竖线） |
/// | 有 `icon_src` 且文件存在 | **暂不支持** —— 需要 PNG/JPEG 解码器 |
/// | 网址 | **暂不支持** —— 原版要联网抓 favicon |
/// | 其余（文件 / 文件夹 / 快捷方式 / 回收站） | 交给 shell 图像列表 |
///
/// 后两条目前退化为首字占位（原版也是这个兜底，只是颜色算法不同）。
fn load_icons(dock: &mut Dock, ctx: &egui::Context, raw: &[Item]) {
    let mut ok = 0usize;
    let mut miss = 0usize;
    for (i, item) in raw.iter().enumerate() {
        if item.kind == ItemKind::Separator {
            continue;
        }
        // 自定义图标：原版用 GDI+ 解码任意图片。我们没有图像解码器，
        // 这种情况先退回系统图标 —— 至少不会画出一个空白块。
        let bmp: Option<IconBitmap> = match item.kind {
            ItemKind::Url => None,
            ItemKind::Trash => icon::shell_icon("shell:RecycleBinFolder"),
            _ => icon::shell_icon(&item.path).or_else(|| icon::resource_icon(&item.path, 0)),
        };
        match bmp {
            Some(b) => {
                dock.load_icon(i, ctx, &b, FitMode::from_code(item.fit));
                ok += 1;
            }
            None => {
                miss += 1;
                // 点名而不是只报个数："为什么这个图标是占位"要能一眼看出。
                eprintln!("  未取到图标: [{}] {} <{}>", i, item.name, item.path);
            }
        }
    }
    eprintln!("图标: 已装 {ok} 个，未取到 {miss} 个（未取到的画首字占位）");
    if trace::geom() {
        for (i, item) in raw.iter().enumerate() {
            eprintln!(
                "  [{i}] kind={:?} path=<{}> args=<{}>",
                item.kind, item.path, item.args
            );
        }
    }
}

/// 把条目顺序写回 ini。
///
/// 每次重新读文件再改：设置窗口可能在此期间往别的段写过值，
/// 用启动时那份内存副本覆盖会把它们抹掉。
fn save_order(paths: &Paths, items: &[Item]) -> Result<(), Box<dyn std::error::Error>> {
    let mut ini = Ini::load(&paths.ini)?;
    store::save_items(items, &mut ini);
    ini.save(&paths.ini)?;
    Ok(())
}

/// 光标是否算"靠近 Dock"。对应原版 `OnAutoHide()` 里的两套判断。
///
/// - **已收起时**只认屏幕边缘的那条窄带：此时 Dock 是看不见的，
///   若按"靠近栏体"判断，光标永远够不着它，就再也弹不出来了。
/// - **已显示时**按栏体外扩一圈判断，鼠标在图标上方（放大溢出区）也算靠近。
fn cursor_near(dock: &Dock, hwnd: HWND) -> bool {
    let mut p = POINT::default();
    if unsafe { GetCursorPos(&raw mut p) }.is_err() {
        return false;
    }

    if dock.hide_target {
        // 收起态：光标贴住 Dock 所在的那条屏幕边，且横向落在窗口范围内。
        let mut wr = RECT::default();
        if unsafe { GetWindowRect(hwnd, &raw mut wr) }.is_err() {
            return false;
        }
        let work = dock.work;
        let on_edge = match dock.cfg.edge {
            Edge::Top => p.y <= work.top + 1,
            Edge::Left => p.x <= work.left + 1,
            Edge::Right => p.x >= work.right - 2,
            Edge::Bottom => p.y >= work.bottom - 2,
        };
        let along = if dock.cfg.edge.is_horizontal() {
            p.x >= wr.left && p.x < wr.right
        } else {
            p.y >= wr.top && p.y < wr.bottom
        };
        return on_edge && along;
    }

    // 显示态：把屏幕坐标转成客户区，再按栏体外扩一圈判断。
    let mut c = p;
    // SAFETY: `c` 是本地 POINT；失败只会让坐标保持屏幕值，不影响安全性。
    unsafe {
        let _ = ScreenToClient(hwnd, &raw mut c);
    }
    let (u, v) = (c.x as f32, c.y as f32);
    let m = &dock.metrics;
    let top_margin = if dock.cfg.zoom { m.z } else { m.s };
    u >= dock.geom.bar_l - 10.0
        && u <= dock.geom.bar_l + dock.geom.bar_w + 10.0
        && v >= dock.geom.bar_top - top_margin - 12.0
        && v <= m.lh as f32 + 4.0
}

/// 栏体这块区域当前是否被普通窗口盖住（智能隐藏判定）。
///
/// `origin` 是窗口左上角的屏幕坐标。栏体在客户区内是 `bar_l/bar_top` 起算的，
/// 加上窗口原点才是屏幕坐标 —— 这里最容易错的是**忘了加原点**，
/// 结果判定的是屏幕左上角一小块，永远判不出遮挡。
fn dock_bar_covered(dock: &Dock, hwnd: HWND, x: i32, y: i32) -> bool {
    let left = dock.geom.bar_l.round() as i32;
    let top = dock.geom.bar_top.round() as i32;
    let right = (dock.geom.bar_l + dock.geom.bar_w).round() as i32;
    // 与原版一致：算到 `barH + margin`，把下方的留白也算进去。
    let bottom = (dock.geom.bar_bottom + dock.metrics.margin).round() as i32;
    let target = LRect::new(x + left, y + top, x + right, y + bottom);
    // 目前只有 Dock 自己的窗口需要排除。将来加设置窗 / 预览窗时，
    // 一定要塞进这个列表，否则"设置窗压在 Dock 上"会被判成被遮挡。
    occlusion::any_window_covers(target, &[hwnd])
}

/// 把 `.lnk` 条目解析成真实 exe 路径，缓存在 `Item.target`。
///
/// 只在启动时跑一次：解析要初始化 COM 并创建 shell 对象，
/// 每次都做的话 1.2 秒一轮的扫描会明显变重。解析失败（指向文档、
/// 指向 URL、文件已删除）就保持空 —— 那种条目本来就没有可匹配的 exe。
fn resolve_targets(items: &mut [Item]) {
    let mut resolved = 0usize;
    for item in items.iter_mut() {
        // 只有文件类条目才有可执行目标；分割线/网址/文件夹/回收站跳过。
        if item.kind != ItemKind::File {
            continue;
        }
        if let Some(sc) = shortcut::resolve(&item.path) {
            item.target = sc.target;
            resolved += 1;
        }
    }
    if trace::geom() {
        eprintln!("[scan] 解析出 {resolved} 个可执行目标");
    }
}

/// 扫描运行中的应用并刷新条目状态。返回是否有变化。
///
/// `Dock` 只持有 `DockItem`，与 `raw` 是两份列表 —— 必须同步，
/// 否则指示点与实际点击行为会对不上。
fn refresh_running(raw: &mut [Item], dock: &mut Dock) -> bool {
    let running: std::collections::HashSet<String> =
        app_window::running_apps().into_keys().collect();
    let changed = running::refresh_running(raw, &running);
    if changed {
        for (slot, item) in dock.items.iter_mut().zip(raw.iter()) {
            slot.running = item.running;
        }
        if trace::geom() {
            let names: Vec<&str> = raw
                .iter()
                .filter(|i| i.running)
                .map(|i| i.name.as_str())
                .collect();
            eprintln!(
                "[scan] 运行中: {names:?} | slot_l={:?} cur={:?}",
                dock.geom.slot_l,
                dock.items.iter().map(|i| i.cur).collect::<Vec<_>>()
            );
        }
    }
    changed
}

// ============================ 右键菜单 ============================

/// 菜单窗口状态。放在堆上，指针挂到 `GWLP_USERDATA`。
struct MenuWindow {
    menu: Menu,
    dark: bool,
    /// 模态循环的退出标志。
    done: bool,
    /// 被选中的命令。`None` + `done` = 取消。
    cmd: Option<MenuCmd>,
    /// DPI 缩放。物理尺寸按它算，命中也要用它换算回逻辑坐标。
    ppp: f32,
}

fn menu_ptr(hwnd: HWND) -> *mut MenuWindow {
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut MenuWindow }
}

/// 客户区物理坐标 → 逻辑坐标（`Menu::hit` 吃的是逻辑坐标）。
fn menu_logical(lparam: LPARAM, ppp: f32) -> (f32, f32) {
    let (x, y) = physical_xy(lparam);
    (x / ppp, y / ppp)
}

unsafe extern "system" fn menu_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if trace::menu() {
            eprintln!("[menu-proc] msg=0x{msg:04X} wparam=0x{:X}", wparam.0);
        }
        match msg {
            // 与 Dock 一样不抢焦点：菜单是工具窗，抢焦点会让用户原本正在
            // 用的窗口失焦，关闭菜单后还得再点一次才能继续。
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_MOUSEMOVE => {
                let ptr = menu_ptr(hwnd);
                if !ptr.is_null() {
                    let (u, v) = menu_logical(lparam, (*ptr).ppp);
                    let hit = (*ptr).menu.hit(u, v);
                    (*ptr).menu.set_hover(hit);
                }
                LRESULT(0)
            }
            // 点在菜单矩形之外 = 取消。这条依赖 `SetCapture`：
            // 菜单没有焦点，别的窗口上的点击不会转发过来。
            WM_LBUTTONDOWN => {
                let ptr = menu_ptr(hwnd);
                if !ptr.is_null() {
                    let (u, v) = menu_logical(lparam, (*ptr).ppp);
                    let (w, h) = (*ptr).menu.logical_size();
                    if u < 0.0 || v < 0.0 || u > w || v > h {
                        (*ptr).done = true;
                    }
                }
                LRESULT(0)
            }
            // 抬起即选中。注意 `hit` 返回 `None` 时也要结束 ——
            // 落在分隔带上的点击等于"什么也没选"，菜单应当收起。
            WM_LBUTTONUP => {
                let ptr = menu_ptr(hwnd);
                if !ptr.is_null() {
                    let (u, v) = menu_logical(lparam, (*ptr).ppp);
                    (*ptr).cmd = (*ptr).menu.hit(u, v).and_then(|i| (*ptr).menu.cmd_at(i));
                    (*ptr).done = true;
                }
                LRESULT(0)
            }
            WM_KEYDOWN => {
                // Esc 取消。其他键忽略：菜单没有键盘导航，别装作有。
                if wparam.0 as u32 == 0x1B {
                    let ptr = menu_ptr(hwnd);
                    if !ptr.is_null() {
                        (*ptr).done = true;
                    }
                }
                LRESULT(0)
            }
            // 捕获被别人抢走（弹窗、Alt+Tab）：已经收不到点击了，
            // 必须收起，否则会挂在屏幕上等一个永远不会到来的"取消"。
            WM_CAPTURECHANGED => {
                let ptr = menu_ptr(hwnd);
                if !ptr.is_null() {
                    (*ptr).done = true;
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// 注册菜单窗口类。已注册时 `RegisterClassW` 返回 0，忽略即可。
fn register_menu_class() -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(menu_proc),
            hInstance: HINSTANCE(hinst.0),
            lpszClassName: w!("MiniDockMenuWindow"),
            ..Default::default()
        };
        RegisterClassW(&raw const wc);
    }
    Ok(())
}

/// 菜单窗口的复用槽：窗口句柄 + 专属渲染器。
///
/// 为什么渲染器要独立：Dock 的 `FrameRenderer` 尺寸固定为栏体大小，
/// 菜单尺寸完全不同。而 `resize` 只重建离屏目标与 DIB，
/// egui 上下文与 wgpu 管线（最贵的两步）都保留。
type MenuSlot = Option<(HWND, FrameRenderer)>;

/// 弹出菜单并阻塞到用户选择或取消。返回被选中的命令。
///
/// **模态**：期间不处理 Dock 的其他输入（上游 `ShowMenu` 同样是模态的）。
/// 用 `SetCapture` 而不是激活窗口来实现"点外面关闭" —— 菜单不抢焦点，
/// 关闭后用户原来在用的窗口仍是前台。
#[allow(clippy::too_many_arguments)]
fn show_menu(
    gpu: &Gpu,
    slot: &mut MenuSlot,
    menu: Menu,
    cursor: (f32, f32),
    bounds: (f32, f32, f32, f32),
    dark: bool,
    ppp: f32,
    start: Instant,
) -> Result<Option<MenuCmd>, Box<dyn std::error::Error>> {
    let (lw, lh) = menu.logical_size();
    // 向上取整：少一个像素会把最后一行的圆角裁掉。
    let pw = (lw * ppp).ceil() as i32;
    let ph = (lh * ppp).ceil() as i32;
    let (lx, ly) = Menu::place(cursor, (lw, lh), bounds);
    let (px, py) = ((lx * ppp).round() as i32, (ly * ppp).round() as i32);

    // 懒建：第一次开菜单时建窗口与渲染器，之后复用。
    // 第一次会慢一点（装 CJK 字体 + 编译 wgpu 管线），之后只剩 resize。
    if slot.is_none() {
        register_menu_class()?;
        let hinst = unsafe { GetModuleHandleW(None)? };
        unsafe {
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("MiniDockMenuWindow"),
                w!("MiniDockMenu"),
                WS_POPUP,
                px,
                py,
                pw,
                ph,
                None,
                None,
                Some(HINSTANCE(hinst.0)),
                None,
            )?;
            let renderer = FrameRenderer::new(gpu, pw as u32, ph as u32, ppp)?;
            *slot = Some((hwnd, renderer));
        }
    }

    // 上面刚保证 `Some`；用 `ok_or` 而不是 `expect`：万一将来改动破坏了这个
    // 不变量，应当报错而不是把进程打崩。
    let Some((hwnd, renderer)) = slot.as_mut() else {
        return Err("菜单窗口未初始化".into());
    };
    renderer.resize(gpu, pw as u32, ph as u32)?;

    // 位置与尺寸一起改：菜单每次打开的高度都可能不同。
    unsafe {
        let _ = SetWindowPos(*hwnd, Some(HWND_TOPMOST), px, py, pw, ph, SWP_NOACTIVATE);
    }

    if trace::geom() {
        eprintln!(
            "[menu] items={} labels={:?} logical=({lw},{lh}) physical=({pw},{ph}) at=({px},{py})",
            menu.entries.len(),
            menu.entries
                .iter()
                .map(|e| e.label.as_str())
                .collect::<Vec<_>>()
        );
    }
    let mut state = Box::new(MenuWindow {
        menu,
        dark,
        done: false,
        cmd: None,
        ppp,
    });
    unsafe {
        SetWindowLongPtrW(*hwnd, GWLP_USERDATA, (&raw mut *state) as isize);
        let _ = ShowWindow(*hwnd, SW_SHOWNA);
        SetCapture(*hwnd);
    }

    // 首帧必画：窗口是重新显示的，之前那块内容已经无效。
    let mut painted: Option<Option<usize>> = None;
    let mut msg = MSG::default();
    loop {
        // 只取本窗口的消息：模态期间 Dock 的输入要压住。
        while unsafe { PeekMessageW(&raw mut msg, Some(*hwnd), 0, 0, PM_REMOVE) }.as_bool() {
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if state.done {
            break;
        }
        // 高亮变了才重绘。菜单很小，但没必要鼠标每动一像素就跑一遍 GPU。
        if painted != Some(state.menu.hover) {
            painted = Some(state.menu.hover);
            let menu = &state.menu;
            let dark = state.dark;
            renderer.render(
                gpu,
                raw_input(renderer.size(), ppp, start),
                *hwnd,
                (px, py),
                |ui| menu.paint(ui, dark),
            )?;
        }
        unsafe {
            MsgWaitForMultipleObjectsEx(None, 200, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
        }
    }

    if trace::geom() {
        eprintln!("[menu] 关闭 cmd={:?}", state.cmd);
    }
    unsafe {
        if GetCapture() == *hwnd {
            let _ = ReleaseCapture();
        }
        SetWindowLongPtrW(*hwnd, GWLP_USERDATA, 0);
        let _ = ShowWindow(*hwnd, SW_HIDE);
    }
    Ok(state.cmd)
}

/// 执行菜单命令。`hit` 是当初右键命中的条目下标（`None` = 空白处）。
///
/// 会改条目列表的三条命令走同一条收尾路径：**先改 `raw`（真源）→ 写回 ini
/// → 整份替换 `dock.items`**。替换之后几何由下一帧 `update()` 重算，
/// 窗口尺寸的变化由主循环 ②.1 兜住 —— 这里不直接碰窗口。
fn run_menu_cmd(
    cmd: MenuCmd,
    hit: Option<usize>,
    raw: &mut Vec<Item>,
    dock: &mut Dock,
    paths: &Paths,
) {
    match cmd {
        // 与左键点击同一条路径：已运行则激活，否则启动。
        MenuCmd::Open => {
            let Some(item) = hit.and_then(|i| raw.get(i)) else {
                return;
            };
            // 先把开关读成 bool：后面要可变借用 `dock`，不能再挂着对
            // `dock.cfg` 的借用。
            let toggle = dock.cfg.get_i32(Key::CLICK_TOGGLE) != 0;
            let switched =
                running::running_key(item).is_some_and(|exe| app_window::activate(&exe, toggle));
            if !switched {
                if let Err(e) = launch::launch(item) {
                    eprintln!("打开「{}」失败：{e}", item.name);
                }
            }
        }
        MenuCmd::OpenInExplorer => {
            if let Some(item) = hit.and_then(|i| raw.get(i)) {
                if let Err(e) = launch::open_folder(&item.path) {
                    eprintln!("打开文件夹「{}」失败：{e}", item.name);
                }
            }
        }
        MenuCmd::QuitApp => {
            if let Some(exe) = hit.and_then(|i| raw.get(i)).and_then(running::running_key) {
                if !app_window::close_app(&exe) {
                    eprintln!("退出「{exe}」失败");
                }
            }
        }
        MenuCmd::ExitDock => {
            // 走正常退出路径：`Drop` 会注销 AppBar、释放单实例锁。
            unsafe { PostQuitMessage(0) };
        }
        // 入口在 `tick` 之前就被截走了（`MenuCmd::Settings` 要 GPU 才能开窗口）。
        // 这里是穷尽匹配要求的占位：真走到这里说明拦截失效了，那是一个 bug。
        MenuCmd::Settings => {
            eprintln!("`Settings` 命令走错了路径：它应当在主循环里被处理");
        }
        MenuCmd::AddSepAfter => {
            let Some(i) = hit else { return };
            if i >= raw.len() {
                return;
            }
            raw.insert(i + 1, Item::new(ItemKind::Separator, "", ""));
            apply_item_change(raw, dock, paths);
        }
        MenuCmd::RemoveItem | MenuCmd::RemoveSep => {
            let Some(i) = hit else { return };
            if i >= raw.len() {
                return;
            }
            // 最后一项不允许删：Dock 会变成一个空壳，而用户没有任何
            // 可点的东西能把它恢复回来（拖放进来的入口还没做）。
            if raw.len() <= 1 {
                eprintln!("这是最后一项，不能移除。");
                return;
            }
            raw.remove(i);
            apply_item_change(raw, dock, paths);
        }
    }
}

/// 条目列表变了之后的统一收尾：写回 ini、替换显示列表。
///
/// 写盘失败不回滚内存里的改动：宁可"这次生效但没保存"，也不要
/// "点了没反应"。失败会打印出来，下一次成功保存时一并写入。
fn apply_item_change(raw: &[Item], dock: &mut Dock, paths: &Paths) {
    if let Err(e) = save_order(paths, raw) {
        eprintln!("保存配置失败（改动仍在本进程内生效）：{e}");
    }
    let items: Vec<DockItem> = raw.iter().map(DockItem::from_item).collect();
    dock.replace_items(items);
}

// ============================ 设置窗口 ============================

/// 首次打开时建窗，之后一直复用同一个窗口与渲染器。
///
/// 懒建是为了别把"装字体 + 编译 GPU 管线"那几十毫秒记在 Dock 的启动时间上：
/// 大部分会话压根不会打开设置。
fn ensure_settings<'a>(
    gpu: &Gpu,
    slot: &'a mut Option<Pin<Box<SettingsWindow>>>,
    paths: &Paths,
    ppp: f32,
) -> Result<&'a mut SettingsWindow, Box<dyn std::error::Error>> {
    if slot.is_none() {
        let monitors = display::monitors();
        let state = SettingsState {
            config_dir: paths.dir.display().to_string(),
            monitors: monitors
                .iter()
                .enumerate()
                .map(|(i, _)| {
                    if i == 0 {
                        "主显示器".to_owned()
                    } else {
                        format!("显示器 {}", i + 1)
                    }
                })
                .collect(),
            // 显示的是注册表里的真值，不是配置项：用户可以在任务管理器里改它。
            auto_start: theme::get_autostart(),
            // 软件光栅器不进下拉框：选它只会得到一帧几十毫秒的结果。
            adapters: minidock_platform::gpu::adapters()
                .into_iter()
                .filter(|a| a.kind != minidock_platform::AdapterKind::Software)
                .map(|a| a.name)
                .collect(),
            ..SettingsState::default()
        };
        // `new` 返回的是 pin 住的装箱值：窗口过程按地址找它，地址不能变。
        *slot = Some(SettingsWindow::new(gpu, state, ppp)?);
    }
    // 同上：刚塞进去就取不出来说明逻辑有 bug，返回错误比 panic 好定位。
    let sw = slot.as_mut().ok_or("设置窗口未初始化")?;
    Ok(&mut **sw)
}

/// 每轮给设置窗口一次机会。
///
/// 返回 `(配置有改动, 需要重挂)`。第二项由调用方做，因为 AppBar 与窗口位置
/// 是主循环的东西 —— 传进来会把这个函数拖成十几个参数。
fn tick_settings(
    gpu: &Gpu,
    slot: &mut Option<Pin<Box<SettingsWindow>>>,
    dock: &mut Dock,
    raw: &mut Vec<Item>,
    paths: &Paths,
    dpi: i32,
) -> Result<(bool, bool), Box<dyn std::error::Error>> {
    let Some(sw) = slot.as_mut() else {
        return Ok((false, false));
    };
    // 停靠边 / 预留 / 隐藏模式变了要重新注册 AppBar 并重算窗口位置。
    let before = (dock.cfg.edge, dock.cfg.reserve, dock.cfg.mode);

    let mut changed = false;
    if sw.visible() && sw.due() {
        // 关键点：`cfg` 就是 Dock 用的那一份。设置窗口改它等于改 Dock，
        // 没有 IPC、没有复制、也没有"保存后重启"。
        sw.render(gpu, &mut dock.cfg)?;
        let out = std::mem::take(&mut sw.outcome);
        changed = out.changed;
        if let Some(cmd) = out.command {
            changed |= run_settings_cmd(cmd, dock, raw, paths, sw, dpi);
        }
    }
    // Esc 与"关闭"都走这个标志：关闭要在命令结算之后执行，
    // 否则点关闭按钮那一帧的改动会被丢掉。
    if sw.close_requested {
        sw.hide();
    }
    let after = (dock.cfg.edge, dock.cfg.reserve, dock.cfg.mode);
    Ok((changed, before != after))
}

/// 执行设置界面发出的命令。返回值表示"是否需要把配置写盘"。
///
/// 还没接通的能力在最后一支里给出明确提示 —— 什么都不做会让用户以为按钮
/// 坏了，而那几项本来就在 `PORTING.md` 的待办里。
fn run_settings_cmd(
    cmd: Command,
    dock: &mut Dock,
    raw: &mut Vec<Item>,
    paths: &Paths,
    sw: &mut SettingsWindow,
    dpi: i32,
) -> bool {
    match cmd {
        // 关闭由 `tick_settings` 处理（窗口要藏起来），这里不再做事。
        Command::Close => false,
        Command::OpenConfigDir => {
            let dir = paths.dir.display().to_string();
            if let Err(e) = launch::open_folder(&dir) {
                sw.state.status = format!("打开配置目录失败：{e}");
            }
            false
        }
        Command::ToggleAutostart(on) => {
            if theme::set_autostart(on) {
                // 写完重新读真值：策略把写入拦下来时，写函数也可能返回成功。
                sw.state.auto_start = theme::get_autostart();
                sw.state.status = if sw.state.auto_start {
                    "已设置开机自启动".to_owned()
                } else {
                    "已取消开机自启动".to_owned()
                };
            } else {
                sw.state.status =
                    format!("写入注册表失败（错误码 {}）", theme::last_registry_error());
                sw.state.auto_start = theme::get_autostart();
            }
            false
        }
        Command::ResetDefaults => {
            dock.cfg = Config::defaults(dpi);
            sw.state.status = "已恢复默认设置".to_owned();
            true
        }
        Command::Recommended => {
            apply_recommended(&mut dock.cfg);
            sw.state.status = "已套用推荐设置（尺寸与停靠边没动）".to_owned();
            true
        }
        Command::LowPower => {
            apply_low_power(&mut dock.cfg);
            sw.state.status = "已关闭高开销视觉效果".to_owned();
            true
        }
        // 这两项与右键菜单里的同一条路径共用收尾：改 `raw` → 写盘 → 换列表。
        Command::AddSeparator => {
            raw.push(Item::new(ItemKind::Separator, "", ""));
            apply_item_change(raw, dock, paths);
            true
        }
        Command::AddTrash => {
            raw.push(Item::new(ItemKind::Trash, "回收站", ""));
            apply_item_change(raw, dock, paths);
            true
        }
        _ => {
            sw.state.status = "这项能力还没接进 Dock（见 PORTING.md 待办）".to_owned();
            false
        }
    }
}

/// 推荐设置：只动"要不要这些效果"，尺寸、颜色与停靠边一概不碰 ——
/// 那些是用户已经调好、一改就得从头再来的东西。
fn apply_recommended(cfg: &mut Config) {
    for (key, v) in [
        (Key::Labels, 1),
        (Key::Zoom, 1),
        (Key::Reflect, 1),
        (Key::Dots, 1),
        (Key::Bounce, 1),
        (Key::MinAnim, 1),
        (Key::Border, 1),
        (Key::Preview, 1),
    ] {
        cfg.apply_i32(key, v);
    }
}

/// 低占用：关掉所有"每帧都要多干一遍"的效果。磨砂（`Blur` + `BgMode`）是
/// 这里面唯一的常驻开销，其余只是少几次在特定时机额外做的事。
fn apply_low_power(cfg: &mut Config) {
    for (key, v) in [
        (Key::Reflect, 0),
        (Key::Blur, 0),
        (Key::MinAnim, 0),
        (Key::Bounce, 0),
        (Key::Preview, 0),
        (Key::FolderFan, 0),
        (Key::ShowRunning, 0),
        (Key::BgMode, 0),
    ] {
        cfg.apply_i32(key, v);
    }
}

/// 把当前配置写回 ini。
///
/// 先读一次再写：`[Items]` 段不归 `write_ini` 管，直接新建一份会丢光条目。
fn save_config(paths: &Paths, cfg: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let mut ini = Ini::load(&paths.ini)?;
    cfg.write_ini(&mut ini);
    ini.save(&paths.ini)?;
    Ok(())
}

/// 给设置窗口的底部状态条写一句话（窗口没建过就什么都不做）。
fn settings_status(slot: &mut Option<Pin<Box<SettingsWindow>>>, text: &str) {
    if let Some(sw) = slot.as_mut() {
        sw.state.status = text.to_owned();
    }
}

/// 构造单帧 `RawInput`。Dock 与菜单共用同一口径 —— 这两处原来各写一遍。
///
/// - `screen_rect` 必须给对：egui 用它决定根 Ui 的尺寸，给错栏体会被画在
///   错误的位置（M1 第一版就踩了）。
/// - `native_pixels_per_point` 决定原生缩放，不设的话 egui 按 1.0 算，
///   画面只占表面的 1/ppp（偏左上、尺寸偏小）。
fn raw_input(size: (u32, u32), ppp: f32, start: Instant) -> egui::RawInput {
    let (w, h) = size;
    let mut raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(w as f32 / ppp, h as f32 / ppp),
        )),
        time: Some(start.elapsed().as_secs_f64()),
        ..Default::default()
    };
    raw.viewports.insert(
        egui::ViewportId::ROOT,
        egui::ViewportInfo {
            native_pixels_per_point: Some(ppp),
            ..Default::default()
        },
    );
    raw
}
