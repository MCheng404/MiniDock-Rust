//! 起动台：全屏的应用网格。
//!
//! 数据模型与"拖到图标上合并成文件夹"
//! 的规则在 `nooksill_core::launchpad`，这里只管画、判命中、收输入。
//!
//! 与前面几个面板的差别：**这个要抢焦点**。
//!
//! - 搜索框直接收键盘（`WM_CHAR`），所以不能用 `WS_EX_NOACTIVATE`；
//! - 打开时铺满整块显示器，`ShowWindow(SW_SHOW)` + 抢前台，
//!   关闭时把焦点还回去；
//! - 图标走 `worker::IconLoader`（与 Dock 同一个线程实现），
//!   只给"这一页看得见的"排任务 —— 一百个应用一次性取图标会让打开卡住几秒。
//!
//! 不支持中文输入法：`WM_CHAR` 能收到已上屏的字符，但 IME 组合窗口没接
//! （PORTING.md 里那条待办）。英文搜索可用，中文名要打出来得等 IME 接上。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Instant;

use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect as EguiRect, Stroke};
use nooksill_core::launchpad::{Launchpad, TakeOut, Top};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetWindowLongPtrW, RegisterClassW, SetForegroundWindow,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, GWLP_USERDATA, SW_SHOW, WM_APP, WM_CHAR,
    WM_DROPFILES, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_PAINT, WM_RBUTTONUP, WM_SETCURSOR, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};

use crate::frame::FrameRenderer;
use crate::gpu::{Gpu, GpuError};
use crate::ime::Ime;
use crate::worker::{IconLoader, IconRequest, Waker};

/// 窗口类名。
pub const CLASS_NAME: &str = "NookSillLaunchpad";

/// 图标线程叫醒我们用的消息（与 Dock 用同一个约定）。
const WM_APP_WAKE: u32 = WM_APP + 1;

/// 搜索框。
const SEARCH_W: f32 = 380.0;
const SEARCH_H: f32 = 44.0;
/// 网格左右留白与上下留白。
const SIDE: f32 = 70.0;
const TOP_H: f32 = 124.0;
const BOT_H: f32 = 146.0;

/// 起动台要交给主循环做的事（它自己改不了 Dock 的配置）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchpadCmd {
    /// 把某个应用加到 Dock 栏。
    AddToDock {
        /// 显示名。
        name: String,
        /// 路径。
        path: String,
    },
}

/// 右键菜单当前是哪一层、作用于谁。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum MenuState {
    /// 顶层格的菜单（未过滤视图）。
    Cell(usize),
    /// 过滤视图里的某一格的菜单（此时格子是应用，不是顶层格）。
    App(usize),
    /// 二级菜单：给这个应用选标签组。
    Category(usize),
    /// 标签栏某一组的菜单。存的是**组下标**（从 0 起），不是界面上的 1 起编号。
    Tag(usize),
}

/// 重命名作用于谁。
///
/// 重命名输入框只有一套，但目标有两种：格子名与标签组名。用枚举区分，
/// 免得再加一个平行字段（那样 ESC / 提交 / 点击别处三条路径都要各写两遍）。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum RenameTarget {
    /// 顶层格下标。
    Cell(usize),
    /// 标签组下标（从 0 起）。
    Tag(usize),
}

/// 画在网格之上的东西：右键菜单与重命名框。
#[derive(Copy, Clone, Debug, Default)]
struct Overlay<'a> {
    /// 菜单项文案（空 = 不画菜单）。
    menu_items: &'a [String],
    /// 菜单左上角（逻辑点）。
    menu_at: (f32, f32),
    /// 键盘选中的格（**当前页内**的序号）。`None` = 还没用键盘选过。
    sel: Option<usize>,
    /// 当前分类筛选。`0` = 全部，`1..=tags.len()` = 第几组标签。
    cat: usize,
    /// 重命名作用于谁。`None` = 没在改名；开着时**搜索框就地变成改名框**。
    renaming: Option<RenameTarget>,
    /// 重命名缓冲。
    rename_buf: &'a str,
}

/// 命中区列表：`(目标, 逻辑矩形)`。绘制时算出来，鼠标事件里用。
type HitList = Vec<(Hit, EguiRect)>;

/// 一次命中的目标。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Hit {
    /// 网格里的第 k 格（当前页内的序号）。
    Cell(usize),
    /// 右键菜单里的第 k 项。
    MenuItem(usize),
    /// 右上角的「＋」。
    Plus,
    /// 标签栏第 k 个：0 = 全部，1..=tags.len() = 标签，最后一个是「＋」。
    Tab(usize),
    /// 左上角的 A–Z 排序按钮。
    SortAz,
    /// 文件夹弹层里的第 k 个。
    FolderCell(usize),
    /// 文件夹弹层里第 k 个的「移出」小按钮。
    FolderOut(usize),
}

/// 起动台窗口。
pub struct LaunchpadWindow {
    hwnd: HWND,
    renderer: FrameRenderer,
    input: crate::input::Input,
    pos: (i32, i32),
    ppp: f32,
    /// 窗口逻辑尺寸（= 显示器物理尺寸 / ppp）。
    size: (f32, f32),

    lp: Launchpad,
    /// 网格行列数（来自 `LpCols` / `LpRows`）。
    grid: (usize, usize),
    /// `launchpad.dat` 的路径。
    dat: PathBuf,
    page: usize,
    query: String,
    open_folder: Option<usize>,

    hover: Option<Hit>,
    hits: HitList,
    /// 当前页/文件夹里已经算出位置的东西，用来判"落点是否在图标上"。
    icon_rects: HitList,
    down: (f32, f32),
    cur: (f32, f32),
    press: Option<Hit>,
    dragging: bool,
    drag_src: Option<usize>,
    /// 拖动开始时的页码，松手时用同一页算落点。
    drag_page: usize,

    icons: HashMap<usize, egui::TextureHandle>,
    /// 已经排进图标线程的路径，避免重复排队。
    asked: HashMap<String, usize>,
    loader: Option<IconLoader>,

    /// 输入法状态（搜索框 / 重命名框）。
    ime: Ime,
    /// 右键菜单当前状态。`None` = 没开。
    menu: Option<MenuState>,
    /// 菜单左上角（逻辑点）。
    menu_at: (f32, f32),
    /// 键盘选中的格（**当前页内**的序号）。`None` = 还没用键盘选过。
    sel: Option<usize>,
    /// 当前分类筛选。`0` = 全部，`1..=tags.len()` = 第几组标签。
    cat: usize,
    /// 正在重命名谁。`None` = 重命名框没开。
    renaming: Option<RenameTarget>,
    /// 重命名缓冲。
    rename_buf: String,
    /// 待主循环处理的命令。
    cmds: Vec<LaunchpadCmd>,
    visible: bool,
    dark: bool,
    dirty_save: bool,
    next_repaint: Instant,
}

impl std::fmt::Debug for LaunchpadWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LaunchpadWindow")
            .field("hwnd", &(self.hwnd.0 as isize))
            .field("visible", &self.visible)
            .field("items", &self.lp.len())
            .finish_non_exhaustive()
    }
}

impl LaunchpadWindow {
    /// 建窗（默认隐藏）。`dat` 是 `launchpad.dat` 的完整路径。
    pub fn new(gpu: &Gpu, ppp: f32, dark: bool, dat: &Path) -> Result<Pin<Box<Self>>, GpuError> {
        let hinst =
            unsafe { GetModuleHandleW(None) }.map_err(|_| GpuError::Win32("GetModuleHandleW"))?;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: windows::Win32::Foundation::HINSTANCE(hinst.0),
            lpszClassName: class_pcwstr(),
            // 类光标留空 = 进入窗口时显示忙光标（ADR-045）。
            hCursor: crate::cursor::class_cursor(),
            ..Default::default()
        };
        unsafe {
            RegisterClassW(&raw const wc);
        }

        // 铺满主显示器。
        let mon = nooksill_platform::display::monitor_rect(0);
        let (pw, ph) = (mon.right - mon.left, mon.bottom - mon.top);
        let size = (pw as f32 / ppp, ph as f32 / ppp);

        let hwnd = unsafe {
            // 必须带 `WS_EX_LAYERED`：呈现用的是 `UpdateLayeredWindow`
            // （见 `present::LayeredPresenter`），没有这个样式它会失败，
            // 而失败会被 `textures_delta` 的 Drop 检查掩盖成一条误导性的
            // "Dropped TexturesDelta"（已顺手修掉，见 frame.rs）。
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                class_pcwstr(),
                windows::core::w!("起动台"),
                WS_POPUP | WS_VISIBLE,
                mon.left,
                mon.top,
                pw,
                ph,
                None,
                None,
                Some(windows::Win32::Foundation::HINSTANCE(hinst.0)),
                None,
            )
        }
        .map_err(|_| GpuError::Win32("CreateWindowExW（起动台）"))?;

        // 建好先藏起来：`WS_VISIBLE` 是为了让窗口过程能收消息，
        // 这里立刻隐藏，等 `open` 再显示。
        unsafe {
            let _ = ShowWindow(hwnd, windows::Win32::UI::WindowsAndMessaging::SW_HIDE);
            // 允许把文件拖进来（WM_DROPFILES）。
            DragAcceptFiles(hwnd, true);
        }

        let renderer = FrameRenderer::new(gpu, pw as u32, ph as u32, ppp)?;

        // 读盘 + 扫描。
        let mut lp = match std::fs::read_to_string(dat) {
            Ok(text) => Launchpad::parse(&text),
            Err(_) => Launchpad::new(),
        };
        lp.absorb(&nooksill_platform::launchpad::scan());

        let mut win = Box::pin(Self {
            hwnd,
            renderer,
            input: crate::input::Input::default(),
            pos: (mon.left, mon.top),
            ppp,
            size,
            lp,
            grid: (7, 4),
            dat: dat.to_path_buf(),
            page: 0,
            query: String::new(),
            open_folder: None,
            hover: None,
            hits: Vec::new(),
            icon_rects: Vec::new(),
            down: (0.0, 0.0),
            cur: (0.0, 0.0),
            press: None,
            dragging: false,
            drag_src: None,
            drag_page: 0,
            icons: HashMap::new(),
            asked: HashMap::new(),
            loader: IconLoader::spawn(Waker::new(hwnd), Some(gpu)).ok(),
            ime: Ime::new(),
            menu: None,
            menu_at: (0.0, 0.0),
            sel: None,
            cat: 0,
            renaming: None,
            rename_buf: String::new(),
            cmds: Vec::new(),
            visible: false,
            dark,
            dirty_save: false,
            next_repaint: Instant::now(),
        });
        win.attach();
        // 确保这个窗口有输入法上下文（否则拼音根本进不来）。
        Ime::enable(hwnd);
        Ok(win)
    }

    fn attach(&mut self) {
        unsafe {
            SetWindowLongPtrW(
                self.hwnd,
                GWLP_USERDATA,
                std::ptr::from_mut::<Self>(self) as isize,
            );
        }
    }

    #[must_use]
    pub const fn visible(&self) -> bool {
        self.visible
    }

    /// 打开（抢焦点，收键盘）。
    ///
    /// `frost` 为 `Some` 时：在 `ShowWindow` **之前**抓一次背景（窗口还没
    /// 上屏，GDI 不会把我们自己也抓进去），并把窗口排除出屏幕捕获 —— 之后
    /// 实时抓取线程才能读到"窗口背后"的桌面而不是面板自己。抓失败就自动
    /// 退回实心底色。
    pub fn open(&mut self, frost: Option<crate::frost::Spec>) {
        match frost {
            Some(spec) => {
                spec.apply(&mut self.renderer, self.hwnd, self.ppp);
                // 必须在 ShowWindow 之前：晚一帧就会抓到自己的镜像。
                crate::frost::exclude_from_capture(self.hwnd);
                self.renderer.start_live_frost(self.hwnd, spec, self.ppp);
            }
            None => {
                self.renderer.clear_frost();
                self.renderer.stop_live_frost();
            }
        }
        unsafe {
            // 全屏覆盖层必须压住别的窗口（包括别的 TOPMOST 窗口）：
            // 不这么做，从 Dock 打开它时常被前台窗口盖住，看起来像没反应。
            let _ = SetWindowPos(
                self.hwnd,
                Some(windows::Win32::UI::WindowsAndMessaging::HWND_TOPMOST),
                0,
                0,
                0,
                0,
                windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                    | windows::Win32::UI::WindowsAndMessaging::SWP_NOSIZE
                    | windows::Win32::UI::WindowsAndMessaging::SWP_SHOWWINDOW,
            );
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = SetForegroundWindow(self.hwnd);
        }
        self.visible = true;
        self.query.clear();
        self.page = 0;
        self.open_folder = None;
        self.hover = None;
        self.sel = None;
        self.cat = 0;
        self.menu = None;
        self.renaming = None;
        self.rename_buf.clear();
        self.next_repaint = Instant::now();
    }

    /// 关掉。
    pub fn close(&mut self) {
        if !self.visible {
            return;
        }
        self.visible = false;
        // 先停抓取再藏窗：反过来的话，隐藏与线程退出之间会多抓一帧
        // （无害，但没必要），而且 `allow_capture` 要在窗口不可见时恢复。
        self.renderer.stop_live_frost();
        crate::frost::allow_capture(self.hwnd);
        unsafe {
            let _ = ShowWindow(self.hwnd, windows::Win32::UI::WindowsAndMessaging::SW_HIDE);
        }
        if self.dirty_save {
            self.save();
        }
    }

    /// 取一条待主循环处理的命令。
    pub fn take_cmd(&mut self) -> Option<LaunchpadCmd> {
        if self.cmds.is_empty() {
            None
        } else {
            Some(self.cmds.remove(0))
        }
    }

    /// 存盘。
    pub fn save(&mut self) {
        self.dirty_save = false;
        let text = self.lp.serialize();
        if let Err(e) = std::fs::write(&self.dat, text) {
            eprintln!("[launchpad] 写 {} 失败: {e}", self.dat.display());
        }
    }

    #[must_use]
    pub fn due(&self) -> bool {
        self.visible && self.next_repaint <= Instant::now()
    }

    /// 每页几格。
    fn per(&self) -> usize {
        let (cols, rows) = self.grid_dims();
        cols * rows
    }

    fn grid_dims(&self) -> (usize, usize) {
        // 列数 / 行数来自配置（`LpCols` / `LpRows`），这里由调用方通过
        // `set_grid` 灌进来；默认 7 × 4。
        self.grid
    }

    /// 由调用方（主循环）在打开前灌入配置里的行列数。
    pub fn set_grid(&mut self, cols: usize, rows: usize) {
        self.grid = (cols.clamp(3, 12), rows.clamp(2, 8));
        self.next_repaint = Instant::now();
    }

    /// 是否处于"过滤视图"：按分类或搜索词筛选。
    ///
    /// 过滤视图里只列应用、不列文件夹 —— 分类是给应用分的，文件夹没有分类，
    /// 混进结果里只会让人困惑（原版 `LpFiltered` 同此）。
    fn filtered(&self) -> bool {
        self.cat != 0 || !self.query.is_empty()
    }

    /// 过滤视图下要显示的应用下标。
    fn shown(&self) -> Vec<usize> {
        self.lp.visible_in(self.cat, &self.query)
    }

    /// 当前显示多少格。
    fn count(&self) -> usize {
        if self.filtered() {
            self.shown().len()
        } else {
            self.lp.len()
        }
    }

    fn pages(&self) -> usize {
        Launchpad::pages(self.count(), self.per())
    }

    /// 打开某一格（应用 → 启动，文件夹 → 展开）。
    fn activate(&mut self, k: usize) {
        let idx = self.page * self.per() + k;
        if !self.filtered() {
            match self.lp.top().get(idx).copied() {
                Some(Top::App(i)) => {
                    let path = self.lp.apps()[i].path.clone();
                    nooksill_platform::sysact::open_uri(&path);
                    self.close();
                }
                Some(Top::Folder(f)) => {
                    self.open_folder = Some(f);
                    self.next_repaint = Instant::now();
                }
                None => {}
            }
        } else {
            if let Some(&ai) = self.shown().get(idx) {
                let path = self.lp.apps()[ai].path.clone();
                nooksill_platform::sysact::open_uri(&path);
                self.close();
            }
        }
    }

    /// 把拖进来的文件加进起动台。
    fn add_paths(&mut self, paths: Vec<String>) {
        for p in paths {
            let name = Path::new(&p)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.clone());
            if self.lp.add(&name, &p).is_some() {
                self.dirty_save = true;
            }
        }
        self.next_repaint = Instant::now();
    }

    /// 给当前可见的格子排图标任务（每帧最多补一批）。
    fn request_icons(&mut self) {
        let Some(loader) = &self.loader else { return };
        let per = self.per();
        let page = self.page;
        let count = self.count();

        // 收集这一页要显示的 app 下标。
        let mut want: Vec<usize> = Vec::new();
        if !self.filtered() {
            for k in 0..per {
                let idx = page * per + k;
                if idx >= count {
                    break;
                }
                match self.lp.top()[idx] {
                    Top::App(i) => want.push(i),
                    Top::Folder(f) => {
                        for &ai in &self.lp.folders()[f].apps {
                            want.push(ai);
                        }
                    }
                }
            }
        } else {
            let hits = self.shown();
            for k in 0..per {
                if let Some(&ai) = hits.get(page * per + k) {
                    want.push(ai);
                }
            }
        }

        for ai in want {
            let app = &self.lp.apps()[ai];
            if self.icons.contains_key(&ai) || self.asked.contains_key(&app.path) {
                continue;
            }
            self.asked.insert(app.path.clone(), ai);
            loader.submit(IconRequest {
                index: ai,
                kind: nooksill_core::item::ItemKind::File,
                path: app.path.clone(),
                icon_src: String::new(),
                fit: 0,
                kids: Vec::new(),
            });
        }
    }

    /// 画一帧。
    pub fn render(&mut self, gpu: &Gpu) -> Result<(), GpuError> {
        self.request_icons();
        // 取图标线程的成果**在帧内**上传。
        //
        // 放在 `ctx.run` 之前或之后都会 panic（"Dropped TexturesDelta with 1
        // unapplied deltas"）：`load_texture` 只是把"要建这张纹理"排进上下文，
        // 真正被认领是在下一次 `run` 的 `textures_delta` 里。帧外调用时
        // 这份排队的 delta 会活到某个无人认领的时刻，然后被 Drop 检查抓住。
        // 帧内调用则当场就进入本帧的 delta，跟着这一帧一起上传。
        let ready = self
            .loader
            .as_ref()
            .map(IconLoader::drain)
            .unwrap_or_default();
        let mut icons = self.icons.clone();
        let pos = self.pos;
        let hwnd = self.hwnd;
        let dark = self.dark;
        let (w, h) = self.size;
        let page = self.page;
        let query = self.query.clone();
        // 组合中的拼音单独传：它要显示出来，但不参与过滤
        // （拿 "nihao" 去过滤中文名只会得到空列表）。
        let composing = self.ime.composing().to_owned();
        let open_folder = self.open_folder;
        let hover = self.hover;
        let dragging = self.dragging;
        let drag_src = self.drag_src;
        let cur = self.cur;
        let lp = self.lp.clone();
        let grid = self.grid;
        let mut hits: HitList = Vec::new();
        let mut icons_rects: HitList = Vec::new();
        let apps = lp.apps().to_vec();
        let rename_buf = self.rename_buf.clone();
        // 选中格越界就当没选：翻页 / 搜索之后格子数会变。
        let sel = self.sel.filter(|k| *k < self.page_len());
        let menu_items = self.menu_items();
        let ov = Overlay {
            menu_items: &menu_items,
            menu_at: self.menu_at,
            sel,
            cat: self.cat,
            renaming: self.renaming,
            rename_buf: &rename_buf,
        };

        let frost = self.renderer.frost_on();
        let raw = self.input.take(self.renderer.size(), self.ppp);
        self.renderer.render(gpu, raw, hwnd, pos, |ui| {
            let ctx = ui.ctx().clone();
            for r in &ready {
                // 排队期间可能已经被移出列表：核对路径。
                if apps.get(r.index).map(|a| a.path.as_str()) != Some(r.path.as_str()) {
                    continue;
                }
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [r.width as usize, r.height as usize],
                    &r.rgba,
                );
                let tex = ctx.load_texture(
                    format!("lp-icon-{}", r.index),
                    image,
                    egui::TextureOptions::LINEAR,
                );
                icons.insert(r.index, tex);
            }
            let out = draw(
                ui,
                dark,
                (w, h),
                &lp,
                grid,
                page,
                &query,
                &composing,
                open_folder,
                hover,
                if dragging { drag_src } else { None },
                cur,
                &icons,
                ov,
                frost,
            );
            hits = out.0;
            icons_rects = out.1;
        })?;
        self.icons = icons;
        self.hits = hits;
        self.icon_rects = icons_rects;
        Ok(())
    }

    fn on_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        // 输入法消息先处理：它们和键盘消息是两条独立的通道，
        // 不先拦下来就会漏进下面的 `match`。
        if matches!(
            msg,
            windows::Win32::UI::WindowsAndMessaging::WM_IME_STARTCOMPOSITION
                | windows::Win32::UI::WindowsAndMessaging::WM_IME_COMPOSITION
                | windows::Win32::UI::WindowsAndMessaging::WM_IME_ENDCOMPOSITION
                | windows::Win32::UI::WindowsAndMessaging::WM_IME_SETCONTEXT
        ) {
            let mut committed = String::new();
            let handled = self
                .ime
                .handle(self.hwnd, msg, wparam, lparam, &mut committed);
            if !committed.is_empty() {
                // 重命名框也要能打中文。
                if self.renaming.is_some() {
                    for c in committed.chars() {
                        if self.rename_buf.chars().count() < 40 {
                            self.rename_buf.push(c);
                        }
                    }
                } else {
                    for c in committed.chars() {
                        if self.query.chars().count() < 32 {
                            self.query.push(c);
                        }
                    }
                    self.page = 0;
                    self.open_folder = None;
                }
            }
            if msg == windows::Win32::UI::WindowsAndMessaging::WM_IME_STARTCOMPOSITION {
                // 候选窗摆到搜索框下方，别让它在屏幕角落飘着。
                self.ime
                    .set_caret(self.hwnd, self.size.0 * 0.5, 40.0 + SEARCH_H, self.ppp);
            }
            self.next_repaint = Instant::now();
            if handled {
                return Some(LRESULT(0));
            }
            return Some(LRESULT(Ime::default_proc(self.hwnd, msg, wparam, lparam)));
        }
        match msg {
            // 光标：不回这一条，系统会退回默认 —— 实测是忙光标（ADR-045）。
            // 落在图标/按钮/菜单项上手型，空白处箭头。
            WM_SETCURSOR => {
                let mut on_hit = false;
                if let Some((px, py)) = crate::cursor::client_pos(self.hwnd) {
                    on_hit = self.hit_at(px / self.ppp, py / self.ppp).is_some();
                }
                crate::cursor::set_hand_if(on_hit);
                Some(LRESULT(1))
            }
            WM_APP_WAKE => {
                // 图标线程有结果了：催一次重绘。
                self.next_repaint = Instant::now();
                None
            }
            WM_MOUSEMOVE => {
                let (x, y) = client_pos(lparam, self.ppp);
                self.cur = (x, y);
                if self.press.is_some() {
                    let dx = x - self.down.0;
                    let dy = y - self.down.1;
                    if !self.dragging && (dx.abs() > 8.0 || dy.abs() > 8.0) {
                        // 只有应用能拖。
                        if let Some(Hit::Cell(k)) = self.press {
                            self.dragging = true;
                            self.drag_src = Some(k);
                            self.drag_page = self.page;
                        }
                    }
                }
                let h = self.hit_at(x, y);
                if self.hover != h || self.dragging {
                    self.hover = h;
                    self.next_repaint = Instant::now();
                }
                None
            }
            WM_LBUTTONDOWN => {
                let (x, y) = client_pos(lparam, self.ppp);
                self.down = (x, y);
                self.cur = (x, y);
                self.press = self.hit_at(x, y);
                self.dragging = false;
                self.drag_src = None;
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let (x, y) = client_pos(lparam, self.ppp);
                let released = self.hit_at(x, y);
                if std::env::var("NOOKSILL_LP_TRACE").is_ok() {
                    eprintln!(
                        "[lp] LBUTTONUP ({x:.0},{y:.0}) press={:?} released={:?} menu={:?}",
                        self.press, released, self.menu
                    );
                }
                // 菜单开着时，这一下点击只用于"选菜单项"或"点外面关掉"。
                if self.menu.is_some() {
                    // 只有"点在菜单外面"才关；点到某一项由 `run_menu_item`
                    // 自己决定下一步（二级菜单要接着显示）。
                    if let Some(Hit::MenuItem(i)) = released {
                        self.run_menu_item(i);
                    } else {
                        self.menu = None;
                    }
                    self.press = None;
                    self.next_repaint = Instant::now();
                    return Some(LRESULT(0));
                }
                // 重命名期间点别处 = 提交。
                if self.renaming.is_some() {
                    self.commit_rename();
                    self.press = None;
                    self.next_repaint = Instant::now();
                    return Some(LRESULT(0));
                }
                if self.dragging {
                    self.dragging = false;
                    self.drop(x, y);
                    self.drag_src = None;
                    self.next_repaint = Instant::now();
                    return Some(LRESULT(0));
                }
                match (self.press, released) {
                    // 在文件夹弹层里点「移出」。
                    (Some(Hit::FolderOut(a)), Some(Hit::FolderOut(b))) if a == b => {
                        if let Some(fi) = self.open_folder {
                            if self.lp.take_from_folder(fi, a, TakeOut::ToTop) {
                                self.dirty_save = true;
                                if self.lp.folders()[fi].apps.len() <= 1 {
                                    self.open_folder = None;
                                }
                            }
                        }
                        self.next_repaint = Instant::now();
                    }
                    (Some(Hit::FolderCell(a)), Some(Hit::FolderCell(b))) if a == b => {
                        if let Some(fi) = self.open_folder {
                            if let Some(&ai) = self.lp.folders()[fi].apps.get(a) {
                                let path = self.lp.apps()[ai].path.clone();
                                nooksill_platform::sysact::open_uri(&path);
                                self.close();
                            }
                        }
                    }
                    (Some(Hit::Cell(a)), Some(Hit::Cell(b))) if a == b => {
                        self.activate(a);
                    }
                    (Some(Hit::Tab(a)), Some(Hit::Tab(b))) if a == b => {
                        let n = self.lp.tags().len();
                        if b <= n {
                            // 切分类：页码与选中都归零（格子数变了）。
                            self.cat = b;
                            self.page = 0;
                            self.sel = None;
                        } else if self.lp.add_tag("").is_some() {
                            // 「＋」新加一组，先给个占位名，右键可改名。
                            self.dirty_save = true;
                        }
                        self.next_repaint = Instant::now();
                    }
                    (Some(Hit::SortAz), Some(Hit::SortAz)) => {
                        self.lp.sort_az();
                        self.dirty_save = true;
                        self.next_repaint = Instant::now();
                    }
                    (Some(Hit::Plus), Some(Hit::Plus)) => {
                        // 这里弹"添加应用/文件夹/网址"菜单。我们改成拖拽添加，
                        // 这里给出提示（点击无效果的按钮比没有按钮更让人困惑）。
                        eprintln!("[launchpad] 把文件、快捷方式或文件夹拖进窗口即可添加");
                    }
                    _ => {
                        // 点在空白处：关掉（弹层的通行做法）。
                        if released.is_none() && self.open_folder.is_none() {
                            self.close();
                        } else if released.is_none() {
                            self.open_folder = None;
                            self.next_repaint = Instant::now();
                        }
                    }
                }
                self.press = None;
                Some(LRESULT(0))
            }
            WM_RBUTTONUP => {
                let (x, y) = client_pos(lparam, self.ppp);
                // 网格里右键 = 打开这一格的菜单。
                if std::env::var("NOOKSILL_LP_TRACE").is_ok() {
                    eprintln!(
                        "[lp] RBUTTONUP ({x},{y}) hit={:?} hits={}",
                        self.hit_at(x, y),
                        self.hits.len()
                    );
                }
                if self.open_folder.is_none() {
                    // 标签栏右键 = 这一组的菜单（改名 / 删除）。
                    // 第 0 个是「全部」、最后一个是「＋」，都没有菜单。
                    if let Some(Hit::Tab(t)) = self.hit_at(x, y) {
                        if t >= 1 && t <= self.lp.tags().len() {
                            self.menu = Some(MenuState::Tag(t - 1));
                            self.menu_at = (x.min(self.size.0 - 200.0).max(8.0), y);
                            self.next_repaint = Instant::now();
                            return Some(LRESULT(0));
                        }
                    }
                    if let Some(Hit::Cell(k)) = self.hit_at(x, y) {
                        let idx = self.page * self.per() + k;
                        // 过滤视图里格子是应用；未过滤时是顶层格。
                        if self.filtered() {
                            if let Some(&ai) = self.shown().get(idx) {
                                self.menu = Some(MenuState::App(ai));
                                self.menu_at = (x.min(self.size.0 - 170.0), y);
                                self.next_repaint = Instant::now();
                                return Some(LRESULT(0));
                            }
                        } else if idx < self.lp.len() {
                            self.menu = Some(MenuState::Cell(idx));
                            self.menu_at = (x.min(self.size.0 - 170.0), y);
                            self.next_repaint = Instant::now();
                            return Some(LRESULT(0));
                        }
                    }
                }
                // 文件夹弹层里右键 = 把这一项移出文件夹。
                if let (Some(fi), Some(Hit::FolderCell(k))) = (self.open_folder, self.hit_at(x, y))
                {
                    if self.lp.take_from_folder(fi, k, TakeOut::ToTop) {
                        self.dirty_save = true;
                        if self.lp.folders()[fi].apps.len() <= 1 {
                            self.open_folder = None;
                        }
                        self.next_repaint = Instant::now();
                    }
                }
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                let delta = ((wparam.0 >> 16) & 0xffff) as u16 as i16;
                if delta > 0 {
                    self.turn_page(-1);
                } else if delta < 0 {
                    self.turn_page(1);
                }
                Some(LRESULT(0))
            }
            WM_KEYDOWN => {
                use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
                // 方向键与回车只在"没有任何浮层"时归网格；菜单、重命名、
                // 文件夹弹层各自有自己的键盘语义，不抢。
                // 只用 if 链而不是 match：没命中的键（比如 Esc）要继续往下
                // 走默认分支，`match` 里写 `_ => return None` 会把 Esc 吞掉。
                if self.menu.is_none() && self.renaming.is_none() && self.open_folder.is_none() {
                    use windows::Win32::UI::Input::KeyboardAndMouse::{
                        VK_DOWN, VK_END, VK_HOME, VK_LEFT, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT,
                        VK_UP,
                    };
                    let vk = wparam.0 as u16;
                    if vk == VK_LEFT.0 {
                        self.move_sel(-1, 0);
                        return Some(LRESULT(0));
                    } else if vk == VK_RIGHT.0 {
                        self.move_sel(1, 0);
                        return Some(LRESULT(0));
                    } else if vk == VK_UP.0 {
                        self.move_sel(0, -1);
                        return Some(LRESULT(0));
                    } else if vk == VK_DOWN.0 {
                        self.move_sel(0, 1);
                        return Some(LRESULT(0));
                    } else if vk == VK_HOME.0 {
                        self.sel = Some(0);
                        self.next_repaint = Instant::now();
                        return Some(LRESULT(0));
                    } else if vk == VK_END.0 {
                        self.sel = Some(self.page_len().saturating_sub(1));
                        self.next_repaint = Instant::now();
                        return Some(LRESULT(0));
                    } else if vk == VK_PRIOR.0 {
                        self.turn_page(-1);
                        return Some(LRESULT(0));
                    } else if vk == VK_NEXT.0 {
                        self.turn_page(1);
                        return Some(LRESULT(0));
                    } else if vk == VK_RETURN.0 {
                        self.enter_sel();
                        return Some(LRESULT(0));
                    }
                }
                if wparam.0 == VK_ESCAPE.0 as usize {
                    // 一层一层往后退：菜单 → 重命名 → 文件夹 → 窗口。
                    if self.menu.is_some() {
                        self.menu = None;
                        self.next_repaint = Instant::now();
                        return Some(LRESULT(0));
                    }
                    if self.renaming.is_some() {
                        self.renaming = None;
                        self.rename_buf.clear();
                        self.next_repaint = Instant::now();
                        return Some(LRESULT(0));
                    }
                    if self.open_folder.is_some() {
                        self.open_folder = None;
                        self.next_repaint = Instant::now();
                    } else {
                        self.close();
                    }
                }
                None
            }
            WM_CHAR => {
                let ch = wparam.0 as u32;
                // 重命名时键盘只改名字，不动搜索框。
                if self.renaming.is_some() {
                    match char::from_u32(ch) {
                        Some('\r') | Some('\n') => self.commit_rename(),
                        Some('\u{1b}') => {
                            self.renaming = None;
                            self.rename_buf.clear();
                        }
                        Some('\u{8}') => {
                            self.rename_buf.pop();
                        }
                        Some(c) if !c.is_control() && self.rename_buf.chars().count() < 40 => {
                            self.rename_buf.push(c);
                        }
                        _ => {}
                    }
                    self.next_repaint = Instant::now();
                    return Some(LRESULT(0));
                }
                match char::from_u32(ch) {
                    Some('\u{8}') => {
                        self.query.pop();
                    }
                    Some('\u{1b}') => {}
                    Some(c) if !c.is_control() && self.query.chars().count() < 32 => {
                        self.query.push(c);
                    }
                    _ => {}
                }
                self.page = 0;
                self.open_folder = None;
                // 过滤结果变了，旧的选中格不再指同一个应用。
                self.sel = None;
                self.next_repaint = Instant::now();
                None
            }
            WM_DROPFILES => {
                let hdrop = HDROP(wparam.0 as *mut core::ffi::c_void);
                let n = unsafe { DragQueryFileW(hdrop, u32::MAX, None) };
                let mut files = Vec::new();
                for i in 0..n {
                    let len = unsafe { DragQueryFileW(hdrop, i, None) };
                    let mut buf = vec![0_u16; len as usize + 1];
                    let got = unsafe { DragQueryFileW(hdrop, i, Some(buf.as_mut_slice())) };
                    if got > 0 {
                        buf.truncate(got as usize);
                        files.push(String::from_utf16_lossy(&buf));
                    }
                }
                unsafe {
                    DragFinish(hdrop);
                }
                self.add_paths(files);
                Some(LRESULT(0))
            }
            WM_PAINT => {
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

    /// 当前页有多少格。
    fn page_len(&self) -> usize {
        let per = self.per().max(1);
        (self.count().saturating_sub(self.page * per)).min(per)
    }

    /// 键盘移动选中格。
    ///
    /// 左右到头会翻页（起动台的惯性操作），上下夹在本页内 —— 上下翻页
    /// 需要额外约定"翻到下一页的第几列"，实际用起来更容易迷路。
    fn move_sel(&mut self, dx: i32, dy: i32) {
        let len = self.page_len();
        if len == 0 {
            self.sel = None;
            self.next_repaint = Instant::now();
            return;
        }
        let cols = self.grid.0.max(1);
        let k = self.sel.unwrap_or(0).min(len - 1);
        let (col, row) = (k % cols, k / cols);
        if dx != 0 {
            let next = k as i32 + dx;
            if next < 0 {
                // 左边界 → 上一页最后一格。
                if self.page > 0 {
                    self.page -= 1;
                    self.sel = Some(self.page_len().saturating_sub(1));
                }
            } else if next >= len as i32 {
                // 右边界 → 下一页第一格。
                if self.page + 1 < self.pages() {
                    self.page += 1;
                    self.sel = Some(0);
                }
            } else {
                self.sel = Some(next as usize);
            }
        } else {
            let nr = (row as i32 + dy).clamp(0, self.grid.1 as i32 - 1) as usize;
            let nk = nr * cols + col;
            if nk < len {
                self.sel = Some(nk);
            }
        }
        self.next_repaint = Instant::now();
    }

    /// 回车：打开选中的格；没选过时，搜索态直接开第一条。
    fn enter_sel(&mut self) {
        if let Some(k) = self.sel {
            if k < self.page_len() {
                self.activate(k);
                return;
            }
        }
        if self.filtered() && self.page_len() > 0 {
            self.activate(0);
        }
    }

    fn turn_page(&mut self, delta: i32) {
        if self.open_folder.is_some() {
            return;
        }
        let pages = self.pages();
        let p = self.page as i32 + delta;
        if p >= 0 && (p as usize) < pages {
            self.page = p as usize;
            self.next_repaint = Instant::now();
        }
    }

    /// 松手：算落点，合并或重排。
    fn drop(&mut self, x: f32, y: f32) {
        let Some(src) = self.drag_src else { return };
        // 只在同一页内算落点：翻页后格子的含义已经变了，原来的下标对不上。
        if let Some(Hit::Cell(k)) = self.hit_at(x, y) {
            if self.page == self.drag_page {
                if self.filtered() {
                    // 过滤视图：格子是**应用**，不是顶层格。落点语义是
                    // "插到这个应用前面"，不合并文件夹 —— 过滤结果里相邻的
                    // 两个应用在完整视图里未必相邻，就地合成文件夹会让人
                    // 回头找不到。
                    let (si, ti) = (self.page * self.per() + src, self.page * self.per() + k);
                    let shown = self.shown();
                    if let (Some(&sa), Some(&ta)) = (shown.get(si), shown.get(ti)) {
                        if self.lp.move_app_before(sa, ta) {
                            self.dirty_save = true;
                        }
                    }
                } else {
                    let on_icon = self
                        .icon_rects
                        .iter()
                        .any(|(h, r)| *h == Hit::Cell(k) && r.contains(Pos2::new(x, y)));
                    if self.lp.drop_at(src, k, on_icon) {
                        self.dirty_save = true;
                    }
                }
            }
        }
        self.press = None;
    }

    /// 当前菜单该显示哪些项。
    ///
    /// 三层：顶层格（应用 / 文件夹）、过滤视图里的应用、给应用选标签组。
    fn menu_items(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        match self.menu {
            Some(MenuState::Cell(idx)) => match self.lp.top().get(idx) {
                Some(Top::Folder(_)) => {
                    out = vec!["打开".into(), "重命名…".into(), "解散文件夹".into()];
                }
                Some(Top::App(_)) => {
                    out = vec![
                        "打开".into(),
                        "重命名…".into(),
                        "移动到标签组…".into(),
                        "添加到 Dock 栏".into(),
                        "从起动台移除".into(),
                    ];
                }
                None => {}
            },
            Some(MenuState::App(_)) => {
                out = vec![
                    "打开".into(),
                    "移动到标签组…".into(),
                    "添加到 Dock 栏".into(),
                ];
            }
            Some(MenuState::Category(ai)) => {
                let cur = self.lp.category_of(ai);
                for (i, t) in self.lp.tags().iter().enumerate() {
                    // 标出当前所在组。用中文后缀而不是 "✓"：字体里没有那个
                    // 勾，画出来是个豆腐块（ADR-045 之后同一个坑又踩了一次）。
                    out.push(if i + 1 == cur {
                        format!("{t}（当前）")
                    } else {
                        t.to_string()
                    });
                }
                out.push("自动分类（清除）".into());
                out.push("（返回）".into());
            }
            Some(MenuState::Tag(_)) => {
                out.push("重命名…".into());
                // 只剩一组时不给删：删完起动台就没有分类栏了，没有意义。
                if self.lp.tags().len() > 1 {
                    out.push("删除这一组".into());
                }
            }
            None => {}
        }
        out
    }

    /// 执行菜单第 `i` 项。
    fn run_menu_item(&mut self, i: usize) {
        if std::env::var("NOOKSILL_LP_TRACE").is_ok() {
            eprintln!("[lp] 菜单项 {i}，作用于 {:?}", self.menu);
        }
        let Some(state) = self.menu else { return };
        let labels = self.menu_items();
        let Some(label) = labels.get(i).map(String::as_str) else {
            return;
        };
        // 标签组自己的菜单。
        if let MenuState::Tag(ti) = state {
            match label {
                "重命名…" => {
                    self.rename_buf = self.lp.tags().get(ti).cloned().unwrap_or_default();
                    self.renaming = Some(RenameTarget::Tag(ti));
                }
                // clippy collapsible-match：守卫与 if 体等价（false 时同样
                // 落到 _ 分支），remove_tag 都只在匹配到本臂时执行一次。
                "删除这一组" if self.lp.remove_tag(ti) => {
                    self.dirty_save = true;
                    // 分类号是 1 起的，删掉第 `ti+1` 组之后要跟着前移；
                    // 正停在被删那组就退回"全部"，否则会指到隔壁组去。
                    if self.cat == ti + 1 {
                        self.cat = 0;
                    } else if self.cat > ti + 1 {
                        self.cat -= 1;
                    }
                    self.page = 0;
                    self.sel = None;
                }
                _ => {}
            }
            self.menu = None;
            self.next_repaint = Instant::now();
            return;
        }
        // 二级菜单：给应用挑一组标签。
        if let MenuState::Category(ai) = state {
            match label {
                "自动分类（清除）" => self.lp.set_category(ai, None),
                "（返回）" => {
                    self.menu = Some(MenuState::App(ai));
                    self.next_repaint = Instant::now();
                    return;
                }
                _ => {
                    // 标签名后面可能带「（当前）」，比对前先剪掉。
                    let bare = label.trim_end_matches("（当前）");
                    if let Some(pos) = self.lp.tags().iter().position(|t| t == bare) {
                        self.lp.set_category(ai, Some(pos + 1));
                    }
                }
            }
            self.dirty_save = true;
            self.menu = None;
            self.next_repaint = Instant::now();
            return;
        }
        // 过滤视图里的应用菜单。
        if let MenuState::App(ai) = state {
            match label {
                "打开" => {
                    let path = self.lp.apps()[ai].path.clone();
                    nooksill_platform::sysact::open_uri(&path);
                    self.close();
                }
                "移动到标签组…" => self.menu = Some(MenuState::Category(ai)),
                "添加到 Dock 栏" => {
                    self.menu = None;
                    let app = &self.lp.apps()[ai];
                    self.cmds.push(LaunchpadCmd::AddToDock {
                        name: app.name.clone(),
                        path: app.path.clone(),
                    });
                }
                _ => {}
            }
            self.next_repaint = Instant::now();
            return;
        }
        let MenuState::Cell(idx) = state else {
            return;
        };
        match label {
            "打开" => self.activate_top(idx),
            "重命名…" => {
                self.rename_buf = self.lp.name_of(self.lp.top()[idx]).to_owned();
                self.renaming = Some(RenameTarget::Cell(idx));
            }
            "添加到 Dock 栏" => {
                if let Some(Top::App(ai)) = self.lp.top().get(idx).copied() {
                    let app = &self.lp.apps()[ai];
                    self.cmds.push(LaunchpadCmd::AddToDock {
                        name: app.name.clone(),
                        path: app.path.clone(),
                    });
                }
            }
            "移动到标签组…" => {
                if let Some(Top::App(ai)) = self.lp.top().get(idx).copied() {
                    self.menu = Some(MenuState::Category(ai));
                }
            }
            "从起动台移除" => {
                if self.lp.remove_top(idx) {
                    self.dirty_save = true;
                    self.clamp_page();
                }
            }
            "解散文件夹" => {
                let changed = self.lp.dissolve(idx);
                if changed {
                    self.dirty_save = true;
                    self.clamp_page();
                }
            }
            _ => {}
        }
        self.menu = None;
        self.next_repaint = Instant::now();
    }

    /// 提交重命名（格子名或标签组名）。
    fn commit_rename(&mut self) {
        if let Some(target) = self.renaming.take() {
            let name = self.rename_buf.trim().to_owned();
            // 空名一律当取消：`rename` / `set_tag` 都会拒绝空串，但把缓冲
            // 留着不清会让人以为改成功了。
            if !name.is_empty() {
                let ok = match target {
                    RenameTarget::Cell(idx) => idx < self.lp.len() && self.lp.rename(idx, &name),
                    RenameTarget::Tag(i) => self.lp.set_tag(i, &name),
                };
                if ok {
                    self.dirty_save = true;
                }
            }
        }
        self.rename_buf.clear();
        self.next_repaint = Instant::now();
    }

    /// 删格之后页码可能越界。
    fn clamp_page(&mut self) {
        let pages = self.pages();
        if self.page >= pages {
            self.page = pages.saturating_sub(1);
        }
    }

    /// 打开第 `idx` 格（顶层下标，不受分页影响）。
    fn activate_top(&mut self, idx: usize) {
        match self.lp.top().get(idx).copied() {
            Some(Top::App(i)) => {
                let path = self.lp.apps()[i].path.clone();
                nooksill_platform::sysact::open_uri(&path);
                self.close();
            }
            Some(Top::Folder(f)) => {
                self.open_folder = Some(f);
                self.next_repaint = Instant::now();
            }
            None => {}
        }
    }

    fn hit_at(&self, x: f32, y: f32) -> Option<Hit> {
        let p = Pos2::new(x, y);
        self.hits
            .iter()
            .find(|(_, r)| r.contains(p))
            .map(|(h, _)| *h)
    }
}

/// 画整个起动台。返回（命中区，图标区）。
#[expect(
    clippy::too_many_arguments,
    reason = "argument group is the API boundary"
)]
fn draw(
    ui: &mut egui::Ui,
    dark: bool,
    (w, h): (f32, f32),
    lp: &Launchpad,
    (cols, rows): (usize, usize),
    page: usize,
    query: &str,
    composing: &str,
    open_folder: Option<usize>,
    hover: Option<Hit>,
    drag_src: Option<usize>,
    cur: (f32, f32),
    icons: &HashMap<usize, egui::TextureHandle>,
    ov: Overlay<'_>,
    frost: bool,
) -> (HitList, HitList) {
    let painter = ui.painter().clone();
    let rect = EguiRect::from_min_size(Pos2::ZERO, egui::vec2(w, h));

    // 背板：毛玻璃开着时只盖一层半透明 tint，让 `frost` 模块画的模糊桌面
    // 透上来；没开（或捕获失败）时维持原来的 246/255 近实心底色 ——
    // 那时候透出真实桌面只会看到窗口轮廓，像没画完。
    let bg = if frost {
        crate::frost::panel_bg(dark, true)
    } else if dark {
        Color32::from_rgba_unmultiplied(12, 13, 18, 246)
    } else {
        Color32::from_rgba_unmultiplied(238, 241, 248, 246)
    };
    painter.rect_filled(rect, CornerRadius::ZERO, bg);

    let tc = if dark {
        Color32::from_rgba_unmultiplied(240, 240, 245, 255)
    } else {
        Color32::from_rgba_unmultiplied(28, 28, 34, 255)
    };
    let sub = if dark {
        Color32::from_rgba_unmultiplied(200, 202, 214, 255)
    } else {
        Color32::from_rgba_unmultiplied(90, 94, 108, 255)
    };
    let accent = ui.visuals().selection.bg_fill;
    let hover_bg = if dark {
        Color32::from_rgba_unmultiplied(255, 255, 255, 46)
    } else {
        Color32::from_rgba_unmultiplied(0, 0, 0, 34)
    };

    let mut hits: HitList = Vec::new();
    let mut icon_rects: HitList = Vec::new();

    // ---- 搜索框 ----
    //
    // 改名时就地变成改名框：位置固定、不和标签栏打架，也省掉一块浮层的绘制。
    // 原来在 y=96 另画一个框，正好压在标签栏上（标签栏 92..120），两个字叠在
    // 一起谁也看不清。
    let renaming = ov.renaming.is_some();
    let search = EguiRect::from_min_size(
        Pos2::new((w - SEARCH_W) * 0.5, 40.0),
        egui::vec2(SEARCH_W, SEARCH_H),
    );
    painter.rect_filled(
        search,
        CornerRadius::same((SEARCH_H * 0.5) as u8),
        if dark {
            Color32::from_rgba_unmultiplied(255, 255, 255, 70)
        } else {
            Color32::from_rgba_unmultiplied(255, 255, 255, 150)
        },
    );
    if renaming {
        // 描一圈 accent：这是"输入态"，跟筛选用的搜索框区分开。
        painter.rect_stroke(
            search,
            CornerRadius::same((SEARCH_H * 0.5) as u8),
            Stroke::new(1.6, accent),
            egui::StrokeKind::Inside,
        );
    }
    // 搜索框：已上屏的文字 + 正在组合的拼音（拼音加下划线，让用户看得出来
    // "还在输入法里"，而不是以为程序没反应）。
    let text = if renaming {
        // 光标用竖线画：这里没有系统输入框，拿不到真实的光标闪烁。
        format!("{}|", ov.rename_buf)
    } else {
        format!("{query}{composing}")
    };
    if renaming {
        // 名字可能很长（上限 40 字），装不下就整行缩字号 —— 溢出圆角比字小
        // 更难看。
        let size = fit_size(ui, &text, SEARCH_W - 28.0, 14.0);
        let full = measure(ui, &text, size);
        painter.text(
            egui::pos2(search.center().x - full * 0.5, search.center().y),
            Align2::LEFT_CENTER,
            &text,
            FontId::proportional(size),
            tc,
        );
        // 提示放在框**外面**右侧：框内已经被名字占满，塞进去必然打架。
        painter.text(
            egui::pos2(search.right() + 14.0, search.center().y),
            Align2::LEFT_CENTER,
            "回车确认 · Esc 取消",
            FontId::proportional(11.5),
            sub,
        );
    } else if text.is_empty() {
        painter.text(
            search.center(),
            Align2::CENTER_CENTER,
            "搜索应用…（可直接打中文）",
            FontId::proportional(13.0),
            sub,
        );
    } else {
        let w_full = measure(ui, &text, 13.0);
        let left = search.center().x - w_full * 0.5;
        painter.text(
            egui::pos2(left, search.center().y),
            Align2::LEFT_CENTER,
            &text,
            FontId::proportional(13.0),
            tc,
        );
        if !composing.is_empty() {
            let w_done = measure(ui, query, 13.0);
            let y = search.center().y + 8.0;
            painter.line_segment(
                [egui::pos2(left + w_done, y), egui::pos2(left + w_full, y)],
                Stroke::new(1.0, accent),
            );
        }
    }

    // ---- 标签栏（全部 + 标签组 + ＋） ----
    //
    // 位置贴着搜索框下方：分类是"搜索的另一种过滤方式"，放一起符合直觉。
    let nt = 1 + lp.tags().len();
    let tab_h = 28.0;
    let tab_gap = 8.0;
    let tab_y = search.bottom() + 8.0;
    let cell_w = (SEARCH_W - tab_gap * nt as f32) / (nt + 1) as f32;
    for t in 0..=nt {
        let tx = search.left() + t as f32 * (cell_w + tab_gap);
        let r = EguiRect::from_min_size(Pos2::new(tx, tab_y), egui::vec2(cell_w, tab_h));
        let on = ov.cat == t && t <= lp.tags().len();
        let adding = t == nt;
        let label = if adding {
            "＋".to_owned()
        } else if t == 0 {
            "全部".to_owned()
        } else {
            lp.tags()[t - 1].clone()
        };
        let fill = if on {
            accent
        } else if hover == Some(Hit::Tab(t)) {
            hover_bg
        } else if dark {
            Color32::from_rgba_unmultiplied(255, 255, 255, 45)
        } else {
            Color32::from_rgba_unmultiplied(0, 0, 0, 26)
        };
        painter.rect_filled(r, CornerRadius::same(9), fill);
        painter.text(
            r.center(),
            Align2::CENTER_CENTER,
            fit_text(ui, &label, cell_w - 8.0, 12.5),
            FontId::proportional(12.5),
            if on { Color32::WHITE } else { sub },
        );
        // 正在改名的组：描一圈，告诉用户"改的是这一个"。
        if t >= 1 && t <= lp.tags().len() && ov.renaming == Some(RenameTarget::Tag(t - 1)) {
            painter.rect_stroke(
                r,
                CornerRadius::same(9),
                Stroke::new(2.0, accent),
                egui::StrokeKind::Inside,
            );
        }
        hits.push((Hit::Tab(t), r));
    }

    // ---- A–Z 排序按钮（搜索框左侧） ----
    let az = EguiRect::from_min_size(
        Pos2::new(search.left() - SEARCH_H - 12.0, search.top()),
        egui::vec2(SEARCH_H, SEARCH_H),
    );
    if az.left() > 8.0 {
        painter.rect_filled(
            az,
            CornerRadius::same((SEARCH_H * 0.5) as u8),
            if hover == Some(Hit::SortAz) {
                hover_bg
            } else if dark {
                Color32::from_rgba_unmultiplied(255, 255, 255, 60)
            } else {
                Color32::from_rgba_unmultiplied(0, 0, 0, 40)
            },
        );
        painter.text(
            az.center(),
            Align2::CENTER_CENTER,
            "A–Z",
            FontId::proportional(13.0),
            tc,
        );
        hits.push((Hit::SortAz, az));
    }

    // ---- 加号按钮 ----
    let plus = EguiRect::from_min_size(Pos2::new(w - 84.0, 40.0), egui::vec2(SEARCH_H, SEARCH_H));
    hits.push((Hit::Plus, plus));
    painter.rect_filled(
        plus,
        CornerRadius::same((SEARCH_H * 0.5) as u8),
        if hover == Some(Hit::Plus) {
            hover_bg
        } else if dark {
            Color32::from_rgba_unmultiplied(255, 255, 255, 60)
        } else {
            Color32::from_rgba_unmultiplied(0, 0, 0, 40)
        },
    );
    painter.text(
        plus.center(),
        Align2::CENTER_CENTER,
        "+",
        FontId::proportional(22.0),
        tc,
    );

    // ---- 网格 ----
    let per = cols * rows;
    let cw = (w - 2.0 * SIDE) / cols as f32;
    let ch = (h - TOP_H - BOT_H) / rows as f32;
    let icon_s = 88.0_f32.min(cw * 0.56).min(ch * 0.5);
    let gx = SIDE;
    let gy = TOP_H;

    // 过滤视图 = 有搜索词**或**选了分类。两者都为空时才按"顶层格 + 文件夹"
    // 显示，否则只列应用（分类是给应用分的，文件夹没有分类）。
    let filtering = ov.cat != 0 || !query.is_empty();
    let filtered: Vec<usize> = if filtering {
        lp.visible_in(ov.cat, query)
    } else {
        Vec::new()
    };
    let count = if filtering { filtered.len() } else { lp.len() };

    for k in 0..per {
        let idx = page * per + k;
        if idx >= count {
            break;
        }
        let col = k % cols;
        let row = k / cols;
        let cell = EguiRect::from_min_size(
            Pos2::new(gx + col as f32 * cw, gy + row as f32 * ch),
            egui::vec2(cw, ch),
        );
        if hover == Some(Hit::Cell(k)) && drag_src.is_none() {
            painter.rect_filled(
                EguiRect::from_min_max(
                    cell.min + egui::vec2(4.0, 2.0),
                    cell.max - egui::vec2(4.0, 8.0),
                ),
                CornerRadius::same(16),
                hover_bg,
            );
        }
        // 正在改名的那一格：描一圈。改的是应用 / 文件夹名，没有这圈的话
        // 用户只看到顶上多了个输入框，不知道在改谁。
        if !filtering && ov.renaming == Some(RenameTarget::Cell(idx)) {
            painter.rect_stroke(
                EguiRect::from_min_max(
                    cell.min + egui::vec2(4.0, 2.0),
                    cell.max - egui::vec2(4.0, 8.0),
                ),
                CornerRadius::same(16),
                Stroke::new(2.0, accent),
                egui::StrokeKind::Inside,
            );
        }

        let icon_rect = EguiRect::from_min_size(
            Pos2::new(
                cell.center().x - icon_s * 0.5,
                cell.top() + (ch - icon_s - 34.0) * 0.5,
            ),
            egui::vec2(icon_s, icon_s),
        );
        icon_rects.push((Hit::Cell(k), icon_rect));
        // 格子本体也要进 `hits`（图标区是 `icon_rects`，只用来判"落在图标上"）。
        // 漏了这一步的表现是：点图标毫无反应、右键也不出菜单 —— 因为
        // `hit_at` 只查 `hits`，而它当时只有右上角的「＋」。
        hits.push((Hit::Cell(k), cell));

        let (name, app_idx, is_folder, folder) = if !filtering {
            match lp.top()[idx] {
                Top::App(i) => (lp.apps()[i].name.clone(), Some(i), false, None),
                Top::Folder(f) => (lp.folders()[f].name.clone(), None, true, Some(f)),
            }
        } else {
            let ai = filtered[idx];
            (lp.apps()[ai].name.clone(), Some(ai), false, None)
        };

        if is_folder {
            draw_folder_icon(&painter, &icon_rect, lp, folder.unwrap_or(0), icons);
        } else if let Some(i) = app_idx {
            if let Some(tex) = icons.get(&i) {
                painter.image(
                    tex.id(),
                    icon_rect,
                    EguiRect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                draw_letter_tile(&painter, &icon_rect, &name, dark);
            }
        }

        // 名字（超宽截断）—— 拖着的那个画淡一点。
        let ghost = drag_src == Some(k);
        painter.text(
            Pos2::new(cell.center().x, icon_rect.bottom() + 8.0),
            Align2::CENTER_TOP,
            fit_text(ui, &name, cw - 12.0, 11.5),
            FontId::proportional(11.5),
            if ghost { sub } else { tc },
        );

        // 键盘选中的格：图标外面一圈高亮。悬停另有底色，两者不冲突。
        if ov.sel == Some(k) && drag_src.is_none() {
            let halo = icon_rect.expand(icon_s * 0.1);
            painter.rect_stroke(
                halo,
                CornerRadius::same((icon_s * 0.24) as u8),
                Stroke::new(2.5, accent),
                egui::StrokeKind::Outside,
            );
        }
    }

    // 空态
    if count == 0 {
        painter.text(
            Pos2::new(w * 0.5, h * 0.5 - 20.0),
            Align2::CENTER_CENTER,
            if !query.is_empty() {
                "没有找到匹配的应用"
            } else if ov.cat != 0 {
                "这一组里还没有应用 —— 右键图标可以把它移过来"
            } else {
                "起动台是空的 —— 把文件、快捷方式或文件夹拖进来"
            },
            FontId::proportional(14.0),
            sub,
        );
    }

    // ---- 页码点 ----
    let pages = Launchpad::pages(count, per);
    if pages > 1 {
        let start = w * 0.5 - (pages - 1) as f32 * 11.0;
        for p in 0..pages.min(40) {
            painter.circle_filled(
                Pos2::new(start + p as f32 * 22.0, h - 104.0),
                4.0,
                if p == page { tc } else { sub },
            );
        }
    }

    // 拖动时图标跟手。
    if let Some(k) = drag_src {
        let idx = page * per + k;
        if filtering {
            // 过滤视图：格子直接是应用，画它的图标就行。
            if let Some(&ai) = filtered.get(idx) {
                let s = icon_s * 1.1;
                let r = EguiRect::from_min_size(
                    Pos2::new(cur.0 - s * 0.5, cur.1 - s * 0.5),
                    egui::vec2(s, s),
                );
                if let Some(tex) = icons.get(&ai) {
                    painter.image(
                        tex.id(),
                        r,
                        EguiRect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        Color32::from_rgba_unmultiplied(255, 255, 255, 220),
                    );
                } else {
                    draw_letter_tile(&painter, &r, &lp.apps()[ai].name, dark);
                }
            }
        } else {
            if let Some(t) = lp.top().get(idx).copied() {
                let s = icon_s * 1.1;
                let r = EguiRect::from_min_size(
                    Pos2::new(cur.0 - s * 0.5, cur.1 - s * 0.5),
                    egui::vec2(s, s),
                );
                match t {
                    Top::App(i) => {
                        if let Some(tex) = icons.get(&i) {
                            painter.image(
                                tex.id(),
                                r,
                                EguiRect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
                                Color32::from_rgba_unmultiplied(255, 255, 255, 220),
                            );
                        } else {
                            draw_letter_tile(&painter, &r, &lp.apps()[i].name, dark);
                        }
                    }
                    Top::Folder(f) => draw_folder_icon(&painter, &r, lp, f, icons),
                }
                // 落到图标上时给对方描个框。
                if let Some(Hit::Cell(tk)) = hit_cell_in(per, cols, gx, gy, cw, ch, cur) {
                    if tk < count && tk != k {
                        let cell = EguiRect::from_min_size(
                            Pos2::new(gx + (tk % cols) as f32 * cw, gy + (tk / cols) as f32 * ch),
                            egui::vec2(cw, ch),
                        );
                        if cell.contains(Pos2::new(cur.0, cur.1)) {
                            let halo = EguiRect::from_min_size(
                                Pos2::new(
                                    gx + (tk % cols) as f32 * cw + cw * 0.5 - icon_s * 0.62,
                                    gy + (tk / cols) as f32 * ch + (ch - icon_s - 34.0) * 0.5
                                        - icon_s * 0.12,
                                ),
                                egui::vec2(icon_s * 1.24, icon_s * 1.24),
                            );
                            painter.rect_stroke(
                                halo,
                                CornerRadius::same((icon_s * 0.3) as u8),
                                Stroke::new(2.5, accent),
                                egui::StrokeKind::Outside,
                            );
                        }
                    }
                }
            }
        }
    }

    // ---- 文件夹弹层 ----
    if let Some(fi) = open_folder {
        draw_folder_panel(
            ui,
            dark,
            (w, h),
            lp,
            fi,
            cols,
            cw,
            ch,
            icon_s,
            icons,
            hover,
            &mut hits,
        );
    }

    // ---- 右键菜单 / 重命名框 ----
    //
    // 两者都画在同一个窗口里，不另开 HWND：这样命中判定、输入法、重绘
    // 全都走同一套，省掉一个窗口类与一份消息循环。
    let mut overlay_hits: HitList = Vec::new();
    if !ov.menu_items.is_empty() {
        let labels = ov.menu_items;
        let row_h = 28.0;
        let pad = 6.0;
        // 宽度按最长一项量：标签名是用户起的（"办公"/"设计素材"…），
        // 固定 170 会把长名字截断。
        let widest = labels
            .iter()
            .map(|l| measure(ui, l, 12.5))
            .fold(170.0_f32 - 34.0, f32::max);
        let mw = (widest + 34.0).max(170.0).min(w - 24.0);
        let mh = labels.len() as f32 * row_h + pad * 2.0;
        let mx = ov.menu_at.0.min(w - mw - 8.0).max(8.0);
        let my = ov.menu_at.1.min(h - mh - 8.0).max(8.0);
        let panel = EguiRect::from_min_size(Pos2::new(mx, my), egui::vec2(mw, mh));
        painter.rect_filled(
            panel,
            CornerRadius::same(10),
            if dark {
                Color32::from_rgba_unmultiplied(46, 47, 54, 252)
            } else {
                Color32::from_rgba_unmultiplied(252, 252, 255, 252)
            },
        );
        painter.rect_stroke(
            panel,
            CornerRadius::same(10),
            Stroke::new(1.0, sub),
            egui::StrokeKind::Inside,
        );
        for (i, label) in labels.iter().enumerate() {
            let r = EguiRect::from_min_size(
                Pos2::new(mx + 4.0, my + pad + i as f32 * row_h),
                egui::vec2(mw - 8.0, row_h),
            );
            if hover == Some(Hit::MenuItem(i)) {
                painter.rect_filled(r, CornerRadius::same(6), hover_bg);
            }
            painter.text(
                egui::pos2(r.left() + 10.0, r.center().y),
                Align2::LEFT_CENTER,
                label.as_str(),
                FontId::proportional(12.5),
                if label == "从起动台移除" {
                    Color32::from_rgba_unmultiplied(230, 74, 70, 255)
                } else {
                    tc
                },
            );
            overlay_hits.push((Hit::MenuItem(i), r));
        }
    }
    // 菜单/重命名框的命中区排在前面：它们浮在网格之上。
    overlay_hits.extend(hits);
    (overlay_hits, icon_rects)
}

/// 拖动时算指针落在哪一格（与 `hit_at` 不同：拖动中命中区还没刷新）。
fn hit_cell_in(
    per: usize,
    cols: usize,
    gx: f32,
    gy: f32,
    cw: f32,
    ch: f32,
    p: (f32, f32),
) -> Option<Hit> {
    if p.0 < gx || p.1 < gy {
        return None;
    }
    let col = ((p.0 - gx) / cw) as usize;
    let row = ((p.1 - gy) / ch) as usize;
    if col >= cols || row >= (per / cols) {
        return None;
    }
    Some(Hit::Cell(row * cols + col))
}

#[expect(
    clippy::too_many_arguments,
    reason = "argument group is the API boundary"
)]
fn draw_folder_panel(
    ui: &mut egui::Ui,
    dark: bool,
    (w, h): (f32, f32),
    lp: &Launchpad,
    fi: usize,
    _cols: usize,
    cw: f32,
    ch: f32,
    icon_s: f32,
    icons: &HashMap<usize, egui::TextureHandle>,
    hover: Option<Hit>,
    hits: &mut HitList,
) {
    let painter = ui.painter().clone();
    let tc = if dark {
        Color32::from_rgba_unmultiplied(240, 240, 245, 255)
    } else {
        Color32::from_rgba_unmultiplied(28, 28, 34, 255)
    };
    let sub = if dark {
        Color32::from_rgba_unmultiplied(200, 202, 214, 255)
    } else {
        Color32::from_rgba_unmultiplied(90, 94, 108, 255)
    };
    let hover_bg = if dark {
        Color32::from_rgba_unmultiplied(255, 255, 255, 46)
    } else {
        Color32::from_rgba_unmultiplied(0, 0, 0, 34)
    };

    let members = &lp.folders()[fi].apps;
    let pc = members.len().clamp(1, 5);
    let pr = members.len().div_ceil(5).max(1);
    let pw = pc as f32 * cw + 60.0;
    let ph = pr as f32 * ch + 96.0;
    let panel = EguiRect::from_min_size(
        Pos2::new((w - pw) * 0.5, (h - ph) * 0.5),
        egui::vec2(pw, ph),
    );

    // 遮罩 + 面板
    painter.rect_filled(
        EguiRect::from_min_size(Pos2::ZERO, egui::vec2(w, h)),
        CornerRadius::ZERO,
        Color32::from_rgba_unmultiplied(0, 0, 0, 120),
    );
    painter.rect_filled(
        panel,
        CornerRadius::same(28),
        if dark {
            Color32::from_rgba_unmultiplied(40, 41, 48, 246)
        } else {
            Color32::from_rgba_unmultiplied(246, 247, 252, 246)
        },
    );
    painter.text(
        Pos2::new(panel.center().x, panel.top() + 30.0),
        Align2::CENTER_CENTER,
        &lp.folders()[fi].name,
        FontId::proportional(15.0),
        tc,
    );
    painter.text(
        Pos2::new(panel.center().x, panel.top() + 56.0),
        Align2::CENTER_CENTER,
        "滚轮 / Esc 关闭 · 右键把应用移出文件夹",
        FontId::proportional(10.5),
        sub,
    );

    for (k, &ai) in members.iter().enumerate() {
        let x = panel.left() + 30.0 + (k % 5) as f32 * cw;
        let y = panel.top() + 76.0 + (k / 5) as f32 * ch;
        let cell = EguiRect::from_min_size(Pos2::new(x, y), egui::vec2(cw, ch));
        if hover == Some(Hit::FolderCell(k)) {
            painter.rect_filled(
                EguiRect::from_min_max(
                    cell.min + egui::vec2(4.0, 2.0),
                    cell.max - egui::vec2(4.0, 8.0),
                ),
                CornerRadius::same(16),
                hover_bg,
            );
        }
        hits.push((Hit::FolderCell(k), cell));

        let ir = EguiRect::from_min_size(
            Pos2::new(
                cell.center().x - icon_s * 0.5,
                y + (ch - icon_s - 34.0) * 0.5,
            ),
            egui::vec2(icon_s, icon_s),
        );
        if let Some(tex) = icons.get(&ai) {
            painter.image(
                tex.id(),
                ir,
                EguiRect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            draw_letter_tile(&painter, &ir, &lp.apps()[ai].name, dark);
        }
        painter.text(
            Pos2::new(cell.center().x, ir.bottom() + 8.0),
            Align2::CENTER_TOP,
            fit_text(ui, &lp.apps()[ai].name, cw - 12.0, 11.5),
            FontId::proportional(11.5),
            tc,
        );

        // 悬停时右上角出现「−」：点它把这一项移出文件夹。
        if hover == Some(Hit::FolderCell(k)) || hover == Some(Hit::FolderOut(k)) {
            let out = EguiRect::from_center_size(
                Pos2::new(ir.right() - 6.0, ir.top() + 6.0),
                egui::vec2(18.0, 18.0),
            );
            painter.circle_filled(
                out.center(),
                9.0,
                Color32::from_rgba_unmultiplied(0, 0, 0, 150),
            );
            painter.line_segment(
                [
                    Pos2::new(out.center().x - 4.0, out.center().y),
                    Pos2::new(out.center().x + 4.0, out.center().y),
                ],
                Stroke::new(1.6, Color32::WHITE),
            );
            hits.push((Hit::FolderOut(k), out));
        }
    }
}

/// 文件夹图标：半透明圆角方块里放 3×3 的小图标。
fn draw_folder_icon(
    painter: &egui::Painter,
    r: &EguiRect,
    lp: &Launchpad,
    fi: usize,
    icons: &HashMap<usize, egui::TextureHandle>,
) {
    painter.rect_filled(
        *r,
        CornerRadius::same((r.width() * 0.24) as u8),
        Color32::from_rgba_unmultiplied(255, 255, 255, 190),
    );
    let s = r.width();
    let cell = s * 0.26;
    let gap = s * 0.05;
    let off = (s - 3.0 * cell - 2.0 * gap) * 0.5;
    for (k, &ai) in lp.folders()[fi].apps.iter().take(9).enumerate() {
        let x = r.left() + off + (k % 3) as f32 * (cell + gap);
        let y = r.top() + off + (k / 3) as f32 * (cell + gap);
        let sub_r = EguiRect::from_min_size(Pos2::new(x, y), egui::vec2(cell, cell));
        if let Some(tex) = icons.get(&ai) {
            painter.image(
                tex.id(),
                sub_r,
                EguiRect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            painter.rect_filled(sub_r, CornerRadius::same(3), Color32::from_gray(160));
        }
    }
}

/// 图标还没取到时的占位：圆角块 + 首字母。
fn draw_letter_tile(painter: &egui::Painter, r: &EguiRect, name: &str, dark: bool) {
    // 用名字的哈希挑一个稳定的色相，免得满屏同色。
    let mut h: u32 = 0;
    for b in name.bytes() {
        h = h.wrapping_mul(31).wrapping_add(u32::from(b));
    }
    let hue = (h % 360) as f32;
    let (rr, gg, bb) = hsl_to_rgb(hue, 0.45, if dark { 0.42 } else { 0.68 });
    painter.rect_filled(
        *r,
        CornerRadius::same((r.width() * 0.22) as u8),
        Color32::from_rgba_unmultiplied(rr, gg, bb, 255),
    );
    let ch = name.chars().next().unwrap_or('?');
    painter.text(
        r.center(),
        Align2::CENTER_CENTER,
        ch.to_uppercase().to_string(),
        FontId::proportional(r.width() * 0.44),
        Color32::WHITE,
    );
}

/// HSL → RGB（只要够用，不求精确）。
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c * 0.5;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

/// 量一段文字的宽度（逻辑点）。
fn measure(ui: &egui::Ui, text: &str, size: f32) -> f32 {
    ui.ctx().fonts_mut(|f| {
        f.layout_no_wrap(text.to_owned(), FontId::proportional(size), Color32::WHITE)
            .size()
            .x
    })
}

/// 找一个能把 `text` 塞进 `max_w` 的字号（下限 9 点）。
///
/// 改名框里不截断名字：名字是用户正在看的东西，少一个字都可能改错。
/// 缩字号比截断安全。
fn fit_size(ui: &egui::Ui, text: &str, max_w: f32, size: f32) -> f32 {
    let w = measure(ui, text, size);
    if w <= max_w || w <= 0.0 {
        size
    } else {
        (size * max_w / w).max(9.0)
    }
}

/// 超宽就截断加省略号。egui 的 `Painter::text` 不会自己裁，长名字会压到隔壁格子。
fn fit_text(ui: &egui::Ui, text: &str, max_w: f32, size: f32) -> String {
    if measure(ui, text, size) <= max_w {
        return text.to_owned();
    }
    let mut out: Vec<char> = text.chars().collect();
    while !out.is_empty() {
        out.pop();
        let candidate: String = out.iter().collect::<String>() + "…";
        if measure(ui, &candidate, size) <= max_w {
            return candidate;
        }
    }
    "…".to_owned()
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut LaunchpadWindow;
    if ptr.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    // SAFETY: 指针由 `attach` 挂在 `Pin<Box<Self>>` 的最终地址上（ADR-036）。
    let win = unsafe { &mut *ptr };
    if let Some(result) = win.on_message(msg, wparam, lparam) {
        return result;
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

impl Drop for LaunchpadWindow {
    fn drop(&mut self) {
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
        }
    }
}

fn class_pcwstr() -> PCWSTR {
    static BUF: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    let buf = BUF.get_or_init(|| {
        CLASS_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect()
    });
    PCWSTR(buf.as_ptr())
}

/// 客户区坐标（物理像素）→ 逻辑点。
fn client_pos(lparam: LPARAM, ppp: f32) -> (f32, f32) {
    let x = (lparam.0 & 0xffff) as u16 as i16 as f32;
    let y = ((lparam.0 >> 16) & 0xffff) as u16 as i16 as f32;
    (x / ppp, y / ppp)
}

/// 供调用方判断这个窗口是不是起动台。
#[must_use]
pub fn is_launchpad_window(hwnd: HWND) -> bool {
    let mut buf = [0_u16; 64];
    let n = unsafe { windows::Win32::UI::WindowsAndMessaging::GetClassNameW(hwnd, &mut buf) };
    n > 0 && String::from_utf16_lossy(&buf[..n as usize]) == CLASS_NAME
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 色相环的几个关键点。
    #[test]
    fn hsl_hits_primary_colors() {
        assert_eq!(hsl_to_rgb(0.0, 1.0, 0.5), (255, 0, 0));
        assert_eq!(hsl_to_rgb(120.0, 1.0, 0.5), (0, 255, 0));
        assert_eq!(hsl_to_rgb(240.0, 1.0, 0.5), (0, 0, 255));
        // 灰色：饱和度为 0 时三通道相同。
        let (r, g, b) = hsl_to_rgb(200.0, 0.0, 0.5);
        assert_eq!((r, g), (b, b));
    }

    /// 拖动落点：格心附近才算"落在图标上"，格边角算"落在空白"。
    #[test]
    fn drop_hit_test_maps_to_a_cell() {
        // 2 行 3 列，起点 (70,124)，格 100×120。
        let h = hit_cell_in(6, 3, 70.0, 124.0, 100.0, 120.0, (175.0, 250.0));
        assert_eq!(h, Some(Hit::Cell(4)));
        // 左边留白里不算。
        assert_eq!(
            hit_cell_in(6, 3, 70.0, 124.0, 100.0, 120.0, (10.0, 200.0)),
            None
        );
        // 网格下面不算。
        assert_eq!(
            hit_cell_in(6, 3, 70.0, 124.0, 100.0, 120.0, (175.0, 400.0)),
            None
        );
    }
}
