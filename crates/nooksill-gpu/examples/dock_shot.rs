//! 把 Dock 栏离屏渲染成 BMP，用来验收倒影/边框这类"只有真跑才看得见"的效果。
//!
//! ```text
//! cargo run -p nooksill-gpu --example dock_shot -- out.bmp
//! cargo run -p nooksill-gpu --example dock_shot -- out.bmp dark
//! ```
//!
//! 走的是和主程序**同一条**渲染路径（`FrameRenderer` + `Dock::paint`），
//! 只是最后一步不 `UpdateLayeredWindow`，改成写文件。
//!
//! 倒影在真实窗口里是呈现层做的（`present::mirror_rows` 直接改 DIB），
//! 离屏没有 DIB，所以这里对回读缓冲做同一件事 —— 用的是同一个函数，
//! 只是每像素 RGBA 而不是 BGRA；四通道同乘衰减，通道顺序不影响结果。

use std::io::Write;

use nooksill_core::config::{Config, Key};
use nooksill_core::layout::Rect as LRect;
use nooksill_gpu::dock::{Dock, DockItem};
use nooksill_gpu::frame::FrameRenderer;
use nooksill_gpu::gpu::Gpu;
use nooksill_gpu::present::mirror_rows;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| "dock.bmp".to_owned());
    let dark = args.next().as_deref() != Some("light");
    let ppp = 1.25_f32;

    let mut cfg = Config::defaults(120);
    cfg.reflect = true;
    cfg.apply_i32(Key::REFL_SIZE, 55);
    cfg.apply_i32(Key::REFL_GAP, 6);
    cfg.apply_i32(Key::REFL_OPACITY, 55);
    cfg.apply_i32(Key::BORDER_WIDTH, 3);
    cfg.border = true;
    cfg.theme = nooksill_core::config::DockTheme::Dark;
    cfg.mode = nooksill_core::config::DockMode::Always;

    // 用真实条目构造：名字决定首字占位图标，路径指向 system32 里的常见程序。
    let items: Vec<DockItem> = [
        ("资源管理器", r"C:\Windows\explorer.exe"),
        ("记事本", r"C:\Windows\System32\notepad.exe"),
        ("画图", r"C:\Windows\System32\mspaint.exe"),
        ("计算器", r"C:\Windows\System32\calc.exe"),
    ]
    .iter()
    .map(|(name, path)| {
        DockItem::from_item(&nooksill_core::item::Item::new(
            nooksill_core::item::ItemKind::File,
            *name,
            *path,
        ))
    })
    .collect();

    let work = LRect::new(0, 0, 1280, 900);
    let mut dock = Dock::new(cfg.clone(), items, work);
    dock.update(120, nooksill_core::layout::REF_FRAME_MS);

    let (w, h) = dock.physical_size_i32();
    let (w, h) = (w as u32, h as u32);
    let gpu = Gpu::new("")?;
    let mut frame = FrameRenderer::new(&gpu, w, h, ppp)?;
    let clock = None;
    let (rgba, _) = frame.render_rgba(&gpu, raw_input(w, h, ppp), |ui| {
        dock.paint(ui, ppp, dark, clock);
    })?;

    // 与真实窗口完全一致的倒影：同一条 `mirror_rows`。
    let mut pixels = rgba.clone();
    if let Some(spec) = dock.reflection_spec() {
        let stride = w as usize;
        mirror_rows(&mut pixels, stride, h as i32, spec);
        println!(
            "倒影：src_top={} src_h={} gap={} height={} opacity={:.2}",
            spec.src_top, spec.src_h, spec.gap, spec.height, spec.opacity
        );
    }
    write_bmp(&path, w, h, &pixels, dark)?;
    println!("已写出 {path}（{w}×{h}）");
    Ok(())
}

fn raw_input(w: u32, h: u32, ppp: f32) -> egui::RawInput {
    let mut raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(w as f32 / ppp, h as f32 / ppp),
        )),
        time: Some(10.0),
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

/// 24 位 BMP，预乘 alpha 合成到主题底色上（与 `settings_shot` 同一套规则）。
fn write_bmp(path: &str, w: u32, h: u32, rgba: &[u8], dark: bool) -> std::io::Result<()> {
    let bg: [u8; 3] = if dark { [18, 19, 23] } else { [238, 239, 243] };
    let row_stride = ((w as usize) * 3).div_ceil(4) * 4;
    let data_len = row_stride * h as usize;
    let mut buf = Vec::with_capacity(54 + data_len);
    buf.extend_from_slice(b"BM");
    buf.extend_from_slice(&(54 + data_len as u32).to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&54u32.to_le_bytes());
    buf.extend_from_slice(&40u32.to_le_bytes());
    buf.extend_from_slice(&(w as i32).to_le_bytes());
    buf.extend_from_slice(&(h as i32).to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&24u16.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&(data_len as u32).to_le_bytes());
    buf.extend_from_slice(&2835i32.to_le_bytes());
    buf.extend_from_slice(&2835i32.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());

    for y in (0..h).rev() {
        let mut written = 0;
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let (r, g, b, a) = (rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]);
            let inv = 255 - a as u32;
            let blend =
                |c: u8, back: u8| -> u8 { (c as u32 + back as u32 * inv / 255).min(255) as u8 };
            buf.push(blend(b, bg[2]));
            buf.push(blend(g, bg[1]));
            buf.push(blend(r, bg[0]));
            written += 3;
        }
        buf.extend(std::iter::repeat_n(0u8, row_stride - written));
    }
    let mut f = std::fs::File::create(path)?;
    f.write_all(&buf)
}
