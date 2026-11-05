//! 毛玻璃背景层：抓窗口背后的桌面 → 可分离高斯模糊 → 圆角 alpha → egui 纹理。
//!
//! 数据来源是 [`glass_backdrop`]：
//!
//! - `capture::DesktopCapture` —— GDI `BitBlt` / `StretchBlt` 抓桌面；
//! - `blur::normalized_gaussian_weights` —— 与 WGSL 侧同一份权重的 CPU 模型；
//! - `glass::glass_sdf` —— 圆角矩形的带符号距离，用来给背景层裁圆角。
//!
//! ## 为什么不用它的 SPIR-V 管线
//!
//! `glass-backdrop` 给的是「抓背景 → GPU 模糊链 → `fs_glass` 全屏 pass →
//! 预乘 alpha 交换链」这条 Vulkan 路径。MiniDock 没有交换链：渲染是
//! `egui → 离屏 sRGB 纹理 → CPU 回读 → 预乘 BGRA DIB →
//! UpdateLayeredWindow`（见 [`crate::frame`]）。所以这里把模糊放在 CPU
//! 上做一次，把结果当一张普通 egui 纹理画在 UI 最底层。
//!
//! ## 两次抓取：开窗前的定妆 + 开着时的实时
//!
//! 面板显示之前先同步抓一次（[`Spec::apply`]），第一帧就有背景，不会先闪
//! 一下实心底色。之后由 [`Live`] 线程按节拍持续重抓，桌面在动（播视频、
//! 拖窗口）时模糊跟着动 —— 只用开窗前那一次的话，用户看到的是"糊了一张
//! 静态截图"。
//!
//! 线程里做三件事：GDI 抓屏（~20 ms）→ 和上一帧逐字节比较（不同才继续）
//! → 模糊 + 圆角（~20 ms）。**没变化就什么都不做**，并且把抓取间隔翻倍
//! 退避到 [`LIVE_IDLE_MAX`]：静态桌面上的开销从 20% 掉到 2% 量级。
//!
//! ## 为什么要把自己排除出抓屏
//!
//! 面板一上屏，GDI 抓到的就是"桌面 + 面板自己"，直接拿来当背景会得到
//! 无限嵌套的镜像。上游的做法是 `WDA_EXCLUDEFROMCAPTURE`，这里沿用：
//! 窗口被排除后，抓屏读到的是它**背后**的真实内容（实测分层窗口同样生效）。
//!
//! 代价是这块面板在屏幕捕获里彻底消失 —— 用户截屏 / 录屏时看不到它。
//! 这是实时模糊绕不开的取舍：不排除自己就只能抓到自己。所以只在面板
//! **可见且开着毛玻璃**时设置，隐藏时立刻恢复（见 [`allow_capture`]）。
//!
//! ## 预乘 alpha
//!
//! 背景层在面板内部是**不透明**的（alpha=255）：模糊后的桌面要完全替换掉
//! 真实桌面，而不是叠一层半透明再透出一次真桌面（那样会看到双重影像）。
//! 圆角外侧才渐隐到 0。面板自己的 tint 是半透明的，叠在背景层之上。

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use egui::{Color32, ColorImage};
use glass_backdrop::blur::{clamp_radius, normalized_gaussian_weights, sigma_for_radius};
use glass_backdrop::capture::DesktopCapture;
use glass_backdrop::glass::glass_sdf;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
};

use crate::frame::FrameRenderer;
use crate::worker::Waker;

/// 捕获分辨率占客户区的百分比。
///
/// Acrylic 只让用户看到卷积结果，25% 与 100% 肉眼几乎无差，像素数却少 16 倍：
/// 全屏 2560×1600 的起动台只需模糊 640×400 = 256k 像素。
pub const CAPTURE_PCT: u8 = 25;

/// 实时抓取的起始间隔（约 10 Hz）。
///
/// 再快没有收益：抓屏本身就要 ~20 ms，而背景是糊的，肉眼分辨不出 15 Hz
/// 与 30 Hz 的差别，CPU 却要翻倍。
const LIVE_INTERVAL: Duration = Duration::from_millis(100);

/// 连续抓不到变化时退避到的上限。
///
/// 桌面静止时没必要每 100 ms 花 20 ms 抓一遍；退到 800 ms 后开销降到 2%
/// 量级，一旦发现变化立刻回到 [`LIVE_INTERVAL`]。
const LIVE_IDLE_MAX: Duration = Duration::from_millis(800);

/// 一次开窗的毛玻璃参数。`None` = 这个功能没开。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spec {
    /// 高斯半径（捕获图 texel）。来自配置键 `Blur`。
    pub radius: u32,
    /// 面板圆角（逻辑点）。起动台是方角，传 0。
    pub corner_pt: f32,
}

impl Spec {
    /// 开窗前调用：抓一次背景 → 模糊 → 交给渲染器。
    ///
    /// 捕获失败（最小化、GDI 出错）时撤掉背景层，面板会按
    /// `renderer.frost_on() == false` 自动退回近实心底色 —— 留一张空白纹理
    /// 会让整个面板变透明，比没有毛玻璃更糟。
    pub fn apply(self, renderer: &mut FrameRenderer, hwnd: HWND, ppp: f32) {
        if let Some(image) = capture_once(hwnd.0 as isize, ppp, self.corner_pt, self.radius) {
            renderer.set_frost(image);
        } else {
            renderer.clear_frost();
        }
    }
}

/// 把窗口排除在屏幕捕获之外。
///
/// 必须在 `ShowWindow` **之前**调用：之后才设的话，中间那一帧抓到的是
/// "桌面 + 自己"，模糊层里会出现一层自己的镜像。
pub fn exclude_from_capture(hwnd: HWND) {
    // 失败只可能是系统太老（Win10 2004 之前没有这个值）。那时实时背景会
    // 抓到自己的镜像 —— 由调用方决定是否还要开，这里不额外处理。
    unsafe {
        let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
    }
}

/// 恢复窗口参与屏幕捕获（面板隐藏时调用）。
pub fn allow_capture(hwnd: HWND) {
    unsafe {
        let _ = SetWindowDisplayAffinity(hwnd, WDA_NONE);
    }
}

/// 面板可见期间的实时背景抓取线程。
///
/// 拿到的帧交给 [`FrameRenderer::set_frost`]；线程自己在每次产出后
/// `PostMessage(WM_APP_WAKE)` 把 UI 线程叫起来重绘。
#[derive(Debug)]
pub struct Live {
    /// 只用来在线程退出时断开 `recv_timeout`；`None` = 已请求停止。
    tx: Option<Sender<()>>,
    rx: Receiver<ColorImage>,
    /// 从不 join：抓屏可能正卡在 GDI 里，退出时等它只会拖慢关闭。
    _thread: JoinHandle<()>,
}

impl Live {
    /// 起一条抓取线程。线程创建失败时返回 `None`（面板退回静态背景）。
    #[must_use]
    pub fn start(hwnd: HWND, spec: Spec, ppp: f32) -> Option<Self> {
        let (tx, cmd_rx) = mpsc::channel::<()>();
        let (img_tx, rx) = mpsc::channel::<ColorImage>();
        let waker = Waker::new(hwnd);
        // `HWND` 里是裸指针，不能跨线程；句柄本体的数值可以。
        let raw = hwnd.0 as isize;
        let thread = std::thread::Builder::new()
            .name("minidock-frost".to_owned())
            .spawn(move || run(raw, spec, ppp, &img_tx, waker, &cmd_rx))
            .ok()?;
        Some(Self {
            tx: Some(tx),
            rx,
            _thread: thread,
        })
    }

    /// 取走最新一帧。
    ///
    /// UI 线程慢的时候通道里可能积压好几帧，只保留最后一帧：背景晚一帧
    /// 无所谓，追着补旧帧反而会让画面越拖越旧。
    pub(crate) fn latest(&self) -> Option<ColorImage> {
        let mut newest = None;
        while let Ok(image) = self.rx.try_recv() {
            newest = Some(image);
        }
        newest
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        // 丢掉发送端 → 线程的 `recv_timeout` 立刻返回 `Disconnected` → 退出。
        self.tx = None;
    }
}

/// 抓取线程主体。
fn run(
    hwnd: isize,
    spec: Spec,
    ppp: f32,
    out: &Sender<ColorImage>,
    waker: Waker,
    stop: &Receiver<()>,
) {
    let Ok(mut cap) = DesktopCapture::new(true) else {
        return;
    };
    let mut prev: Vec<u8> = Vec::new();
    let mut dims = (0u32, 0u32);
    let mut interval = LIVE_INTERVAL;
    let mut seq = 0u64;

    loop {
        let started = Instant::now();
        // 最小化（`Ok(None)`）或抓取失败：这一轮跳过，按原节拍重试。
        if let Ok(Some((bits, w, h))) = cap.capture_behind_pct(hwnd, CAPTURE_PCT) {
            let same = dims == (w, h) && prev == bits;
            if same {
                // 桌面没动：翻倍退避，别拿抓屏功耗换一个不会变的背景。
                interval = (interval * 2).min(LIVE_IDLE_MAX);
            } else {
                let diff = if trace() { count_diff(&prev, bits) } else { 0 };
                dims = (w, h);
                prev.clear();
                prev.extend_from_slice(bits);
                interval = LIVE_INTERVAL;
                if let Some(image) = compose(bits, w, h, spec, ppp) {
                    if trace() {
                        report(seq, bits, w, h, started.elapsed(), diff);
                        seq += 1;
                    }
                    if out.send(image).is_err() {
                        return;
                    }
                    waker.wake();
                }
            }
        }

        // 抓 + 模糊的耗时要扣掉，否则实际节拍是"间隔 + 耗时"。
        match stop.recv_timeout(interval.saturating_sub(started.elapsed())) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

/// `MINIDOCK_FROST_TRACE=1`：每产出一帧打一行尺寸 / 平均色 / 耗时。
///
/// 验证"模糊是实时的"就靠它：背后放一个不断变色的窗口，这里的平均色应当
/// 跟着变；一直不变说明抓到的还是开窗前那张静态图（或者抓到了自己 ——
/// 那时平均色会被面板自己的浅色底拉平）。
fn trace() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("MINIDOCK_FROST_TRACE").is_ok_and(|v| v.trim() != "0"))
}

/// 两帧之间变了多少个像素（诊断用，只在 [`trace`] 开着时算）。
fn count_diff(a: &[u8], b: &[u8]) -> usize {
    a.as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .filter(|(p, q)| p != q)
        .count()
}

/// 打一帧的摘要（只在 [`trace`] 开着时调用）。
fn report(seq: u64, bits: &[u8], w: u32, h: u32, spent: Duration, diff: usize) {
    let (mut r, mut g, mut b) = (0u64, 0u64, 0u64);
    let px = bits.as_chunks::<4>().0;
    for p in px {
        r += u64::from(p[0]);
        g += u64::from(p[1]);
        b += u64::from(p[2]);
    }
    let n = px.len().max(1) as u64;
    eprintln!(
        "[frost] #{seq} {w}x{h} 平均色=({},{},{}) 变化像素={diff}/{} 抓+模糊={} ms",
        r / n,
        g / n,
        b / n,
        px.len(),
        spent.as_millis()
    );
}

/// 面板底色。
///
/// 毛玻璃开着时降到约 55% 不透明：下面的模糊桌面要透上来，但文字仍需足够
/// 对比度。不开时维持原来的近实心底色。
#[must_use]
pub fn panel_bg(dark: bool, frost: bool) -> Color32 {
    let (r, g, b) = if dark { (24, 25, 30) } else { (246, 247, 250) };
    let a = if frost { 140 } else { 250 };
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// 圆角矩形的像素覆盖率（0..=255）。
///
/// `corner` 是**图像像素**为单位的圆角半径，不是逻辑点。SDF 在形状内部恒为
/// 负，所以覆盖用 `0.5 - sdf` 做一像素羽化 —— 不能用 `sdf < 0` 的二值判断，
/// 否则圆角会是锯齿。
#[must_use]
pub fn rounded_coverage(x: f32, y: f32, w: f32, h: f32, corner: f32) -> u8 {
    // `glass_sdf` 在形状内部恒为 `-radius`，半径 0 时会退化成处处 `sdf=0`
    // → 覆盖率恒为 50%（上游 README 里明确点名的 "square panel" 坑）。
    // 方角面板不需要 SDF，直接全不透明。
    if corner <= 0.0 {
        return 255;
    }
    let half = [w * 0.5, h * 0.5];
    let p = [x + 0.5 - half[0], y + 0.5 - half[1]];
    let sdf = glass_sdf(p, half, corner);
    let cov = (0.5 - sdf).clamp(0.0, 1.0);
    (cov * 255.0).round() as u8
}

/// 可分离高斯模糊（RGBA，边界夹取）。
///
/// 两趟一维卷积：先横后竖，复杂度从 `O(r²)` 降到 `O(2r)`。权重直接取
/// `glass_backdrop::blur::normalized_gaussian_weights`，与上游 WGSL 侧的
/// 卷积核是同一份定义 —— 改一边忘了另一边，常量测试会先红。
///
/// ## 为什么先拆平面
///
/// 交错 RGBA 里同一通道相邻像素隔 4 个字节，卷积内层要按步长取数，编译器
/// 无法向量化；拆成三个连续平面后内层就是"切片 × 权重"的点积。第二趟
/// 按转置写出（跨行写，一次卷积只写一个值，不心疼），于是两趟都不需要
/// 额外的转置内存。实测 640×400 / r=12 从 40 ms 降到 20 ms。
#[must_use]
pub fn blur_rgba(src: &[u8], w: usize, h: usize, radius: u32) -> Vec<u8> {
    let need = w.saturating_mul(h).saturating_mul(4);
    if w == 0 || h == 0 || src.len() < need {
        return Vec::new();
    }
    let r = clamp_radius(radius) as usize;
    let weights = normalized_gaussian_weights(radius, sigma_for_radius(radius));
    let n = w * h;

    let mut plane = [vec![0.0f32; n], vec![0.0f32; n], vec![0.0f32; n]];
    for i in 0..n {
        plane[0][i] = f32::from(src[i * 4]);
        plane[1][i] = f32::from(src[i * 4 + 1]);
        plane[2][i] = f32::from(src[i * 4 + 2]);
    }

    // 横向：按转置写 → `mid[x * h + y]` 就是"原图第 x 列"的第 y 个值。
    let mut mid = [vec![0.0f32; n], vec![0.0f32; n], vec![0.0f32; n]];
    for c in 0..3 {
        for y in 0..h {
            conv_row(
                &plane[c][y * w..(y + 1) * w],
                &mut mid[c],
                y,
                h,
                &weights,
                r,
            );
        }
    }

    // 纵向：把"列"当行走一遍，同样按转置写 → 恢复成正常的 w×h 布局。
    let mut out = [vec![0.0f32; n], vec![0.0f32; n], vec![0.0f32; n]];
    for c in 0..3 {
        for x in 0..w {
            conv_row(&mid[c][x * h..(x + 1) * h], &mut out[c], x, w, &weights, r);
        }
    }

    let mut rgba = vec![0u8; need];
    for i in 0..n {
        rgba[i * 4] = out[0][i].round().clamp(0.0, 255.0) as u8;
        rgba[i * 4 + 1] = out[1][i].round().clamp(0.0, 255.0) as u8;
        rgba[i * 4 + 2] = out[2][i].round().clamp(0.0, 255.0) as u8;
        rgba[i * 4 + 3] = 255;
    }
    rgba
}

/// 一行一维卷积：`out[base + x * stride] = Σ row[x - r + k] * weights[k]`。
///
/// 左右各 `r` 个像素越界，按边界夹取（等于把边缘像素当常数延拓）。中间段
/// 没有夹取也没有分支，是这条路径上唯一需要跑快的部分。
fn conv_row(row: &[f32], out: &mut [f32], base: usize, stride: usize, weights: &[f32], r: usize) {
    let w = row.len();
    let taps = weights.len();
    if w >= taps {
        for x in 0..r {
            out[base + x * stride] = clamp_taps(row, x, r, weights);
        }
        for x in r..w - r {
            let win = &row[x - r..x - r + taps];
            let mut acc = 0.0f32;
            for (a, b) in win.iter().zip(weights) {
                acc += a * b;
            }
            out[base + x * stride] = acc;
        }
        for x in w - r..w {
            out[base + x * stride] = clamp_taps(row, x, r, weights);
        }
    } else {
        // 图比卷积核还窄：整行都按夹取算，没有"中间段"可言。
        for x in 0..w {
            out[base + x * stride] = clamp_taps(row, x, r, weights);
        }
    }
}

/// 带夹取的一维卷积（只用于左右边缘与窄图）。
#[inline]
fn clamp_taps(row: &[f32], x: usize, r: usize, weights: &[f32]) -> f32 {
    let last = row.len() - 1;
    let mut acc = 0.0f32;
    for (k, wt) in weights.iter().enumerate() {
        let sx = (x + k).saturating_sub(r).min(last);
        acc += row[sx] * wt;
    }
    acc
}

/// 把圆角 alpha 与预乘写进模糊结果。
///
/// 只有四个角需要算 SDF：圆角半径以外的像素覆盖率恒为 255。四角方块按
/// 半径截断，整幅图跑一遍 SDF 在实时路径上（每秒十次）是纯浪费。
fn apply_round_alpha(rgba: &mut [u8], w: usize, h: usize, corner: f32) {
    if corner <= 0.0 {
        return; // `blur_rgba` 已经把 alpha 写成 255
    }
    let (fw, fh) = (w as f32, h as f32);
    let n = (corner.ceil() as usize + 1).min(w).min(h);
    if 2 * n > w || 2 * n > h {
        // 面板小到四个角重叠：退回整幅扫描，避免同一像素被乘两次 alpha。
        for y in 0..h {
            for x in 0..w {
                let a = rounded_coverage(x as f32, y as f32, fw, fh, corner);
                put_alpha(rgba, (y * w + x) * 4, a);
            }
        }
        return;
    }
    for y in 0..n {
        for x in 0..n {
            for (px, py) in [
                (x, y),
                (w - 1 - x, y),
                (x, h - 1 - y),
                (w - 1 - x, h - 1 - y),
            ] {
                let a = rounded_coverage(px as f32, py as f32, fw, fh, corner);
                put_alpha(rgba, (py * w + px) * 4, a);
            }
        }
    }
}

/// 写一个像素的 alpha，并把它乘进 RGB。
///
/// egui 的 `ColorImage` 在这一步之后按**预乘**解释：圆角外侧 alpha→0 时
/// 若 RGB 还留着亮色，线性采样会把亮边拖进圆角。
fn put_alpha(rgba: &mut [u8], off: usize, a: u8) {
    rgba[off + 3] = a;
    if a < 255 {
        let k = f32::from(a) / 255.0;
        for c in 0..3 {
            rgba[off + c] = (f32::from(rgba[off + c]) * k).round() as u8;
        }
    }
}

/// 抓一次背景并做成可直接上传的图像。
///
/// - `hwnd`：目标窗口（开窗前的同步路径里**必须还没显示**）；
/// - `ppp`：每逻辑点的物理像素数，用来把圆角半径换到捕获图尺度；
/// - `corner_pt`：面板圆角（逻辑点）；
/// - `radius`：高斯半径（捕获图 texel）。
///
/// 返回 `None` 时调用方应退回实心底色，不要留一张空白纹理 —— 那会让整个
/// 面板变成透明，比没有毛玻璃更糟。
#[must_use]
fn capture_once(hwnd: isize, ppp: f32, corner_pt: f32, radius: u32) -> Option<ColorImage> {
    let mut cap = DesktopCapture::new(true).ok()?;
    let (bytes, w, h) = cap.capture_behind_pct(hwnd, CAPTURE_PCT).ok()??;
    compose(bytes, w, h, Spec { radius, corner_pt }, ppp)
}

/// 抓到的原始 BGRA → 模糊 → 圆角 → egui 图像。
///
/// 捕获图是窗口的 `CAPTURE_PCT%`，圆角与 ppp 都要按同一比例缩。
fn compose(bits: &[u8], w: u32, h: u32, spec: Spec, ppp: f32) -> Option<ColorImage> {
    let (w, h) = (w as usize, h as usize);
    let need = w.checked_mul(h)?.checked_mul(4)?;
    if w == 0 || h == 0 || bits.len() < need {
        return None;
    }
    let mut rgba = blur_rgba(&bits[..need], w, h, spec.radius);
    if rgba.is_empty() {
        return None;
    }
    let scale = f32::from(CAPTURE_PCT) / 100.0;
    apply_round_alpha(&mut rgba, w, h, spec.corner_pt * ppp * scale);
    Some(ColorImage::from_rgba_premultiplied([w, h], &rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 常量图模糊后不变（权重归一化，和必须为 1）。
    #[test]
    fn constant_image_stays_constant() {
        let src = vec![90u8; 5 * 5 * 4];
        let out = blur_rgba(&src, 5, 5, 4);
        assert_eq!(out.len(), src.len());
        assert_eq!(out[0], 90);
        assert_eq!(out[1], 90);
        assert_eq!(out[2], 90);
        assert_eq!(out[3], 255);
    }

    /// 单点会被摊开，且能量守恒（近似：所有像素之和 ≈ 原点值）。
    #[test]
    fn impulse_spreads_and_conserves_energy() {
        let w = 9;
        let h = 9;
        let mut src = vec![0u8; w * h * 4];
        for px in src.as_chunks_mut::<4>().0 {
            px[3] = 255;
        }
        src[(4 * w + 4) * 4] = 255;
        let out = blur_rgba(&src, w, h, 3);
        let sum: u32 = out.as_chunks::<4>().0.iter().map(|p| u32::from(p[0])).sum();
        assert!((220..290).contains(&sum), "能量应接近 255，实际 {sum}");
        assert!(out[(4 * w + 4) * 4] < 255, "中心应当被摊薄");
        assert!(out[(4 * w + 3) * 4] > 0, "核半径内的邻居该分到能量");
        assert_eq!(out[0], 0, "核半径外不该有能量");
    }

    /// 非方图（宽 ≠ 高）也要模糊得对：转置写不能让轴搞混。
    ///
    /// 横向放一条亮线、纵向全暗，模糊后亮带应当保持在中间几行，不会跑到列上去。
    #[test]
    fn non_square_keeps_axes_straight() {
        let (w, h) = (24usize, 8usize);
        let mut src = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                src[(y * w + x) * 4] = if y == h / 2 { 200 } else { 0 };
            }
        }
        let out = blur_rgba(&src, w, h, 2);
        let row = |y: usize| u32::from(out[(y * w + w / 2) * 4]);
        assert!(row(h / 2) > 100, "亮带应留在原来的行上");
        assert!(row(0) < 20 && row(h - 1) < 20, "角落不该被点亮");
    }

    /// 与朴素二维卷积逐像素对拍（±1 舍入）。
    ///
    /// 可分离 + 平面 + 转置写的实现改过好几处索引，光看代码很难确信没把
    /// 横竖搞混：这里用最直白的 `Σ row[y][x] * w * w` 二维卷积当基准，
    /// 覆盖非方图与奇数半径 —— 一旦观感变了，先红的是这条。
    #[test]
    fn separable_blur_matches_naive_2d() {
        let (w, h, radius) = (13usize, 9usize, 2u32);
        // 确定的伪随机内容（不用外部 crate，也不依赖时间）。
        let mut seed = 0x1234_5678u32;
        let mut src = vec![0u8; w * h * 4];
        for (i, px) in src.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let mut next = || {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 24) as u8
            };
            px[0] = next();
            px[1] = next();
            px[2] = next();
            px[3] = 255;
            let _ = i;
        }

        let fast = blur_rgba(&src, w, h, radius);
        let weights = normalized_gaussian_weights(radius, sigma_for_radius(radius));
        let r = clamp_radius(radius) as i32;
        let taps = weights.len() as i32;
        assert_eq!(taps, 2 * r + 1, "权重个数必须是 2r+1");

        for y in 0..h {
            for x in 0..w {
                for c in 0..3 {
                    // 真正的二维卷积：核是两个一维权重的外积（不是对角线和）。
                    let mut acc = 0.0f32;
                    for (j, wy) in weights.iter().enumerate() {
                        let sy = (y as i32 + j as i32 - r).clamp(0, h as i32 - 1) as usize;
                        for (k, wx) in weights.iter().enumerate() {
                            let sx = (x as i32 + k as i32 - r).clamp(0, w as i32 - 1) as usize;
                            acc += f32::from(src[(sy * w + sx) * 4 + c]) * wy * wx;
                        }
                    }
                    let want = acc.round().clamp(0.0, 255.0) as i32;
                    let got = i32::from(fast[(y * w + x) * 4 + c]);
                    assert!(
                        (got - want).abs() <= 1,
                        "({x},{y}) 通道 {c}：可分离={got} 朴素={want}"
                    );
                }
            }
        }
    }

    /// 合法性：非法尺寸早退。
    #[test]
    fn malformed_buffers_do_not_panic() {
        assert!(blur_rgba(&[], 0, 0, 4).is_empty());
        assert!(blur_rgba(&[0u8; 4], 10, 10, 4).is_empty());
    }

    /// 圆角：中心与内部不透明，四角透明，边缘是渐变。
    #[test]
    fn rounded_mask_keeps_center_and_cuts_corners() {
        let (w, h, r) = (100.0f32, 100.0f32, 12.0f32);
        assert_eq!(rounded_coverage(50.0, 50.0, w, h, r), 255);
        assert_eq!(rounded_coverage(0.0, 0.0, w, h, r), 0);
        assert_eq!(rounded_coverage(99.0, 0.0, w, h, r), 0);
        // 方角（radius=0）不能走 SDF：上游那个 SDF 在内部恒为 -radius，
        // 半径 0 时处处返回 0，会把整块面板钉在 50% 覆盖率。
        assert_eq!(rounded_coverage(0.0, 0.0, w, h, 0.0), 255);
    }

    /// 只算四角的 alpha 必须和整幅扫描完全一致 —— 这是实时路径的性能前提，
    /// 一旦角块尺寸算少了，圆角外侧就会出现没被扣掉的亮边。
    #[test]
    fn corner_only_alpha_matches_full_scan() {
        for (w, h, corner) in [(120usize, 80usize, 12.0f32), (200, 150, 7.3), (60, 60, 4.0)] {
            // 模拟 `blur_rgba` 的产物：RGB 是内容，alpha 已经全 255。
            let start: Vec<u8> = (0..w * h).flat_map(|_| [200u8, 200, 200, 255]).collect();
            let mut fast = start.clone();
            apply_round_alpha(&mut fast, w, h, corner);

            // 整幅扫描的参考实现（预乘写全，不看 `put_alpha`）。
            let mut slow = start;
            for y in 0..h {
                for x in 0..w {
                    let a = rounded_coverage(x as f32, y as f32, w as f32, h as f32, corner);
                    let off = (y * w + x) * 4;
                    slow[off + 3] = a;
                    let k = f32::from(a) / 255.0;
                    for c in 0..3 {
                        slow[off + c] = (200.0f32 * k).round() as u8;
                    }
                }
            }
            assert_eq!(
                fast, slow,
                "{w}x{h} corner={corner} 的四角裁剪应等价于整幅扫描"
            );
        }
    }

    /// 方角面板一个字节都不该动（模糊已经写成 255），更不能做预乘。
    #[test]
    fn square_panel_leaves_alpha_alone() {
        let start: Vec<u8> = (0..8 * 8).flat_map(|_| [180u8, 90, 30, 255]).collect();
        let mut buf = start.clone();
        apply_round_alpha(&mut buf, 8, 8, 0.0);
        assert_eq!(buf, start);
    }

    /// 毛玻璃底色确实比实心底色更透。
    #[test]
    fn frost_background_is_more_transparent() {
        assert!(panel_bg(true, true).a() < panel_bg(true, false).a());
        assert_eq!(panel_bg(true, false).a(), 250);
    }
}
