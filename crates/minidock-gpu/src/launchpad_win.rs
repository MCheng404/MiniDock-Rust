//! 起动台：全屏的应用网格。
//!
//! 对应上游 v1.6.0 的 `ShowLaunchpad()` / `LpDraw()` / `LpProc()` /
//! `LpDrop()`（minidock.c:6757-7350）。数据模型与"拖到图标上合并成文件夹"
//! 的规则在 `minidock_core::launchpad`，这里只管画、判命中、收输入。
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
use minidock_core::launchpad::{Launchpad, TakeOut, Top};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, GetWindowLongPtrW, RegisterClassW,
    SetForegroundWindow, SetWindowLongPtrW, ShowWindow, GWLP_USERDATA, SW_SHOW, WM_APP, WM_CHAR,
    WM_DROPFILES, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_PAINT, WM_RBUTTONUP, WNDCLASSW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_POPUP,
    WS_VISIBLE,
};

use crate::frame::FrameRenderer;
use crate::gpu::{Gpu, GpuError};
use crate::worker::{IconLoader, IconRequest, Waker};

/// 窗口类名。
pub const CLASS_NAME: &str = "MiniDockLaunchpad";

/// 图标线程叫醒我们用的消息（与 Dock 用同一个约定）。
const WM_APP_WAKE: u32 = WM_APP + 1;

/// 搜索框。
const SEARCH_W: f32 = 380.0;
const SEARCH_H: f32 = 44.0;
/// 网格左右留白与上下留白（上游 `side` / `topH` / `botH`）。
const SIDE: f32 = 70.0;
const TOP_H: f32 = 124.0;
const BOT_H: f32 = 146.0;

/// 命中区列表：`(目标, 逻辑矩形)`。绘制时算出来，鼠标事件里用。
type HitList = Vec<(Hit, EguiRect)>;

/// 一次命中的目标。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Hit {
    /// 网格里的第 k 格（当前页内的序号）。
    Cell(usize),
    /// 右上角的「＋」。
    Plus,
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
            ..Default::default()
        };
        unsafe {
            RegisterClassW(&raw const wc);
        }

        // 铺满主显示器。
        let mon = minidock_platform::display::monitor_rect(0);
        let (pw, ph) = (mon.right - mon.left, mon.bottom - mon.top);
        let size = (pw as f32 / ppp, ph as f32 / ppp);

        let hwnd = unsafe {
            // 必须带 `WS_EX_LAYERED`：呈现用的是 `UpdateLayeredWindow`
            // （见 `present::LayeredPresenter`），没有这个样式它会失败，
            // 而失败会被 `textures_delta` 的 Drop 检查掩盖成一条误导性的
            // "Dropped TexturesDelta"（已顺手修掉，见 frame.rs）。
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW,
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
        // 这里立刻隐藏，等 `open()` 再显示。
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
        lp.absorb(&minidock_platform::launchpad::scan());

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
            loader: IconLoader::spawn(Waker::new(hwnd)).ok(),
            visible: false,
            dark,
            dirty_save: false,
            next_repaint: Instant::now(),
        });
        win.attach();
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
    pub fn open(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = SetForegroundWindow(self.hwnd);
        }
        self.visible = true;
        self.query.clear();
        self.page = 0;
        self.open_folder = None;
        self.hover = None;
        self.next_repaint = Instant::now();
    }

    /// 关掉。
    pub fn close(&mut self) {
        if !self.visible {
            return;
        }
        self.visible = false;
        unsafe {
            let _ = ShowWindow(self.hwnd, windows::Win32::UI::WindowsAndMessaging::SW_HIDE);
        }
        if self.dirty_save {
            self.save();
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

    /// 当前显示多少格（搜索时是过滤结果的数量）。
    fn count(&self) -> usize {
        if self.query.is_empty() {
            self.lp.len()
        } else {
            self.lp.filter(&self.query).len()
        }
    }

    fn pages(&self) -> usize {
        Launchpad::pages(self.count(), self.per())
    }

    /// 打开某一格（应用 → 启动，文件夹 → 展开）。
    fn activate(&mut self, k: usize) {
        let idx = self.page * self.per() + k;
        if self.query.is_empty() {
            match self.lp.top().get(idx).copied() {
                Some(Top::App(i)) => {
                    let path = self.lp.apps()[i].path.clone();
                    minidock_platform::sysact::open_uri(&path);
                    self.close();
                }
                Some(Top::Folder(f)) => {
                    self.open_folder = Some(f);
                    self.next_repaint = Instant::now();
                }
                None => {}
            }
        } else {
            let hits = self.lp.filter(&self.query);
            if let Some(&ai) = hits.get(idx) {
                let path = self.lp.apps()[ai].path.clone();
                minidock_platform::sysact::open_uri(&path);
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
        if self.query.is_empty() {
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
            let hits = self.lp.filter(&self.query);
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
                kind: minidock_core::item::ItemKind::File,
                path: app.path.clone(),
                fit: 0,
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
                open_folder,
                hover,
                if dragging { drag_src } else { None },
                cur,
                &icons,
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
        match msg {
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
                        // 只有应用能拖（文件夹拖动在上游也是重排，这里先只支持应用）。
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
                                minidock_platform::sysact::open_uri(&path);
                                self.close();
                            }
                        }
                    }
                    (Some(Hit::Cell(a)), Some(Hit::Cell(b))) if a == b => {
                        self.activate(a);
                    }
                    (Some(Hit::Plus), Some(Hit::Plus)) => {
                        // 上游这里弹"添加应用/文件夹/网址"菜单。我们改成拖拽添加，
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
                // 文件夹弹层里右键 = 把这一项移出文件夹（不做菜单，
                // 菜单要另开一个窗口，代价与收益不成比例）。
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
                if wparam.0 == VK_ESCAPE.0 as usize {
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
        if self.query.is_empty() {
            // 只在同一页内算落点。
            if let Some(Hit::Cell(k)) = self.hit_at(x, y) {
                if self.page == self.drag_page {
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

    fn hit_at(&self, x: f32, y: f32) -> Option<Hit> {
        let p = Pos2::new(x, y);
        self.hits
            .iter()
            .find(|(_, r)| r.contains(p))
            .map(|(h, _)| *h)
    }

    /// 客户区尺寸（逻辑点）。
    #[allow(dead_code)]
    fn client_size(&self) -> (f32, f32) {
        let mut rc = windows::Win32::Foundation::RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rc);
        }
        (rc.right as f32 / self.ppp, rc.bottom as f32 / self.ppp)
    }
}

/// 画整个起动台。返回（命中区，图标区）。
#[allow(clippy::too_many_arguments)]
fn draw(
    ui: &mut egui::Ui,
    dark: bool,
    (w, h): (f32, f32),
    lp: &Launchpad,
    (cols, rows): (usize, usize),
    page: usize,
    query: &str,
    open_folder: Option<usize>,
    hover: Option<Hit>,
    drag_src: Option<usize>,
    cur: (f32, f32),
    icons: &HashMap<usize, egui::TextureHandle>,
) -> (HitList, HitList) {
    let painter = ui.painter().clone();
    let rect = EguiRect::from_min_size(Pos2::ZERO, egui::vec2(w, h));

    // 背板：深色下 91% 的深灰，浅色下 91% 的浅灰。上游在磨砂可用时用磨砂，
    // 我们这里不拍窗口快照，用不透明度换观感。
    // 不透明度取 246/255（约 96%）：我们没有上游那种"磨砂快照"可用，
    // 半透明会让背后的窗口轮廓透进来，看起来像没画完。留一点点透，
    // 只为让它在切换时不像一块实心板。
    let bg = if dark {
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
    painter.text(
        search.center(),
        Align2::CENTER_CENTER,
        if query.is_empty() {
            "搜索应用…（直接输入文字）".to_owned()
        } else {
            format!("{query}|")
        },
        FontId::proportional(13.0),
        if query.is_empty() { sub } else { tc },
    );

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

    let filtered: Vec<usize> = if query.is_empty() {
        Vec::new()
    } else {
        lp.filter(query)
    };
    let count = if query.is_empty() {
        lp.len()
    } else {
        filtered.len()
    };

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

        let icon_rect = EguiRect::from_min_size(
            Pos2::new(
                cell.center().x - icon_s * 0.5,
                cell.top() + (ch - icon_s - 34.0) * 0.5,
            ),
            egui::vec2(icon_s, icon_s),
        );
        icon_rects.push((Hit::Cell(k), icon_rect));

        let (name, app_idx, is_folder, folder) = if query.is_empty() {
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
    }

    // 空态
    if count == 0 {
        painter.text(
            Pos2::new(w * 0.5, h * 0.5 - 20.0),
            Align2::CENTER_CENTER,
            if query.is_empty() {
                "起动台是空的 —— 把文件、快捷方式或文件夹拖进来"
            } else {
                "没有找到匹配的应用"
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
        if query.is_empty() {
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

    (hits, icon_rects)
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

#[allow(clippy::too_many_arguments)]
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

/// 超宽就截断加省略号。egui 的 `Painter::text` 不会自己裁，长名字会压到隔壁格子。
fn fit_text(ui: &egui::Ui, text: &str, max_w: f32, size: f32) -> String {
    let font = FontId::proportional(size);
    let measure = |s: &str| {
        ui.ctx().fonts_mut(|f| {
            f.layout_no_wrap(s.to_owned(), font.clone(), Color32::WHITE)
                .size()
                .x
        })
    };
    if measure(text) <= max_w {
        return text.to_owned();
    }
    let mut out: Vec<char> = text.chars().collect();
    while !out.is_empty() {
        out.pop();
        let candidate: String = out.iter().collect::<String>() + "…";
        if measure(&candidate) <= max_w {
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
