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

#[path = "common/bmp.rs"]
mod bmp;

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
    // `grad`：开三色斜向渐变（`BgColor3` + `GradAngle`），用来肉眼验这两个键。
    let demo_grad = args.next().as_deref() == Some("grad");
    let ppp = 1.25_f32;

    let mut cfg = Config::defaults(120);
    cfg.reflect = true;
    cfg.apply_i32(Key::REFL_SIZE, 55);
    cfg.apply_i32(Key::REFL_GAP, 6);
    cfg.apply_i32(Key::REFL_OPACITY, 55);
    cfg.apply_i32(Key::BORDER_WIDTH, 3);
    if demo_grad {
        cfg.apply_i32(Key::BG_COLOR3, 0x3A7BD5); // 中段：蓝
        cfg.apply_i32(Key::GRAD_ANGLE, 30);
        cfg.apply_i32(Key::GRAD_A1, 100);
        cfg.apply_i32(Key::GRAD_A2, 55);
        cfg.apply_i32(Key::GRAD_A3, 90);
        cfg.apply_i32(Key::BgMode, 1);
    }
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
    let bg = if dark { [18, 19, 23] } else { [238, 239, 243] };
    bmp::write_bmp(&path, w, h, &pixels, move |_, _| bg)?;
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
