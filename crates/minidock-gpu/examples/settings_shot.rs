//! 把设置界面离屏渲染成 BMP，用来和 `docs/settings-prototype.html` 对照。
//!
//! 为什么需要它：设置窗口是分层窗口，只有真跑起来才看得到；而"布局改对了
//! 没有"这件事又不该只能靠人肉截图。这个例子走**与真实窗口完全相同的**
//! 渲染路径（`FrameRenderer` + `settings_win::shell`），只是最后一步不调
//! `UpdateLayeredWindow`，改成把像素写成文件。
//!
//! ```text
//! cargo run -p minidock-gpu --example settings_shot -- 输出.bmp [页名] [深色]
//! ```
//!
//! - `页名`：`常规` / `外观` / `位置` / `图标` / `交互` / `附件` / `效果` /
//!   `性能` / `图标库` / `添加`，省略则为「外观」。
//! - `深色`：填 `dark` 走深色主题。
//!
//! 不需要窗口，也不需要显示器 —— 有可用的 DX12 适配器就行。

use std::io::Write as _;

use minidock_core::config::Config;
use minidock_gpu::frame::FrameRenderer;
use minidock_gpu::gpu::Gpu;
use minidock_gpu::settings_win::{shell, LOGICAL_SIZE};
use minidock_settings::{Outcome, Page, SettingsState};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let out_path = args.next().unwrap_or_else(|| "settings.bmp".to_owned());
    let page_name = args.next();
    let dark = args.next().is_some_and(|a| a == "dark");

    let page = match page_name.as_deref() {
        None => Page::Appearance,
        Some(name) => Page::ALL
            .into_iter()
            .find(|p| p.title() == name)
            .ok_or_else(|| format!("没有叫「{name}」的页；可选：{:?}", titles()))?,
    };

    let (lw, lh) = LOGICAL_SIZE;
    // 截图固定 1.0：BMP 没有 DPI 概念，用 2 倍只是把文件变大四倍。
    let ppp = 1.0_f32;
    let (w, h) = ((lw * ppp) as u32, (lh * ppp) as u32);

    let gpu = Gpu::new("")?;
    let mut frame = FrameRenderer::new(&gpu, w, h, ppp)?;
    frame.ctx().set_theme(if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });

    let mut cfg = Config::defaults(96);
    let mut state = SettingsState {
        page,
        config_dir: r"C:\Users\you\AppData\Roaming\MiniDock".to_owned(),
        monitors: vec!["DISPLAY1  1920×1080".to_owned()],
        adapters: vec!["AMD Radeon(TM) 610M".to_owned()],
        active_adapter: "AMD Radeon(TM) 610M".to_owned(),
        stats: "空闲 · 0 帧/秒 · 上一次出帧 42 ms 前".to_owned(),
        ..SettingsState::default()
    };
    let mut out = Outcome::default();

    let raw = raw_input((w, h), ppp);
    // 第一帧只是让 egui 量出文字尺寸并上传字形图集。
    let _ = frame.render_rgba(&gpu, raw.clone(), |ui| {
        shell(ui, &mut state, &mut cfg, &mut out)
    })?;
    // 第二帧才是"字体已就绪"的稳定画面。
    let (pixels, _) =
        frame.render_rgba(&gpu, raw, |ui| shell(ui, &mut state, &mut cfg, &mut out))?;

    write_bmp(&out_path, w, h, &pixels, dark)?;
    println!(
        "已写出 {out_path}（{}×{}，{:?}，{}）",
        w,
        h,
        page,
        if dark { "深色" } else { "浅色" }
    );
    Ok(())
}

fn titles() -> Vec<&'static str> {
    Page::ALL.iter().map(|p| p.title()).collect()
}

/// 单帧输入。与 `bin/dock.rs` 里的 `raw_input` 同口径。
fn raw_input(size: (u32, u32), ppp: f32) -> egui::RawInput {
    let (w, h) = size;
    let mut raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(w as f32 / ppp, h as f32 / ppp),
        )),
        time: Some(0.0),
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

/// 24 位 BMP，把预乘 BGRA 合成到浅灰底上。
///
/// 不用 PNG：那要引一个编码库，而这里只是给人看一眼。也不保留 alpha：
/// 分层窗口的透明区域直接看是黑的，合成到灰底才能看清圆角。
fn write_bmp(path: &str, w: u32, h: u32, bgra: &[u8], dark: bool) -> std::io::Result<()> {
    // 与 `settings_win::shell` 画的窗口底色一致：合成用的底必须跟着主题走，
    // 否则深色截图会得到"浅灰窗口 + 深色卡片"这种真实窗口里不存在的画面。
    let bg: [u8; 3] = if dark { [24, 25, 30] } else { [246, 247, 250] };
    // 行按 4 字节对齐（BMP 的 stride 规则）。
    let row_stride = ((w as usize) * 3).div_ceil(4) * 4;
    let data_len = row_stride * h as usize;
    let mut buf = Vec::with_capacity(54 + data_len);

    // BITMAPFILEHEADER
    buf.extend_from_slice(b"BM");
    buf.extend_from_slice(&(54 + data_len as u32).to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&54u32.to_le_bytes());
    // BITMAPINFOHEADER
    buf.extend_from_slice(&40u32.to_le_bytes());
    buf.extend_from_slice(&(w as i32).to_le_bytes());
    buf.extend_from_slice(&(h as i32).to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&24u16.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes()); // 无压缩
    buf.extend_from_slice(&(data_len as u32).to_le_bytes());
    buf.extend_from_slice(&2835i32.to_le_bytes()); // 72 DPI
    buf.extend_from_slice(&2835i32.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());

    // BMP 是自下而上存的。
    for y in (0..h).rev() {
        let mut written = 0;
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let (b, g, r, a) = (bgra[i], bgra[i + 1], bgra[i + 2], bgra[i + 3]);
            // 预乘 alpha：直接加上背景的 (1-a) 部分。
            let inv = 255 - a as u32;
            let blend = |c: u8, bg: u8| -> u8 { (c as u32 + bg as u32 * inv / 255).min(255) as u8 };
            buf.push(blend(b, bg[2]));
            buf.push(blend(g, bg[1]));
            buf.push(blend(r, bg[0]));
            written += 3;
        }
        // 行尾补齐到 4 字节边界（BMP 的 stride 规则）。
        buf.extend(std::iter::repeat_n(0u8, row_stride - written));
    }

    let mut f = std::fs::File::create(path)?;
    f.write_all(&buf)
}
