//! 分组面板：点开 Dock 上的一个"目录"，看它里面装了什么。
//!
//! 与其它面板（日历 / 控制中心 / 资源监视器 / 计时器）同一套做法：进程内
//! 第二个分层窗口，共用 GPU 设备，自己的 `FrameRenderer`。
//!
//! # 图标
//!
//! 面板有**自己的** `egui::Context`，纹理不能跨上下文用，所以它自己起一条
//! 取图标的线程（与起动台同一做法）。Dock 上那个四宫格组图标是另一条
//! 链路（`worker` 里把子项拼成一张图），两者互不干扰。
//!
//! # 交互
//!
//! | 操作 | 结果 |
//! |---|---|
//! | 左键点子项 | 启动它 |
//! | 悬停时格子右上角的 × | 从分组里移出 |
//! | 标题栏的「解散分组」 | 子项全部放回栏体，分组消失 |
//! | 点面板外面 | 收起（受 `[widgets] PanelDismiss` 控制）|

use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::time::Instant;

use egui::{Align2, Color32, CornerRadius, FontId, Rect as EguiRect, Stroke};
use nooksill_core::item::ItemKind;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetWindowLongPtrW, RegisterClassW, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, GWLP_USERDATA, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE,
    SW_SHOW, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR, WNDCLASSW, WS_EX_APPWINDOW, WS_EX_LAYERED,
    WS_EX_TOPMOST, WS_POPUP,
};

use crate::frame::FrameRenderer;
use crate::gpu::{Gpu, GpuError};
use crate::worker::{IconLoader, IconRequest, Waker, WM_APP_WAKE};

/// 窗口类名。
const CLASS_NAME: &str = "NookSillGroupPanel";

/// 每行放几个。
pub const COLS: usize = 4;
/// 一个格子的宽（逻辑像素）。
const CELL_W: f32 = 104.0;
/// 一个格子的高（逻辑像素）。
const CELL_H: f32 = 104.0;
/// 图标边长。
const ICON_S: f32 = 60.0;
/// 四周留白。
const PAD: f32 = 16.0;
/// 标题栏高度。
const HEADER_H: f32 = 40.0;

/// 面板里发生的一个动作。主循环取走后执行。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupAction {
    /// 没动作。
    None,
    /// 启动第 `k` 个子项。
    Launch(usize),
    /// 把第 `k` 个子项移出分组。
    Remove(usize),
    /// 在分组里把第 `k` 个子项上移（`-1`）或下移（`+1`）。
    Move(usize, i32),
    /// 解散整个分组。
    Dissolve,
}

/// 面板里的一个子项。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupKid {
    pub name: String,
    pub path: String,
    pub kind: ItemKind,
    pub running: bool,
}

/// 面板的逻辑尺寸（按子项数）。
#[must_use]
pub fn logical_size(count: usize) -> (f32, f32) {
    let rows = count.div_ceil(COLS).max(1);
    let w = (COLS as f32 * CELL_W + PAD * 2.0).min(CELL_W * count as f32 + PAD * 2.0);
    let h = HEADER_H + PAD + rows as f32 * CELL_H + PAD;
    (w.max(200.0), h)
}

/// 第 `k` 个子项的图标矩形（逻辑坐标，原点是面板左上角）。
///
/// 空面板时返回 `None`。纯函数：命中判定与绘制必须用同一份几何。
#[must_use]
pub fn cell_rect(k: usize, count: usize) -> Option<(f32, f32, f32, f32)> {
    if k >= count {
        return None;
    }
    let cols = COLS.min(count.max(1));
    let col = k % cols;
    let row = k / cols;
    let x = PAD + col as f32 * CELL_W + (CELL_W - ICON_S) / 2.0;
    let y = HEADER_H + PAD + row as f32 * CELL_H;
    Some((x, y, ICON_S, ICON_S))
}

/// 分组面板。
pub struct GroupWindow {
    hwnd: HWND,
    renderer: FrameRenderer,
    input: crate::input::Input,
    pos: (i32, i32),
    ppp: f32,
    dark: bool,
    visible: bool,
    /// 分组名（画在标题栏）。
    title: String,
    kids: Vec<GroupKid>,
    /// 子项图标（按子项下标）。
    icons: HashMap<usize, egui::TextureHandle>,
    /// 已经问过 shell 的路径，不重复问。
    asked: HashSet<String>,
    loader: Option<IconLoader>,
    /// 悬停的子项下标。
    hover: Option<usize>,
    /// 按下的子项下标。
    press: Option<usize>,
    /// 抬起时命中的子项下标（`None` = 抬起时不在任何格子上）。
    released: Option<usize>,
    /// 待执行的动作。
    action: GroupAction,
    /// 下一次要重绘的时刻。
    next_repaint: Option<Instant>,
}

impl std::fmt::Debug for GroupWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroupWindow")
            .field("hwnd", &(self.hwnd.0 as isize))
            .field("visible", &self.visible)
            .field("kids", &self.kids.len())
            .finish_non_exhaustive()
    }
}

impl GroupWindow {
    /// 建窗（默认隐藏）。
    pub fn new(gpu: &Gpu, ppp: f32, dark: bool) -> Result<Pin<Box<Self>>, GpuError> {
        let hinst =
            unsafe { GetModuleHandleW(None) }.map_err(|_| GpuError::Win32("GetModuleHandleW"))?;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: windows::Win32::Foundation::HINSTANCE(hinst.0),
            lpszClassName: class_pcwstr(),
            hCursor: crate::cursor::class_cursor(),
            hIcon: crate::app_icon().unwrap_or_default(),
            ..Default::default()
        };
        unsafe {
            RegisterClassW(&raw const wc);
        }

        let pw = (400.0 * ppp).round() as i32;
        let ph = (300.0 * ppp).round() as i32;
        let (x, y) = centered_origin(pw, ph);
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_APPWINDOW | WS_EX_TOPMOST,
                class_pcwstr(),
                windows::core::w!("NookSill 分组"),
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
        .map_err(|_| GpuError::Win32("CreateWindowExW（分组面板）"))?;

        let renderer = FrameRenderer::new(gpu, pw as u32, ph as u32, ppp)?;
        let mut win = Box::pin(Self {
            hwnd,
            renderer,
            input: crate::input::Input::default(),
            pos: (x, y),
            ppp,
            dark,
            visible: false,
            title: String::new(),
            kids: Vec::new(),
            icons: HashMap::new(),
            asked: HashSet::new(),
            loader: None,
            hover: None,
            press: None,
            released: None,
            action: GroupAction::None,
            next_repaint: None,
        });
        win.attach();
        // 自己的取图标线程：纹理不能跨 egui 上下文复用。
        win.loader = IconLoader::spawn(Waker::new(hwnd), Some(gpu)).ok();
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
    pub const fn hwnd(&self) -> HWND {
        self.hwnd
    }

    #[must_use]
    pub const fn visible(&self) -> bool {
        self.visible
    }

    /// 打开面板。`anchor` 是 Dock 上那个分组的屏幕矩形（物理像素）——
    /// 面板从它上方弹出来。
    pub fn open(
        &mut self,
        gpu: &Gpu,
        title: &str,
        kids: Vec<GroupKid>,
        anchor: (i32, i32, i32, i32),
    ) -> Result<(), GpuError> {
        self.title = title.to_owned();
        self.kids = kids;
        // 换了分组就清掉图标缓存：下标不再指向同一个东西。
        self.icons.clear();
        self.asked.clear();
        self.hover = None;
        self.press = None;
        self.released = None;
        self.action = GroupAction::None;

        let (lw, lh) = logical_size(self.kids.len());
        let pw = (lw * self.ppp).round() as i32;
        let ph = (lh * self.ppp).round() as i32;
        self.renderer.resize(gpu, pw as u32, ph as u32)?;

        // 摆在分组正上方，水平居中；顶到屏幕上沿就往下挪。
        let (al, at, ar, _ab) = anchor;
        let cx = (al + ar) / 2;
        let mut x = cx - pw / 2;
        let work = nooksill_platform::display::work_area(0);
        if x < work.left {
            x = work.left;
        }
        if x + pw > work.right {
            x = work.right - pw;
        }
        let mut y = at - ph - 8;
        if y < work.top {
            y = work.top;
        }
        self.pos = (x, y);
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                x,
                y,
                pw,
                ph,
                SWP_NOACTIVATE | SWP_NOZORDER,
            );
            let _ = ShowWindow(self.hwnd, SW_SHOW);
        }
        self.visible = true;
        self.next_repaint = Some(Instant::now());
        Ok(())
    }

    pub fn hide(&mut self) {
        if !self.visible {
            return;
        }
        self.visible = false;
        self.hover = None;
        self.press = None;
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    /// 取走待执行的动作。
    pub fn take_action(&mut self) -> GroupAction {
        std::mem::replace(&mut self.action, GroupAction::None)
    }

    /// 是否该重绘。
    #[must_use]
    pub fn due(&self) -> bool {
        self.visible && self.next_repaint.is_some_and(|t| t <= Instant::now())
    }

    /// 画一帧。
    pub fn render(&mut self, gpu: &Gpu) -> Result<(), GpuError> {
        self.next_repaint = None;
        self.request_icons();

        let pos = self.pos;
        let hwnd = self.hwnd;
        let dark = self.dark;
        let title = self.title.clone();
        let kids = self.kids.clone();
        let hover = self.hover;
        let released = self.released;
        let raw = self.input.take(self.renderer.size(), self.ppp);

        // 图标纹理要在这个上下文里上传。先把结果取出来（`drain` 不阻塞），
        // 再在绘制闭包里 `load_texture` —— 两步分开是为了不同时借用
        // `self.renderer`（可变）与 `self.loader`（可变）。
        let ready: Vec<crate::worker::ReadyIcon> =
            self.loader.as_ref().map(|l| l.drain()).unwrap_or_default();
        let mut icons = self.icons.clone();
        let mut actions: Vec<GroupAction> = Vec::new();
        let mut dissolve = false;

        self.renderer.render(gpu, raw, hwnd, pos, |ui| {
            for r in &ready {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [r.width as usize, r.height as usize],
                    &r.rgba,
                );
                let tex = ui.ctx().load_texture(
                    format!("group-kid-{}", r.index),
                    image,
                    egui::TextureOptions::LINEAR,
                );
                icons.insert(r.index, tex);
            }
            let (kid_actions, dis) = paint(ui, dark, &title, &kids, &icons, hover, released);
            actions = kid_actions;
            dissolve = dis;
        })?;

        self.icons = icons;
        // `released` 是一次性事件：不清的话，下一次因为悬停/图标到位
        // 重绘时会**再启动一次**同一个应用。
        self.released = None;
        // 面板产出的动作直接交给主循环执行。命中矩形与绘制用同一份几何
        // （`cell_rect`），不会"看到的是 A、点到的是 B"。
        for action in actions {
            self.action = action;
            self.next_repaint = Some(Instant::now());
        }
        if dissolve {
            self.action = GroupAction::Dissolve;
            self.next_repaint = Some(Instant::now());
        }
        Ok(())
    }

    /// 给还没图标的子项排一次取图标任务。
    fn request_icons(&mut self) {
        let Some(loader) = self.loader.as_ref() else {
            return;
        };
        for (i, kid) in self.kids.iter().enumerate() {
            if self.icons.contains_key(&i) || kid.path.is_empty() {
                continue;
            }
            if !self.asked.insert(kid.path.clone()) {
                continue;
            }
            loader.submit(IconRequest {
                index: i,
                kind: kid.kind,
                path: kid.path.clone(),
                icon_src: String::new(),
                fit: 0,
                kids: Vec::new(),
            });
        }
    }

    /// 命中测试：光标在哪个子项上。
    fn hit_at(&self, x: f32, y: f32) -> Option<usize> {
        (0..self.kids.len()).find(|&k| match cell_rect(k, self.kids.len()) {
            Some((cx, cy, cw, ch)) => x >= cx && x <= cx + cw && y >= cy && y <= cy + ch,
            None => false,
        })
    }

    fn on_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            // 取图标线程交回结果了。
            //
            // 图标是异步到的；没有这条，面板会一直停在"首字占位"上，
            // 非得用户晃一下鼠标才重绘。
            WM_APP_WAKE => {
                self.next_repaint = Some(Instant::now());
                Some(LRESULT(0))
            }
            WM_SETCURSOR => {
                crate::cursor::set(crate::cursor::arrow());
                Some(LRESULT(1))
            }
            WM_MOUSEMOVE => {
                let (x, y) = client_pos(lparam, self.ppp);
                self.input.pointer_moved(x, y);
                let h = self.hit_at(x, y);
                if h != self.hover {
                    self.hover = h;
                    self.next_repaint = Some(Instant::now());
                }
                None
            }
            WM_LBUTTONDOWN => {
                let (x, y) = client_pos(lparam, self.ppp);
                self.input.pointer_moved(x, y);
                self.press = self.hit_at(x, y);
                self.input
                    .pointer_button(true, egui::PointerButton::Primary);
                self.next_repaint = Some(Instant::now());
                None
            }
            WM_LBUTTONUP => {
                let (x, y) = client_pos(lparam, self.ppp);
                self.input.pointer_moved(x, y);
                self.input
                    .pointer_button(false, egui::PointerButton::Primary);
                let released = self.hit_at(x, y);
                // 点在 × 上还是图标上，由 `paint` 判定；这里只记
                // "按下了第 k 个"，抬起时同一格才算点中。
                if self.press == released {
                    self.released = released;
                }
                self.press = None;
                self.next_repaint = Some(Instant::now());
                None
            }
            WM_RBUTTONDOWN | WM_RBUTTONUP => None,
            WM_KEYDOWN => {
                // Esc 收起：面板不靠捕获也能关（`PanelDismiss` 关掉时
                // 唯一的出口）。
                if wparam.0 == 0x1B {
                    self.hide();
                }
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
}

/// 离屏验收入口（`examples/panels_shot.rs`）。
///
/// 图标纹理传空表 → 走首字占位分支；离屏没有 shell 图标可取，正好
/// 把这条退化路径也画一遍。
pub fn paint_preview(
    ui: &mut egui::Ui,
    dark: bool,
    title: &str,
    kids: &[GroupKid],
    hover: Option<usize>,
) {
    let empty: HashMap<usize, egui::TextureHandle> = HashMap::new();
    let _ = paint(ui, dark, title, kids, &empty, hover, None);
}

/// 画面板。返回 `(子项动作列表, 要解散分组)`。
fn paint(
    ui: &mut egui::Ui,
    dark: bool,
    title: &str,
    kids: &[GroupKid],
    icons: &HashMap<usize, egui::TextureHandle>,
    hover: Option<usize>,
    released: Option<usize>,
) -> (Vec<GroupAction>, bool) {
    let rect = ui.max_rect();
    let (bg, ink, faint, border) = if dark {
        (
            Color32::from_rgba_unmultiplied(30, 30, 34, 246),
            Color32::from_rgba_unmultiplied(240, 240, 245, 255),
            Color32::from_rgba_unmultiplied(255, 255, 255, 130),
            Color32::from_rgba_unmultiplied(255, 255, 255, 34),
        )
    } else {
        (
            Color32::from_rgba_unmultiplied(250, 250, 252, 248),
            Color32::from_rgba_unmultiplied(28, 28, 32, 255),
            Color32::from_rgba_unmultiplied(0, 0, 0, 120),
            Color32::from_rgba_unmultiplied(0, 0, 0, 26),
        )
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(16), bg);
    painter.rect_stroke(
        rect,
        CornerRadius::same(16),
        Stroke::new(1.0, border),
        egui::StrokeKind::Inside,
    );
    painter.text(
        egui::pos2(rect.left() + PAD, rect.top() + PAD + 2.0),
        Align2::LEFT_TOP,
        title,
        FontId::proportional(15.0),
        ink,
    );

    // 标题栏右侧的「解散分组」。
    let dissolve;
    {
        let bw = 84.0;
        let bh = 26.0;
        let br = EguiRect::from_min_size(
            egui::pos2(rect.right() - PAD - bw, rect.top() + PAD),
            egui::vec2(bw, bh),
        );
        let resp = ui.interact(br, egui::Id::unique("group-dissolve"), egui::Sense::click());
        painter.rect_filled(br, CornerRadius::same(8), faint.linear_multiply(0.18));
        painter.text(
            br.center(),
            Align2::CENTER_CENTER,
            "解散分组",
            FontId::proportional(12.0),
            ink,
        );
        dissolve = resp.clicked();
    }

    // 子项网格。
    let mut actions = Vec::new();
    for (k, kid) in kids.iter().enumerate() {
        let Some((cx, cy, cw, ch)) = cell_rect(k, kids.len()) else {
            continue;
        };
        let cell = EguiRect::from_min_size(egui::pos2(cx, cy), egui::vec2(cw, ch));
        let hovered = hover == Some(k);
        if hovered {
            painter.rect_filled(cell, CornerRadius::same(10), faint.linear_multiply(0.12));
        }
        // 图标。
        if let Some(tex) = icons.get(&k) {
            painter.image(
                tex.id(),
                cell,
                // UV 是归一化的 0..1，不是像素尺寸。`load_texture` 把图
                // 排进图集后给的 id 对应整块区域，写 0..60 会只采到
                // 左上角一个角、把图标放大 60 倍。
                EguiRect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        } else if let Some(c) = kid.name.chars().next() {
            crate::glyph::paint_monogram(
                painter,
                cell,
                c,
                crate::glyph::IconStyle::new(
                    Color32::WHITE,
                    Color32::from_rgb(46, 123, 246),
                    crate::glyph::IconState::default(),
                ),
                dark,
            );
        }
        // 名字。
        painter.text(
            egui::pos2(cell.center().x, cell.bottom() + 16.0),
            Align2::CENTER_CENTER,
            fit_text(painter, &kid.name, cw - 8.0, 11.5),
            FontId::proportional(11.5),
            faint,
        );
        // 运行指示点。
        if kid.running {
            painter.circle_filled(
                egui::pos2(cell.center().x, cell.bottom() + 30.0),
                3.0,
                Color32::from_rgb(80, 200, 120),
            );
        }
        // 悬停时右上角的 ×：把子项移出分组。
        //
        // 必须**完全落在 `cell_rect` 里**：× 只在 `hover == Some(k)` 时
        // 画，而 `hover` 是 `hit_at` 按 `cell_rect` 判的。放到格子外面
        // 的话，光标一移到 × 上 hover 就变 None、× 自己消失，永远点不到。
        if hovered {
            let cr = EguiRect::from_min_size(
                egui::pos2(cell.right() - 18.0, cell.top() + 2.0),
                egui::vec2(16.0, 16.0),
            );
            let resp = ui.interact(cr, egui::Id::unique(("group-x", k)), egui::Sense::click());
            painter.circle_filled(cr.center(), 8.0, faint.linear_multiply(0.55));
            painter.line_segment(
                [
                    egui::pos2(cr.center().x - 3.5, cr.center().y - 3.5),
                    egui::pos2(cr.center().x + 3.5, cr.center().y + 3.5),
                ],
                Stroke::new(1.6, ink),
            );
            painter.line_segment(
                [
                    egui::pos2(cr.center().x + 3.5, cr.center().y - 3.5),
                    egui::pos2(cr.center().x - 3.5, cr.center().y + 3.5),
                ],
                Stroke::new(1.6, ink),
            );
            if resp.clicked() {
                actions.push(GroupAction::Remove(k));
            }

            // 组内排序：左上角的小箭头。和 × 一样**完全落在 `cell_rect` 里**，
            // 否则光标一移过去 hover 就丢了、按钮自己消失。
            if k > 0 {
                let r = EguiRect::from_min_size(
                    egui::pos2(cell.left() + 2.0, cell.top() + 2.0),
                    egui::vec2(16.0, 16.0),
                );
                if arrow_button(ui, painter, r, -1, faint, ink, k) {
                    actions.push(GroupAction::Move(k, -1));
                }
            }
            if k + 1 < kids.len() {
                let r = EguiRect::from_min_size(
                    egui::pos2(cell.left() + 20.0, cell.top() + 2.0),
                    egui::vec2(16.0, 16.0),
                );
                if arrow_button(ui, painter, r, 1, faint, ink, k) {
                    actions.push(GroupAction::Move(k, 1));
                }
            }
        }
    }

    // 点在图标上（没点到小按钮）→ 启动。用"抬起时命中的格"避免重复触发。
    if let Some(k) = released {
        let touched = actions.iter().any(|a| match a {
            GroupAction::Launch(i) | GroupAction::Remove(i) | GroupAction::Move(i, _) => *i == k,
            _ => false,
        });
        if !touched {
            actions.push(GroupAction::Launch(k));
        }
    }

    (actions, dissolve)
}

/// 一个圆底小箭头按钮。返回本帧是否被点。
fn arrow_button(
    ui: &egui::Ui,
    painter: &egui::Painter,
    rect: EguiRect,
    dir: i32,
    bg: Color32,
    ink: Color32,
    kid: usize,
) -> bool {
    let resp = ui.interact(
        rect,
        egui::Id::unique(("group-arrow", kid, dir)),
        egui::Sense::click(),
    );
    painter.circle_filled(rect.center(), 8.0, bg.linear_multiply(0.55));
    let c = rect.center();
    // 负号不能少：`dir = -1` 要画「<」，`dir = +1` 要画「>」。
    // 少了它两个箭头会反过来，功能没错但一眼就能看出是错的。
    let d = -3.0 * dir as f32;
    painter.line_segment(
        [egui::pos2(c.x + d, c.y - 3.5), egui::pos2(c.x - d, c.y)],
        Stroke::new(1.6, ink),
    );
    painter.line_segment(
        [egui::pos2(c.x - d, c.y), egui::pos2(c.x + d, c.y + 3.5)],
        Stroke::new(1.6, ink),
    );
    resp.clicked()
}

/// 超宽就截断加省略号。
fn fit_text(painter: &egui::Painter, text: &str, max_w: f32, size: f32) -> String {
    let font = FontId::proportional(size);
    if painter
        .layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE)
        .size()
        .x
        <= max_w
    {
        return text.to_owned();
    }
    let mut out: Vec<char> = text.chars().collect();
    while !out.is_empty() {
        out.pop();
        let s: String = out.iter().collect();
        if painter
            .layout_no_wrap(format!("{s}…"), font.clone(), Color32::WHITE)
            .size()
            .x
            <= max_w
        {
            return format!("{s}…");
        }
    }
    String::new()
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut GroupWindow;
    if ptr.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    // SAFETY: 指针由 `attach` 挂在 `Pin<Box<Self>>` 的最终地址上。
    let win = unsafe { &mut *ptr };
    if let Some(result) = win.on_message(msg, wparam, lparam) {
        return result;
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

impl Drop for GroupWindow {
    fn drop(&mut self) {
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
        }
    }
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

fn centered_origin(pw: i32, ph: i32) -> (i32, i32) {
    let work = nooksill_platform::display::work_area(0);
    let cx = (work.left + work.right) / 2;
    let cy = (work.top + work.bottom) / 2;
    (cx - pw / 2, cy - ph / 2)
}

fn client_pos(lparam: LPARAM, ppp: f32) -> (f32, f32) {
    let x = (lparam.0 & 0xFFFF) as i16 as f32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
    (x / ppp, y / ppp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_grows_with_the_kid_count() {
        let one = logical_size(1);
        let many = logical_size(9);
        assert!(many.1 > one.1, "九个子项应当比一个高");
        assert!(many.0 >= one.0);
    }

    #[test]
    fn cells_lay_out_in_a_grid() {
        let n = 5;
        let (x0, y0, _, _) = cell_rect(0, n).expect("第一个格子");
        let (x1, y1, _, _) = cell_rect(1, n).expect("第二个格子");
        let (x4, y4, _, _) = cell_rect(4, n).expect("第五个格子");
        assert!(x1 > x0, "同一行的下一个格子在右边");
        assert!((y1 - y0).abs() < 0.01, "同一行 y 相同");
        assert!(y4 > y1, "换行后 y 变大");
        assert!((x4 - x0).abs() < 0.01, "换行后回到第一列");
    }

    #[test]
    fn out_of_range_cells_are_none() {
        assert!(cell_rect(3, 3).is_none());
        assert!(cell_rect(0, 0).is_none());
    }

    #[test]
    fn one_kid_gets_a_single_column() {
        let (x, _, w, _) = cell_rect(0, 1).expect("唯一格子");
        assert!(x >= 0.0);
        assert!(w > 0.0);
    }
}
