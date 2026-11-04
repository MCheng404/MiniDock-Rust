//! 控制中心：点条带上的控制中心卡片弹出来。
//!
//! 360 宽的面板，3 列磁贴，底部一条
//! 主音量滑块与电量文案。
//!
//! 与资源监视器面板的两点差别：
//!
//! - **音量是真的**。走 Core Audio（`platform::audio`），不是模拟
//!   `VK_VOLUME_*` 按键 —— 后者会改"应用程序音量"分组、会在系统 OSD 上闪。
//! - **滑块要能拖**。按下在滑块上就开始跟手，抬起才结束；拖动期间自己
//!   拿鼠标捕获，光标滑出面板也还跟得住。
//!
//! 磁贴里的深色模式 / 节能是"有状态"的：点完要把自己的状态改掉并重画，
//! 深色还要通知主循环重读主题（`dirty_theme`）。

use std::pin::Pin;
use std::time::Instant;

use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect as EguiRect, Stroke};
use minidock_core::control::{self, Tile};
use minidock_platform::audio::{Audio, Battery};
use minidock_platform::sysact;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetWindowLongPtrW, RegisterClassW, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, GWLP_USERDATA, SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOWNA,
    WM_ERASEBKGND, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SETCURSOR, WNDCLASSW,
    WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::frame::FrameRenderer;
use crate::gpu::{Gpu, GpuError};

/// 窗口类名。
pub const CLASS_NAME: &str = "MiniDockCtrlCenter";

const PANEL_W: f32 = 360.0;
const PM: f32 = 8.0;
const TILE_H: f32 = 74.0;
const TILE_GAP: f32 = 10.0;
const SLIDER_H: f32 = 28.0;

/// 面板高度：标题 + 磁贴 + 滑块 + 电量。
fn panel_h(tiles: usize) -> f32 {
    PM * 2.0 + 40.0 + control::rows(tiles) as f32 * (TILE_H + TILE_GAP) + 30.0 + 30.0
}

/// 磁贴区域左上角与宽度。
fn tile_origin() -> (f32, f32) {
    (PM + 12.0, PM + 40.0)
}

fn tile_w() -> f32 {
    (PANEL_W - 2.0 * PM - 24.0 - (control::COLS as f32 - 1.0) * TILE_GAP) / control::COLS as f32
}

/// 一次点击的落点。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Hit {
    Tile(usize),
    Slider,
}

/// 控制中心面板。
pub struct CcWindow {
    hwnd: HWND,
    renderer: FrameRenderer,
    input: crate::input::Input,
    pos: (i32, i32),
    ppp: f32,
    visible: bool,
    dark: bool,
    /// 主题被这个面板改过，主循环要重读一次。
    pub dirty_theme: bool,

    /// 磁贴列表（顺序由 core 决定）。
    tiles: Vec<Tile>,
    /// 音频接口。没有可用设备时为 `None`，滑块画成禁用态。
    audio: Option<Audio>,
    /// 本地音量缓存（拖动时先改这个，再写系统）。
    volume: u32,
    mute: bool,
    /// 节能模式的开/关（只用来画底色；系统状态改完不回头查）。
    energy: bool,

    hover: Option<Hit>,
    hits: Vec<EguiRect>,
    slider: EguiRect,
    dragging: bool,
    next_repaint: Instant,
}

impl std::fmt::Debug for CcWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CcWindow")
            .field("hwnd", &(self.hwnd.0 as isize))
            .field("visible", &self.visible)
            .finish_non_exhaustive()
    }
}

impl CcWindow {
    /// 建窗（默认隐藏）。
    pub fn new(
        gpu: &Gpu,
        ppp: f32,
        dark: bool,
        energy_on: bool,
        hotspot_on: bool,
    ) -> Result<Pin<Box<Self>>, GpuError> {
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

        let tiles = control::tiles(energy_on, hotspot_on, false);
        let (pw, ph) = physical_size(ppp, tiles.len());
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                class_pcwstr(),
                windows::core::w!("控制中心"),
                WS_POPUP,
                0,
                0,
                pw,
                ph,
                None,
                None,
                Some(windows::Win32::Foundation::HINSTANCE(hinst.0)),
                None,
            )
        }
        .map_err(|_| GpuError::Win32("CreateWindowExW（控制中心）"))?;

        let renderer = FrameRenderer::new(gpu, pw as u32, ph as u32, ppp)?;
        let audio = Audio::new();
        let volume = audio.as_ref().and_then(Audio::volume).unwrap_or(50);
        let mute = audio.as_ref().map(Audio::muted).unwrap_or(false);

        let mut win = Box::pin(Self {
            hwnd,
            renderer,
            input: crate::input::Input::default(),
            pos: (0, 0),
            ppp,
            visible: false,
            dark,
            dirty_theme: false,
            tiles,
            audio,
            volume,
            mute,
            // **不能**拿 `CcEnergy` 当"节能模式开着"：那个键的语义是
            // "要不要显示这一格"。系统当前是不是节能模式，`powercfg`
            // 没有便宜的查询方式，所以初始按"关"显示，点一下才切。
            energy: false,
            hover: None,
            hits: Vec::new(),
            slider: EguiRect::NOTHING,
            dragging: false,
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

    /// 窗口句柄。主循环判"点是否落在面板里"要用。
    #[must_use]
    pub const fn hwnd(&self) -> HWND {
        self.hwnd
    }

    #[must_use]
    pub const fn visible(&self) -> bool {
        self.visible
    }

    /// 打开面板，摆在控制中心卡片旁边。
    pub fn show_at(&mut self, anchor: (f32, f32, f32, f32), edge: minidock_core::config::Edge) {
        // 每次打开都重读一次系统音量：用户可能在别处调过。
        if let Some(a) = &self.audio {
            self.volume = a.volume().unwrap_or(self.volume);
            self.mute = a.muted();
        }
        let (pw, ph) = physical_size(self.ppp, self.tiles.len());
        let (x, y) = place(pw, ph, anchor, edge, self.ppp);
        self.pos = (x, y);
        unsafe {
            let _ = SetWindowPos(self.hwnd, None, x, y, pw, ph, SWP_NOZORDER | SWP_NOACTIVATE);
            let _ = ShowWindow(self.hwnd, SW_SHOWNA);
        }
        self.visible = true;
        self.next_repaint = Instant::now();
    }

    /// 关掉。会释放鼠标捕获。
    pub fn hide(&mut self) {
        if !self.visible {
            return;
        }
        self.visible = false;
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    #[must_use]
    pub fn due(&self) -> bool {
        self.visible && self.next_repaint <= Instant::now()
    }

    /// 画一帧。
    pub fn render(&mut self, gpu: &Gpu) -> Result<(), GpuError> {
        let pos = self.pos;
        let hwnd = self.hwnd;
        let dark = self.dark;
        let tiles = self.tiles.clone();
        let volume = self.volume;
        let mute = self.mute;
        let energy = self.energy;
        let has_audio = self.audio.is_some();
        let hover = self.hover;
        let battery = Battery::read();
        let raw = self.input.take(self.renderer.size(), self.ppp);
        let mut hits = Vec::new();
        let mut slider = EguiRect::NOTHING;
        self.renderer.render(gpu, raw, hwnd, pos, |ui| {
            let (h, s) = draw_panel(
                ui, dark, &tiles, volume, mute, energy, has_audio, battery, hover,
            );
            hits = h;
            slider = s;
        })?;
        self.hits = hits;
        self.slider = slider;
        Ok(())
    }

    /// 设音量（拖动时每帧调一次）。
    fn set_volume(&mut self, v: u32) {
        let v = v.min(100);
        if v == self.volume {
            return;
        }
        self.volume = v;
        if let Some(a) = &self.audio {
            a.set_volume(v);
        }
        self.next_repaint = Instant::now();
    }

    /// 点一个磁贴。
    fn click_tile(&mut self, idx: usize) {
        let Some(tile) = self.tiles.get(idx).copied() else {
            return;
        };
        match tile {
            // 这三个没有稳定的公开接口，拉系统自己的面板。
            Tile::Wifi | Tile::Bluetooth | Tile::Hotspot => {
                sysact::open_quick_settings();
                self.hide();
            }
            Tile::Ethernet => {
                sysact::open_uri("ms-settings:network-status");
                self.hide();
            }
            Tile::DarkMode => {
                let next = !self.dark;
                self.dark = sysact::set_dark_mode(next);
                self.dirty_theme = true;
                self.next_repaint = Instant::now();
            }
            Tile::Location => {
                sysact::open_uri("ms-settings:privacy-location");
                self.hide();
            }
            Tile::Settings => {
                sysact::open_uri("ms-settings:");
                self.hide();
            }
            Tile::Mute => {
                self.mute = match &self.audio {
                    Some(a) => a.toggle_mute(),
                    None => !self.mute,
                };
                self.next_repaint = Instant::now();
            }
            Tile::Energy => {
                self.energy = !self.energy;
                sysact::set_power_saver(self.energy);
                self.next_repaint = Instant::now();
            }
        }
    }

    fn on_message(&mut self, msg: u32, _wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_MOUSEMOVE => {
                let (x, y) = client_pos(lparam, self.ppp);
                let p = egui::pos2(x, y);
                if self.dragging {
                    // 滑块跟手：按比例算，两端各留 7 像素给圆钮。
                    let left = self.slider.left() + 30.0;
                    let right = self.slider.right() - 4.0;
                    if right > left {
                        let f = ((x - left) / (right - left)).clamp(0.0, 1.0);
                        self.set_volume((f * 100.0).round() as u32);
                    }
                    return None;
                }
                let h = if self.slider.contains(p) {
                    Some(Hit::Slider)
                } else {
                    self.hits.iter().position(|r| r.contains(p)).map(Hit::Tile)
                };
                if self.hover != h {
                    self.hover = h;
                    self.next_repaint = Instant::now();
                }
                // 面板一露头就 `SetCapture`，捕获期间收不到 `WM_SETCURSOR`，
                // 光标只能在这里设（ADR-045）。
                crate::cursor::set_hand_if(self.hover.is_some());
                None
            }
            WM_LBUTTONDOWN => {
                let (x, y) = client_pos(lparam, self.ppp);
                let p = egui::pos2(x, y);
                if self.slider.contains(p) {
                    self.dragging = true;
                    // 拖动要自己拿捕获，否则光标一滑出面板就收不到
                    // `WM_MOUSEMOVE`，滑块会卡在半路。
                    unsafe {
                        SetCapture(self.hwnd);
                    }
                    let left = self.slider.left() + 30.0;
                    let right = self.slider.right() - 4.0;
                    if right > left {
                        let f = ((x - left) / (right - left)).clamp(0.0, 1.0);
                        self.set_volume((f * 100.0).round() as u32);
                    }
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                if self.dragging {
                    self.dragging = false;
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    return Some(LRESULT(0));
                }
                let (x, y) = client_pos(lparam, self.ppp);
                if let Some(Hit::Tile(k)) = self
                    .hits
                    .iter()
                    .position(|r| r.contains(egui::pos2(x, y)))
                    .map(Hit::Tile)
                {
                    self.click_tile(k);
                }
                Some(LRESULT(0))
            }
            WM_SETCURSOR => {
                crate::cursor::set_hand_if(self.hover.is_some());
                Some(LRESULT(1))
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

/// 画整块。返回（磁贴命中区，滑块命中区）。
#[allow(clippy::too_many_arguments)]
fn draw_panel(
    ui: &mut egui::Ui,
    dark: bool,
    tiles: &[Tile],
    volume: u32,
    mute: bool,
    energy: bool,
    has_audio: bool,
    battery: Battery,
    hover: Option<Hit>,
) -> (Vec<EguiRect>, EguiRect) {
    let rect = ui.max_rect();
    let painter = ui.painter().clone();

    let bg = if dark {
        Color32::from_rgba_unmultiplied(24, 25, 30, 250)
    } else {
        Color32::from_rgba_unmultiplied(246, 247, 250, 250)
    };
    painter.rect_filled(rect, CornerRadius::same(16), bg);

    let (tc, sub) = if dark {
        (
            Color32::from_rgba_unmultiplied(242, 242, 246, 255),
            Color32::from_rgba_unmultiplied(200, 202, 214, 255),
        )
    } else {
        (
            Color32::from_rgba_unmultiplied(28, 28, 34, 255),
            Color32::from_rgba_unmultiplied(90, 94, 108, 255),
        )
    };
    let accent = ui.visuals().selection.bg_fill;
    let tile_off = if dark {
        Color32::from_rgba_unmultiplied(255, 255, 255, 58)
    } else {
        Color32::from_rgba_unmultiplied(0, 0, 0, 22)
    };
    let tile_off_hover = if dark {
        Color32::from_rgba_unmultiplied(255, 255, 255, 88)
    } else {
        Color32::from_rgba_unmultiplied(0, 0, 0, 38)
    };

    // 标题
    painter.text(
        egui::pos2(rect.left() + PANEL_W * 0.5, rect.top() + PM + 14.0),
        Align2::CENTER_CENTER,
        "控制中心",
        FontId::proportional(15.0),
        tc,
    );

    let (tx0, ty0) = tile_origin();
    let tw = tile_w();
    let mut hits = Vec::with_capacity(tiles.len());

    for (k, tile) in tiles.iter().enumerate() {
        let col = k % control::COLS;
        let row = k / control::COLS;
        let x = rect.left() + tx0 + col as f32 * (tw + TILE_GAP);
        let y = rect.top() + ty0 + row as f32 * (TILE_H + TILE_GAP);
        let r = EguiRect::from_min_size(egui::pos2(x, y), egui::vec2(tw, TILE_H));
        hits.push(r);

        let on = match tile {
            Tile::DarkMode => dark,
            Tile::Mute => mute,
            Tile::Energy => energy,
            _ => false,
        };
        let hovered = hover == Some(Hit::Tile(k));
        let fill = if on {
            accent
        } else if hovered {
            tile_off_hover
        } else {
            tile_off
        };
        painter.rect_filled(r, CornerRadius::same(14), fill);

        let fg = if on { Color32::WHITE } else { tc };
        glyph(
            &painter,
            *tile,
            egui::pos2(x + tw * 0.5, y + 24.0),
            24.0,
            fg,
        );
        painter.text(
            egui::pos2(x + tw * 0.5, y + 52.0),
            Align2::CENTER_CENTER,
            tile.label(),
            FontId::proportional(11.5),
            if on {
                Color32::from_rgba_unmultiplied(255, 255, 255, 240)
            } else {
                sub
            },
        );
    }

    // 音量滑块。
    let rows = control::rows(tiles.len());
    let sy = rect.top() + ty0 + rows as f32 * (TILE_H + TILE_GAP) + 6.0;
    let slider = EguiRect::from_min_size(
        egui::pos2(rect.left() + tx0, sy),
        egui::vec2(
            tw * control::COLS as f32 + TILE_GAP * (control::COLS as f32 - 1.0),
            SLIDER_H,
        ),
    );
    let icon_c = egui::pos2(slider.left() + 11.0, slider.center().y);
    speaker(&painter, icon_c, 20.0, sub, mute);
    let track = EguiRect::from_min_max(
        egui::pos2(slider.left() + 30.0, slider.center().y - 3.0),
        egui::pos2(slider.right() - 4.0, slider.center().y + 3.0),
    );
    let track_bg = if dark {
        Color32::from_rgba_unmultiplied(255, 255, 255, 70)
    } else {
        Color32::from_rgba_unmultiplied(0, 0, 0, 40)
    };
    painter.rect_filled(track, CornerRadius::same(3), track_bg);

    let frac = if has_audio {
        volume as f32 / 100.0
    } else {
        0.0
    };
    let filled_w = track.width() * frac;
    if filled_w > 1.0 {
        painter.rect_filled(
            EguiRect::from_min_size(track.min, egui::vec2(filled_w, track.height())),
            CornerRadius::same(3),
            if has_audio { accent } else { sub },
        );
    }
    // 圆钮：贴住两端时不要露到轨道外面。
    let knob_x = (track.left() + filled_w).clamp(track.left() + 7.0, track.right() - 7.0);
    painter.circle_filled(
        egui::pos2(knob_x, slider.center().y),
        7.0,
        Color32::from_rgba_unmultiplied(255, 255, 255, 255),
    );

    // 电量
    painter.text(
        egui::pos2(slider.left(), slider.bottom() + 8.0),
        Align2::LEFT_TOP,
        battery.text(),
        FontId::proportional(11.0),
        sub,
    );

    (hits, slider)
}

/// 画一个喇叭。`muted` 时右边是个叉，否则是两道声波弧。
///
/// 音量滑块左边的图标与「静音」磁贴共用这一个。
fn speaker(painter: &egui::Painter, c: Pos2, s: f32, color: Color32, muted: bool) {
    let stroke = Stroke::new((s * 0.09).max(1.2), color);
    // 喇叭本体：一个方块接一个三角（用多边形拼，避免描边缝隙）。
    painter.rect_filled(
        EguiRect::from_min_size(
            egui::pos2(c.x - s * 0.42, c.y - s * 0.10),
            egui::vec2(s * 0.16, s * 0.20),
        ),
        CornerRadius::same(1),
        color,
    );
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(c.x - s * 0.26, c.y - s * 0.10),
            egui::pos2(c.x - s * 0.06, c.y - s * 0.28),
            egui::pos2(c.x - s * 0.06, c.y + s * 0.28),
            egui::pos2(c.x - s * 0.26, c.y + s * 0.10),
        ],
        color,
        Stroke::NONE,
    ));
    if muted {
        painter.line_segment(
            [
                egui::pos2(c.x + s * 0.04, c.y - s * 0.13),
                egui::pos2(c.x + s * 0.26, c.y + s * 0.13),
            ],
            stroke,
        );
        painter.line_segment(
            [
                egui::pos2(c.x + s * 0.26, c.y - s * 0.13),
                egui::pos2(c.x + s * 0.04, c.y + s * 0.13),
            ],
            stroke,
        );
    } else {
        for i in 0..2 {
            let r = s * (0.16 + i as f32 * 0.12);
            let mut pts = Vec::new();
            for k in 0..=12 {
                let t = -0.9 + 1.8 * k as f32 / 12.0;
                pts.push(egui::pos2(c.x + s * 0.02 + r * t.cos(), c.y + r * t.sin()));
            }
            painter.add(egui::Shape::line(pts, stroke));
        }
    }
}

/// 画一个磁贴的图标。全部用线段/圆弧拼，不用字符 —— 打包的字体里没有
/// 这些符号（ADR-040 在 ✓/✕ 上踩过豆腐块）。
fn glyph(painter: &egui::Painter, tile: Tile, c: Pos2, s: f32, color: Color32) {
    let stroke = Stroke::new((s * 0.09).max(1.2), color);
    let arc = |cx: f32, cy: f32, r: f32, from: f32, to: f32, w: f32| {
        let mut pts = Vec::new();
        let steps = 18;
        for i in 0..=steps {
            let t = from + (to - from) * i as f32 / steps as f32;
            pts.push(egui::pos2(cx + r * t.cos(), cy + r * t.sin()));
        }
        painter.add(egui::Shape::line(pts, Stroke::new(w, color)));
    };
    match tile {
        Tile::Wifi => {
            for i in 0..3 {
                let r = s * (0.46 - i as f32 * 0.13);
                arc(c.x, c.y + s * 0.30, r, -2.50, -0.64, stroke.width);
            }
            painter.circle_filled(egui::pos2(c.x, c.y + s * 0.30), s * 0.06, color);
        }
        Tile::Ethernet => {
            painter.rect_filled(
                EguiRect::from_min_size(
                    egui::pos2(c.x - s * 0.26, c.y - s * 0.20),
                    egui::vec2(s * 0.52, s * 0.34),
                ),
                CornerRadius::same(2),
                color,
            );
            for dx in [-0.12_f32, 0.0, 0.12] {
                painter.line_segment(
                    [
                        egui::pos2(c.x + s * dx, c.y + s * 0.14),
                        egui::pos2(c.x + s * dx, c.y + s * 0.30),
                    ],
                    stroke,
                );
            }
        }
        Tile::Bluetooth => {
            let p = |x: f32, y: f32| egui::pos2(c.x + s * x, c.y + s * y);
            painter.line_segment([p(-0.22, -0.20), p(0.22, 0.20)], stroke);
            painter.line_segment([p(-0.22, 0.20), p(0.22, -0.20)], stroke);
            painter.line_segment([p(0.0, -0.42), p(0.22, -0.20)], stroke);
            painter.line_segment([p(0.0, 0.42), p(0.22, 0.20)], stroke);
            painter.line_segment([p(0.0, -0.42), p(0.0, 0.42)], stroke);
        }
        Tile::DarkMode => {
            // 月牙：一段大圆弧 + 一段反向小圆弧。
            arc(c.x, c.y, s * 0.40, 0.8, 5.5, stroke.width);
            arc(
                c.x + s * 0.16,
                c.y - s * 0.10,
                s * 0.30,
                1.4,
                4.9,
                stroke.width,
            );
        }
        Tile::Location => {
            arc(
                c.x,
                c.y - s * 0.10,
                s * 0.28,
                0.0,
                std::f32::consts::TAU,
                stroke.width,
            );
            painter.circle_filled(egui::pos2(c.x, c.y - s * 0.10), s * 0.06, color);
            painter.line_segment(
                [
                    egui::pos2(c.x - s * 0.14, c.y + s * 0.12),
                    egui::pos2(c.x, c.y + s * 0.40),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(c.x + s * 0.14, c.y + s * 0.12),
                    egui::pos2(c.x, c.y + s * 0.40),
                ],
                stroke,
            );
        }
        Tile::Mute => speaker(painter, c, s, color, true),
        Tile::Settings => {
            arc(c.x, c.y, s * 0.26, 0.0, std::f32::consts::TAU, stroke.width);
            for i in 0..4 {
                let a = i as f32 * std::f32::consts::FRAC_PI_2;
                painter.line_segment(
                    [
                        egui::pos2(c.x + a.cos() * s * 0.30, c.y + a.sin() * s * 0.30),
                        egui::pos2(c.x + a.cos() * s * 0.42, c.y + a.sin() * s * 0.42),
                    ],
                    stroke,
                );
            }
        }
        Tile::Energy => {
            arc(c.x, c.y, s * 0.34, 2.1, 5.6, stroke.width);
            painter.line_segment(
                [
                    egui::pos2(c.x - s * 0.28, c.y + s * 0.28),
                    egui::pos2(c.x + s * 0.26, c.y - s * 0.26),
                ],
                stroke,
            );
        }
        Tile::Hotspot => {
            painter.circle_filled(egui::pos2(c.x, c.y + s * 0.24), s * 0.07, color);
            arc(c.x, c.y + s * 0.34, s * 0.24, 3.6, 5.8, stroke.width);
            arc(c.x, c.y + s * 0.34, s * 0.38, 3.6, 5.8, stroke.width);
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut CcWindow;
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

impl Drop for CcWindow {
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

fn physical_size(ppp: f32, tiles: usize) -> (i32, i32) {
    (
        (PANEL_W * ppp).round() as i32,
        (panel_h(tiles) * ppp).round() as i32,
    )
}

/// 摆在卡片旁边，规则同日历弹层，再夹进显示器。
fn place(
    pw: i32,
    ph: i32,
    anchor: (f32, f32, f32, f32),
    edge: minidock_core::config::Edge,
    ppp: f32,
) -> (i32, i32) {
    use minidock_core::config::Edge;
    let (al, at, ar, ab) = anchor;
    let ax = (al + ar) * 0.5;
    let ay = (at + ab) * 0.5;
    let pm = (PM * ppp).round() as i32;
    let gap = (4.0 * ppp).round() as i32;
    let (x, y) = match edge {
        Edge::Top => (ax - pw as f32 * 0.5, ab - pm as f32 + gap as f32),
        Edge::Left => (ar - pm as f32 + gap as f32, ay - ph as f32 * 0.5),
        Edge::Right => (
            al - pw as f32 + pm as f32 - gap as f32,
            ay - ph as f32 * 0.5,
        ),
        Edge::Bottom => (
            ax - pw as f32 * 0.5,
            at - ph as f32 + pm as f32 - gap as f32,
        ),
    };
    let mon = minidock_platform::display::monitor_rect(0);
    let mut xi = x.round() as i32;
    let mut yi = y.round() as i32;
    if xi + pw > mon.right {
        xi = mon.right - pw;
    }
    if yi + ph > mon.bottom {
        yi = mon.bottom - ph;
    }
    if xi < mon.left {
        xi = mon.left;
    }
    if yi < mon.top {
        yi = mon.top;
    }
    (xi, yi)
}

/// 客户区坐标（物理像素）→ 逻辑点。
fn client_pos(lparam: LPARAM, ppp: f32) -> (f32, f32) {
    let x = (lparam.0 & 0xffff) as u16 as i16 as f32;
    let y = ((lparam.0 >> 16) & 0xffff) as u16 as i16 as f32;
    (x / ppp, y / ppp)
}

/// 供调用方判断这个窗口是不是控制中心面板。
#[must_use]
pub fn is_cc_window(hwnd: HWND) -> bool {
    let mut buf = [0_u16; 64];
    let n = unsafe { windows::Win32::UI::WindowsAndMessaging::GetClassNameW(hwnd, &mut buf) };
    n > 0 && String::from_utf16_lossy(&buf[..n as usize]) == CLASS_NAME
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 面板高度随磁贴行数变化，且不会小于内容需要的高度。
    #[test]
    fn panel_height_follows_rows() {
        let two = panel_h(6);
        let three = panel_h(8);
        assert!(three > two);
        assert!((three - two - (TILE_H + TILE_GAP)).abs() < 0.01);
    }

    /// 三列磁贴的宽度：整行加上两个间距要正好铺满内容宽度。
    #[test]
    fn three_tiles_fill_the_row() {
        let tw = tile_w();
        let total = tw * 3.0 + TILE_GAP * 2.0;
        let expect = PANEL_W - 2.0 * PM - 24.0;
        assert!((total - expect).abs() < 0.01, "{total} vs {expect}");
    }
}
