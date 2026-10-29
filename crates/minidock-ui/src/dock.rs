//! Dock 主窗口：分层渲染、悬停放大、自动隐藏、托盘与热键。
//!
//! 这是**第一阶段骨架**：几何、配色、隐藏动画、悬停放大、时钟卡片已经按
//! 原版公式实现，图标仍用"圆角底盘 + 首字"占位（图标加载在
//! `PORTING.md` 的 P2 阶段）。这样做的目的是尽早把
//! 「配置 → 度量 → GDI+ → 分层窗口」整条链路跑通，而不是先堆功能。

use minidock_core::color::{argb, hsv_to_rgb, rgb};
use minidock_core::config::{BgMode, Config, Edge};
use minidock_core::item::Item;
use minidock_core::layout::{self, Metrics, Rect, SlotKind};
use minidock_gfx::{Brush, Canvas, Font, FontFamily, GfxError, Path, Pen, RectF, StringFormat};
use minidock_platform::{display, theme, Paths};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::UpdateWindow;
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, TrackMouseEvent, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL,
    TME_LEAVE, TRACKMOUSEEVENT, VK_D,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, FindWindowW, GetCursorPos, GetMessageW,
    GetWindowLongPtrW, KillTimer, LoadIconW, PostMessageW, PostQuitMessage, RegisterClassW,
    SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage, GWLP_USERDATA,
    HWND_TOPMOST, IDI_APPLICATION, MA_NOACTIVATE, MSG, SWP_NOACTIVATE, SW_SHOWNA, WM_APP,
    WM_CAPTURECHANGED, WM_DESTROY, WM_DISPLAYCHANGE, WM_HOTKEY, WM_LBUTTONDBLCLK, WM_LBUTTONUP,
    WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_RBUTTONUP, WM_SETTINGCHANGE, WM_TIMER, WNDCLASSW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use minidock_core::APP_NAME;

/// 窗口类名。与 C 版一致 —— 单实例的"唤起已有窗口"依赖它。
///
/// `w!()` 只接受字面量，而类名要在三处使用，所以缓存成 NUL 结尾的
/// UTF-16 并复用；改类名只需要改这一行。
const CLS: &str = "MiniDockWnd";

fn cls_pcwstr() -> PCWSTR {
    static BUF: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    let buf = BUF.get_or_init(|| minidock_platform::win::to_wide(CLS));
    PCWSTR(buf.as_ptr())
}

const TIMER_ANIM: usize = 1;
const TIMER_HIDE: usize = 3;
const WM_APP_TRAY: u32 = WM_APP + 2;

/// 隐藏动画的收敛系数（原版 `0.28`）。数值越大越快。
const HIDE_LERP: f32 = 0.28;
/// 拖拽滑动的收敛系数（原版 `0.62`）。
const SLIDE_LERP: f32 = 0.62;

#[derive(Debug)]
pub enum DockError {
    Gfx(GfxError),
    Window(&'static str),
    Platform(String),
}

impl std::fmt::Display for DockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Gfx(e) => write!(f, "{e}"),
            Self::Window(what) => write!(f, "窗口创建失败: {what}"),
            Self::Platform(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for DockError {}

impl From<GfxError> for DockError {
    fn from(e: GfxError) -> Self {
        Self::Gfx(e)
    }
}

/// 一帧的几何缓存。对应原版的一组全局变量
/// （`barL` / `barW` / `barTop` / `barBottom` / `slotL[]` / `actC[]`）。
#[derive(Debug, Default)]
struct FrameGeom {
    bar_l: f32,
    bar_w: f32,
    bar_top: f32,
    /// 栏体底边（运行指示点从这条线往上排，P2 使用）。
    #[allow(dead_code)]
    bar_bottom: f32,
    icon_bottom: f32,
    slot_l: Vec<f32>,
    act_c: Vec<f32>,
}

pub struct Dock {
    hwnd: HWND,
    paths: Paths,
    cfg: Config,
    items: Vec<Item>,
    dpi: i32,
    work: Rect,
    metrics: Metrics,

    canvas: Option<Canvas>,
    geom: FrameGeom,

    /// 保留字体族：P2 需要按配置动态重建字号。
    #[allow(dead_code)]
    font_family: FontFamily,
    font_small: Font,
    font_bold: Font,
    font_tile: Font,
    fmt_center: StringFormat,
    fmt_leading: StringFormat,

    mouse_u: f32,
    hover: bool,
    hot: Option<usize>,
    hide_cur: f32,
    hide_target: f32,
    anim_on: bool,
    dark: bool,
}

impl std::fmt::Debug for Dock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dock")
            .field("items", &self.items.len())
            .field("edge", &self.cfg.edge)
            .field("size", &(self.metrics.lw, self.metrics.lh))
            .finish_non_exhaustive()
    }
}

impl Dock {
    /// 构造窗口之前的所有状态。此时还没有 `HWND`。
    pub fn new(paths: Paths, cfg: Config, items: Vec<Item>, dpi: i32) -> Result<Self, DockError> {
        let work = display::work_area(cfg.monitor);
        let n_sep = items.iter().filter(|i| i.kind.is_separator()).count();
        let metrics = layout::calc_metrics(&cfg, dpi, items.len(), n_sep, work);
        let dark = cfg.dock_is_dark(theme::is_dark());

        let font_family = FontFamily::new("Microsoft YaHei UI")?;
        let font_small = Font::new(&font_family, scale(13, dpi), false)?;
        let font_bold = Font::new(&font_family, scale(15, dpi), true)?;
        let font_tile = Font::new(&font_family, scale(26, dpi), true)?;
        let fmt_center = StringFormat::centered()?;
        let fmt_leading = StringFormat::leading()?;

        Ok(Self {
            hwnd: HWND::default(),
            paths,
            cfg,
            items,
            dpi,
            work,
            metrics,
            canvas: None,
            geom: FrameGeom::default(),
            font_family,
            font_small,
            font_bold,
            font_tile,
            fmt_center,
            fmt_leading,
            mouse_u: -1.0,
            hover: false,
            hot: None,
            hide_cur: 0.0,
            hide_target: 0.0,
            anim_on: false,
            dark,
        })
    }

    /// 创建窗口并进入消息循环。函数返回时窗口已销毁。
    pub fn run(mut self) -> Result<(), DockError> {
        self.register_class()?;
        let (w, h) = self.window_size();
        let (x, y) = self.window_pos(w, h);

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                cls_pcwstr(),
                w!("MiniDock"),
                WS_POPUP,
                x,
                y,
                w,
                h,
                None,
                None,
                None,
                None,
            )
        }
        .map_err(|_| DockError::Window("CreateWindowExW"))?;

        self.hwnd = hwnd;
        self.sync_surface()?;
        self.add_tray();
        self.register_hotkey();

        // 现在才把状态交给窗口过程：在此之前任何消息都走 DefWindowProcW。
        let ptr = Box::into_raw(Box::new(self));
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr as isize) };

        let dock = unsafe { &mut *ptr };
        dock.render();
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNA);
            let _ = UpdateWindow(hwnd);
        }
        dock.kick();
        if dock.cfg.mode != minidock_core::config::DockMode::Always {
            unsafe { SetTimer(Some(hwnd), TIMER_HIDE, 400, None) };
        }

        let mut msg = MSG::default();
        while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        drop(unsafe { Box::from_raw(ptr) });
        Ok(())
    }

    fn window_size(&self) -> (i32, i32) {
        if self.cfg.edge.is_horizontal() {
            (self.metrics.lw, self.metrics.lh)
        } else {
            (self.metrics.lh, self.metrics.lw)
        }
    }

    /// 窗口贴哪。与原版 `UpdateGeometry()` 的四种情形一一对应
    /// （`Reserve` 的应用栏预留尚未移植，见 PORTING.md）。
    fn window_pos(&self, w: i32, h: i32) -> (i32, i32) {
        let wa = self.work;
        match self.cfg.edge {
            Edge::Top => (wa.left + (wa.width() - w) / 2, wa.top),
            Edge::Left => (wa.left, wa.top + (wa.height() - h) / 2),
            Edge::Right => (wa.right - w, wa.top + (wa.height() - h) / 2),
            Edge::Bottom => (wa.left + (wa.width() - w) / 2, wa.bottom - h),
        }
    }

    fn register_class(&self) -> Result<(), DockError> {
        let hinstance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }
            .map_err(|_| DockError::Window("GetModuleHandleW"))?;
        let wc = WNDCLASSW {
            hInstance: windows::Win32::Foundation::HINSTANCE(hinstance.0),
            hCursor: unsafe {
                windows::Win32::UI::WindowsAndMessaging::LoadCursorW(
                    None,
                    windows::Win32::UI::WindowsAndMessaging::IDC_ARROW,
                )
            }
            .unwrap_or_default(),
            lpfnWndProc: Some(wndproc),
            lpszClassName: cls_pcwstr(),
            hbrBackground: HBRUSH::default(),
            ..Default::default()
        };
        let atom = unsafe { RegisterClassW(&wc) };
        if atom == 0 {
            // 已注册（重复运行同一进程）不算错误。
            return Ok(());
        }
        Ok(())
    }

    /// 让画布尺寸与当前度量一致。尺寸变化时整块重建。
    fn sync_surface(&mut self) -> Result<(), DockError> {
        let (w, h) = self.window_size();
        let need = match &self.canvas {
            Some(c) => c.width() != w || c.height() != h,
            None => true,
        };
        if need {
            self.canvas = Some(Canvas::new(w, h)?);
        }
        Ok(())
    }

    fn kick(&mut self) {
        if !self.anim_on {
            self.anim_on = true;
            unsafe { SetTimer(Some(self.hwnd), TIMER_ANIM, 15, None) };
        }
    }

    /// 计算一帧的槽位与插值状态。返回"是否还需要下一帧"。
    ///
    /// 几何全部来自 [`minidock_core::layout`]，与 GPU 版共用同一份实现。
    fn step_layout(&mut self) -> bool {
        let m = self.metrics;
        let mut anim = false;

        // 隐藏动画：指数逼近，差值 < 0.004 视为到位。
        let ht = self.hide_target;
        if (ht - self.hide_cur).abs() > 0.004 {
            self.hide_cur += (ht - self.hide_cur) * HIDE_LERP;
            anim = true;
        } else {
            self.hide_cur = ht;
        }

        let kinds: Vec<SlotKind> = self
            .items
            .iter()
            .map(|it| {
                if it.kind.is_separator() {
                    SlotKind::Separator
                } else {
                    SlotKind::Icon
                }
            })
            .collect();

        // 基准中心 → 放大目标 → 指数逼近。
        let bars = layout::bars(&m);
        let base_c = layout::base_centers(&m, &kinds, m.clock_extra);
        let view = layout::View {
            // 遗留版只跟踪横向位置，纵向直接取图标底边 —— 一定落在栏体内。
            mouse: (self.mouse_u, bars.icon_bottom),
            hover: self.hover,
            hide_cur: self.hide_cur,
        };
        let targets =
            layout::magnify_targets(&m, &kinds, &base_c, &self.geom.act_c, view, self.cfg.zoom);
        for (it, target) in self.items.iter_mut().zip(&targets) {
            anim |= layout::lerp_to(&mut it.cur, *target);
            // 拖拽滑动偏移（P2 才真正赋值，这里只负责衰减）。
            if it.slide.abs() > 0.5 {
                it.slide *= SLIDE_LERP;
                anim = true;
            } else {
                it.slide = 0.0;
            }
        }

        // 实际槽位与命中（用本帧槽位，否则标签会比图标慢一帧）。
        let cur: Vec<f32> = self.items.iter().map(|i| i.cur).collect();
        let slots = layout::place_slots(&m, &cur, m.clock_extra);
        self.hot = layout::hit_test(&m, &bars, &slots, &cur, view);

        self.geom = FrameGeom {
            bar_l: slots.bar_l,
            bar_w: slots.bar_w,
            bar_top: bars.top,
            bar_bottom: bars.bottom,
            icon_bottom: bars.icon_bottom,
            slot_l: slots.slot_l,
            act_c: slots.act_c,
        };

        anim
    }

    /// 渲染一帧。返回是否仍有动画在进行。
    fn render(&mut self) -> bool {
        let anim = self.step_layout();
        if let Err(e) = self.draw_frame() {
            // 绘制失败不应让 Dock 死掉：记录到调试输出，维持窗口存在。
            eprintln!("MiniDock: 渲染失败: {e}");
        }
        anim
    }

    fn draw_frame(&self) -> Result<(), GfxError> {
        let Some(canvas) = self.canvas.as_ref() else {
            return Ok(());
        };
        let g = canvas.graphics()?;
        g.clear(0)?;

        let m = self.metrics;
        if self.hide_cur < 0.995 {
            let off = self.hide_cur * (m.bar_h + m.margin + 14.0);
            let (tx, ty) = match self.cfg.edge {
                Edge::Top => (0.0, -off),
                Edge::Left => (-off, 0.0),
                Edge::Right => (off, 0.0),
                Edge::Bottom => (0.0, off),
            };
            if off > 0.0 {
                g.translate(tx, ty)?;
            }

            let lh = m.lh as f32;
            let (bx, by, bw, bh) = layout::map_rect(
                self.cfg.edge,
                self.geom.bar_l,
                self.geom.bar_top,
                self.geom.bar_w,
                m.bar_h,
                lh,
            );
            let radius = (self.cfg.radius as f32).min(m.bar_h / 2.0);
            self.paint_bar(&g, bx, by, bw, bh, radius)?;

            if self.items.is_empty() {
                let text_color = if self.dark {
                    argb(190, 255, 255, 255)
                } else {
                    argb(190, 40, 40, 40)
                };
                let brush = Brush::solid(text_color)?;
                g.draw_text(
                    "将应用 / 文件夹 / 网址拖到这里",
                    &self.font_small,
                    RectF {
                        X: bx,
                        Y: by,
                        Width: bw,
                        Height: bh,
                    },
                    &self.fmt_center,
                    &brush,
                )?;
            }

            self.paint_clock(&g, bx, by, bw, bh, radius)?;

            // 图标（第一阶段：底盘 + 首字占位）。
            for (i, it) in self.items.iter().enumerate() {
                if it.kind.is_separator() || it.cur < 1.0 {
                    continue;
                }
                let sz = it.cur;
                let (ix, iy, iw, ih) = layout::map_rect(
                    self.cfg.edge,
                    self.geom.slot_l[i] + it.slide,
                    self.geom.icon_bottom - sz,
                    sz,
                    sz,
                    lh,
                );
                let pr = (sz * self.cfg.plate_radius as f32 / 100.0).min(sz / 2.0);
                let plate = if self.dark {
                    argb(38, 255, 255, 255)
                } else {
                    argb(28, 0, 0, 0)
                };
                let path = Path::round_rect(ix, iy, iw, ih, pr)?;
                g.fill_path(&Brush::solid(plate)?, &path)?;

                if let Some(ch) = it.name.chars().next() {
                    // 首字用色相区分条目，便于在没有图标时辨认。
                    let hue = (hash_name(&it.name) % 360) as f32;
                    let (r, gg, b) = hsv_to_rgb(hue, 0.45, if self.dark { 0.95 } else { 0.75 });
                    let brush = Brush::solid(argb(235, r, gg, b))?;
                    let text = ch.to_string();
                    g.draw_text(
                        &text,
                        &self.font_tile,
                        RectF {
                            X: ix,
                            Y: iy,
                            Width: iw,
                            Height: ih,
                        },
                        &self.fmt_center,
                        &brush,
                    )?;
                }
            }

            // 悬停标签。
            if let Some(hot) = self.hot {
                if self.cfg.labels && hot < self.items.len() && self.items[hot].cur > 1.0 {
                    self.paint_bubble(&g, hot, lh)?;
                }
            }

            g.reset_transform()?;
        }

        drop(g);
        canvas.present(
            self.hwnd,
            self.window_pos(canvas.width(), canvas.height()).0,
            self.window_pos(canvas.width(), canvas.height()).1,
        )?;
        Ok(())
    }

    /// 栏体背景：纯色 / 渐变 / 磨砂（磨砂尚未移植，退化为渐变）。
    fn paint_bar(
        &self,
        g: &minidock_gfx::Graphics<'_>,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
    ) -> Result<(), GfxError> {
        if self.cfg.opacity <= 0 {
            return Ok(());
        }
        let op = self.cfg.opacity as f32 / 100.0;
        let alpha = (op * 255.0).round().clamp(0.0, 255.0) as u8;

        let (c1, c2) = self.bar_colors();
        let path = Path::round_rect(x, y, w, h, radius)?;
        let brush = if self.cfg.bg_mode == BgMode::Solid {
            Brush::solid(minidock_core::color::with_alpha(c1, alpha))?
        } else {
            // 方向与原版一致：横向 Dock 用「左上→右下」斜向渐变，
            // 竖向 Dock 用垂直渐变。改成纯水平会丢掉斜向高光。
            let mode = if self.cfg.edge.is_horizontal() {
                windows::Win32::Graphics::GdiPlus::LinearGradientModeForwardDiagonal
            } else {
                windows::Win32::Graphics::GdiPlus::LinearGradientModeVertical
            };
            Brush::linear(
                RectF {
                    X: x,
                    Y: y,
                    Width: w,
                    Height: h,
                },
                minidock_core::color::with_alpha(c1, alpha),
                minidock_core::color::with_alpha(c2, alpha),
                mode,
            )?
        };
        g.fill_path(&brush, &path)?;

        if self.cfg.border {
            let ba = if self.dark {
                (90.0 * (op / 0.35).min(1.0)) as u8
            } else {
                (150.0 * (op / 0.35).min(1.0)) as u8
            };
            let pen = Pen::new(argb(ba, 255, 255, 255), 1.2)?;
            let stroke =
                Path::round_rect(x + 0.6, y + 0.6, w - 1.2, h - 1.2, (radius - 0.6).max(0.0))?;
            g.draw_path(&pen, &stroke)?;
        }
        Ok(())
    }

    /// 栏体两端颜色。顺序：强调色 → 自定义颜色 → 主题默认。
    fn bar_colors(&self) -> (u32, u32) {
        if self.cfg.accent_tone {
            // TODO(P2)：接 DwmGetColorizationColor 取真实强调色相。
            let hue = 210.0;
            if self.dark {
                let (r1, g1, b1) = hsv_to_rgb(hue, 0.42, 0.34);
                let (r2, g2, b2) = hsv_to_rgb(hue + 18.0, 0.5, 0.22);
                (rgb(r1, g1, b1), rgb(r2, g2, b2))
            } else {
                let (r1, g1, b1) = hsv_to_rgb(hue, 0.16, 0.97);
                let (r2, g2, b2) = hsv_to_rgb(hue + 18.0, 0.24, 0.90);
                (rgb(r1, g1, b1), rgb(r2, g2, b2))
            }
        } else if self.cfg.bg_color1 >= 0 {
            let c1 = minidock_core::color::from_colorref(self.cfg.bg_color1 as u32);
            let c2 = if self.cfg.bg_color2 >= 0 {
                minidock_core::color::from_colorref(self.cfg.bg_color2 as u32)
            } else {
                minidock_core::color::mix(c1, rgb(0, 0, 0), 0.25)
            };
            (c1, c2)
        } else if self.dark {
            (rgb(58, 60, 68), rgb(30, 31, 36))
        } else {
            (rgb(255, 255, 255), rgb(232, 235, 243))
        }
    }

    fn paint_clock(
        &self,
        g: &minidock_gfx::Graphics<'_>,
        bx: f32,
        by: f32,
        _bw: f32,
        _bh: f32,
        radius: f32,
    ) -> Result<(), GfxError> {
        if !self.cfg.clock {
            return Ok(());
        }
        // 竖向停靠的时钟卡片是另一种排布（原版额外预留 170 px），P2 处理。
        if !self.cfg.edge.is_horizontal() {
            return Ok(());
        }
        let m = self.metrics;
        let (cx, cy, cw, ch) = layout::map_rect(
            self.cfg.edge,
            self.geom.bar_l + self.geom.bar_w + m.gap + 6.0,
            self.geom.bar_top,
            m.clock_w as f32,
            m.bar_h,
            m.lh as f32,
        );
        let _ = (bx, by);
        self.paint_bar(g, cx, cy, cw, ch, radius)?;

        let st = unsafe { GetLocalTime() };
        let time = format!("{:02}:{:02}", st.wHour, st.wMinute);
        let weeks = ["日", "一", "二", "三", "四", "五", "六"];
        let wd = weeks[(st.wDayOfWeek as usize).min(6)];
        let date = format!("{}月{}日 周{}", st.wMonth, st.wDay, wd);

        let tc = if self.dark {
            argb(245, 240, 240, 245)
        } else {
            argb(245, 30, 30, 36)
        };
        let brush = Brush::solid(tc)?;
        g.draw_text(
            &time,
            &self.font_bold,
            RectF {
                X: cx,
                Y: cy + ch * 0.12,
                Width: cw,
                Height: ch * 0.44,
            },
            &self.fmt_center,
            &brush,
        )?;
        let sub = if self.dark {
            argb(190, 220, 220, 228)
        } else {
            argb(190, 60, 60, 70)
        };
        g.draw_text(
            &date,
            &self.font_small,
            RectF {
                X: cx,
                Y: cy + ch * 0.54,
                Width: cw,
                Height: ch * 0.32,
            },
            &self.fmt_center,
            &Brush::solid(sub)?,
        )?;
        Ok(())
    }

    fn paint_bubble(
        &self,
        g: &minidock_gfx::Graphics<'_>,
        index: usize,
        lh: f32,
    ) -> Result<(), GfxError> {
        let it = &self.items[index];
        let sz = it.cur;
        let (ix, iy, iw, _ih) = layout::map_rect(
            self.cfg.edge,
            self.geom.slot_l[index] + it.slide,
            self.geom.icon_bottom - sz,
            sz,
            sz,
            lh,
        );

        let measured = g.measure_text(&it.name, &self.font_small, &self.fmt_leading)?;
        let pad = 10.0;
        let bubble_w = (measured.Width + pad * 2.0).clamp(48.0, 320.0);
        let bubble_h = 26.0;
        let bx = ix + iw / 2.0 - bubble_w / 2.0;
        let by = iy - bubble_h - 8.0;

        let fill = if self.dark {
            argb(230, 32, 33, 38)
        } else {
            argb(235, 255, 255, 255)
        };
        let path = Path::round_rect(bx, by, bubble_w, bubble_h, 8.0)?;
        g.fill_path(&Brush::solid(fill)?, &path)?;
        let border = if self.dark {
            argb(60, 255, 255, 255)
        } else {
            argb(50, 0, 0, 0)
        };
        g.draw_path(&Pen::new(border, 1.0)?, &path)?;

        let text_color = if self.dark {
            argb(240, 240, 240, 245)
        } else {
            argb(240, 30, 30, 36)
        };
        g.draw_text(
            &it.name,
            &self.font_small,
            RectF {
                X: bx + pad,
                Y: by,
                Width: bubble_w - pad * 2.0,
                Height: bubble_h,
            },
            &self.fmt_leading,
            &Brush::solid(text_color)?,
        )?;
        Ok(())
    }

    fn add_tray(&self) {
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: WM_APP_TRAY,
            hIcon: unsafe { LoadIconW(None, IDI_APPLICATION) }.unwrap_or_default(),
            ..Default::default()
        };
        let tip_text = format!("{APP_NAME} — {}", self.paths.dir.display());
        let tip: Vec<u16> = tip_text.encode_utf16().collect();
        let n = tip.len().min(nid.szTip.len());
        nid.szTip[..n].copy_from_slice(&tip[..n]);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        }
    }

    fn remove_tray(&self) {
        let nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            ..Default::default()
        };
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
        }
    }

    fn register_hotkey(&self) {
        if !self.cfg.hotkey {
            return;
        }
        unsafe {
            let _ = RegisterHotKey(
                Some(self.hwnd),
                1,
                HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_ALT.0),
                VK_D.0 as u32,
            );
        }
    }

    fn unregister_hotkey(&self) {
        if self.cfg.hotkey {
            unsafe {
                let _ = UnregisterHotKey(Some(self.hwnd), 1);
            }
        }
    }

    fn toggle_hide(&mut self) {
        self.hide_target = if self.hide_target > 0.5 { 0.0 } else { 1.0 };
        self.hover = false;
        self.mouse_u = -1.0;
        self.kick();
    }

    /// 自动隐藏：光标离开栏体就收起。
    fn on_hide_timer(&mut self) {
        if self.cfg.mode == minidock_core::config::DockMode::Always {
            return;
        }
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt) }.is_err() {
            return;
        }
        // 窗口矩形做一点外扩：贴着边缘时不算"离开"。
        let (w, h) = self.window_size();
        let (x, y) = self.window_pos(w, h);
        let pad = 8;
        let inside =
            pt.x >= x - pad && pt.x <= x + w + pad && pt.y >= y - pad && pt.y <= y + h + pad;
        let target = if inside { 0.0 } else { 1.0 };
        if (target - self.hide_target).abs() > 0.01 {
            self.hide_target = target;
            self.kick();
        }
    }

    fn on_mouse_move(&mut self, x: i32, y: i32) {
        let (u, _v) = layout::to_logical(self.cfg.edge, x as f32, y as f32, self.metrics.lh as f32);
        self.mouse_u = u;
        if !self.hover {
            self.hover = true;
            unsafe {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: self.hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tme);
            }
            unsafe { SetTimer(Some(self.hwnd), 6, 40, None) };
        }
        self.kick();
    }

    fn on_display_change(&mut self) {
        self.work = display::work_area(self.cfg.monitor);
        let n_sep = self.items.iter().filter(|i| i.kind.is_separator()).count();
        self.metrics =
            layout::calc_metrics(&self.cfg, self.dpi, self.items.len(), n_sep, self.work);
        let (w, h) = self.window_size();
        let (x, y) = self.window_pos(w, h);
        unsafe {
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE);
        }
        let _ = self.sync_surface();
        self.kick();
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Dock;
    if ptr.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    // SAFETY: 指针由 `Dock::run` 用 `Box::into_raw` 产生，并在消息循环结束后
    // 才回收；窗口过程只在 UI 线程被调用，因此这里不存在别名 `&mut`。
    let dock = unsafe { &mut *ptr };

    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_MOUSEMOVE => {
            dock.on_mouse_move(lparam.0 as i16 as i32, (lparam.0 >> 16) as i16 as i32);
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            dock.hover = false;
            dock.mouse_u = -1.0;
            dock.hot = None;
            dock.kick();
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            // 点击图标启动应用属于 P2（需要 shell 层），这里先收起预览态。
            dock.kick();
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            // 右键菜单在 P3 移植；目前给出视觉反馈，避免"点了没反应"。
            dock.toggle_hide();
            LRESULT(0)
        }
        WM_TIMER => {
            match wparam.0 {
                TIMER_ANIM => {
                    if !dock.render() {
                        unsafe {
                            let _ = KillTimer(Some(hwnd), TIMER_ANIM);
                        }
                        dock.anim_on = false;
                    }
                }
                TIMER_HIDE => dock.on_hide_timer(),
                _ => {}
            }
            LRESULT(0)
        }
        WM_HOTKEY => {
            dock.toggle_hide();
            LRESULT(0)
        }
        WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
            dock.on_display_change();
            LRESULT(0)
        }
        WM_APP_TRAY => {
            let ev = (lparam.0 & 0xFFFF) as u32;
            if ev == WM_LBUTTONDBLCLK {
                // 双击托盘图标 = 切换显隐，两种情况同一个动作。
                dock.toggle_hide();
            } else if ev == WM_RBUTTONUP {
                dock.toggle_hide();
            }
            LRESULT(0)
        }
        WM_CAPTURECHANGED => LRESULT(0),
        WM_DESTROY => {
            dock.remove_tray();
            dock.unregister_hotkey();
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn scale(v: i32, dpi: i32) -> f32 {
    minidock_core::units::scale(v, dpi) as f32
}

fn hash_name(s: &str) -> u32 {
    let mut h: u32 = 2166136261;
    for b in s.as_bytes() {
        h ^= u32::from(*b);
        h = h.wrapping_mul(16777619);
    }
    h
}

/// 让 minidock 可执行文件可以把「已有实例」唤到前台。
#[must_use]
pub fn find_existing() -> Option<HWND> {
    unsafe { FindWindowW(cls_pcwstr(), None) }
        .ok()
        .filter(|h| !h.0.is_null())
}

/// 给已有实例投递托盘消息（双击图标的行为）。失败说明对方已退出。
pub fn ping_existing(hwnd: HWND) {
    unsafe {
        let _ = PostMessageW(
            Some(hwnd),
            WM_APP_TRAY,
            WPARAM(0),
            LPARAM(WM_LBUTTONDBLCLK as isize),
        );
    }
}
