//! 图标总览：把 Dock 与控制中心的全部矢量图标离屏排成一张表。
//!
//! 这两处图标在真窗口里分别藏在 Dock 悬停放大动画和控制中心弹层里，
//! 改完形状好不好看、缩放会不会错位，靠真机点很慢。这个例子走与真实
//! 窗口相同的 `FrameRenderer`，只是最后把像素写成 BMP。
//!
//! ```text
//! cargo run -p minidock-gpu --example icon_sheet -- 出图.bmp [dark] [accent=3b7ddd]
//! ```

use std::io::Write as _;

use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect};
use minidock_core::control::Tile;
use minidock_gpu::frame::FrameRenderer;
use minidock_gpu::glyph::{self, Builtin};
use minidock_gpu::gpu::Gpu;

const W: u32 = 780;
const H: u32 = 520;
const CELL: f32 = 108.0;
const ROW_H: f32 = 112.0;
const COLS: usize = 7;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "icons.bmp".to_owned());
    let rest: Vec<String> = args.collect();
    let dark = rest.iter().any(|a| a == "dark");
    let accent = rest
        .iter()
        .find_map(|a| a.strip_prefix("accent="))
        .and_then(|h| u32::from_str_radix(h, 16).ok())
        .map_or_else(
            || Color32::from_rgb(46, 123, 246),
            |v| {
                Color32::from_rgb(
                    ((v >> 16) & 0xFF) as u8,
                    ((v >> 8) & 0xFF) as u8,
                    (v & 0xFF) as u8,
                )
            },
        );

    let gpu = Gpu::new("")?;
    let mut frame = FrameRenderer::new(&gpu, W, H, 1.0)?;
    frame.ctx().set_theme(if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });

    let mut pixels = Vec::new();
    for t in [0.0_f64, 0.4, 10.0] {
        let (px, _) = frame.render_rgba(&gpu, raw_input(t), |ui| {
            sheet(ui, dark, accent);
        })?;
        // 三帧：字体图集 → 动画推进 → 稳定画面，只保留最后一帧。
        pixels = px;
    }
    let bg = if dark { [24, 25, 30] } else { [246, 247, 250] };
    write_bmp(&out, &pixels, bg)?;
    println!(
        "已写出 {out}（{W}×{H}，{}）",
        if dark { "深色" } else { "浅色" }
    );
    Ok(())
}

/// 一个格子：淡底 + 图标 + 英文标签（打包字体没有中文，避免豆腐块）。
fn sheet(ui: &mut egui::Ui, dark: bool, accent: Color32) {
    let p = ui.painter();
    let bg = if dark {
        Color32::from_rgb(24, 25, 30)
    } else {
        Color32::from_rgb(246, 247, 250)
    };
    p.rect_filled(ui.max_rect(), CornerRadius::ZERO, bg);
    let fg = if dark {
        Color32::from_rgb(238, 240, 246)
    } else {
        Color32::from_rgb(28, 30, 38)
    };
    p.text(
        Pos2::new(16.0, 14.0),
        Align2::LEFT_TOP,
        "Dock + Control Center · icon sheet",
        FontId::proportional(15.0),
        fg,
    );

    let tiles: [(Tile, &str); 9] = [
        (Tile::Wifi, "Wifi"),
        (Tile::Bluetooth, "Bluetooth"),
        (Tile::DarkMode, "DarkMode"),
        (Tile::Location, "Location"),
        (Tile::Mute, "Mute"),
        (Tile::Settings, "Settings"),
        (Tile::Energy, "Energy"),
        (Tile::Hotspot, "Hotspot"),
        (Tile::Ethernet, "Ethernet"),
    ];

    let mut n = 0;
    let cell =
        |p: &egui::Painter, n: &mut usize, label: &str, draw: &dyn Fn(&egui::Painter, Rect)| {
            let col = *n % COLS;
            let row = *n / COLS;
            let x = 12.0 + col as f32 * CELL;
            let y = 44.0 + row as f32 * ROW_H;
            let box_r = Rect::from_min_size(Pos2::new(x, y), egui::vec2(CELL - 8.0, ROW_H - 10.0));
            let tonal = if dark {
                Color32::from_rgba_unmultiplied(255, 255, 255, 22)
            } else {
                Color32::from_rgba_unmultiplied(23, 26, 35, 14)
            };
            p.rect_filled(box_r, CornerRadius::same(12), tonal);
            draw(
                p,
                Rect::from_center_size(
                    Pos2::new(box_r.center().x, y + 38.0),
                    egui::vec2(60.0, 60.0),
                ),
            );
            p.text(
                Pos2::new(box_r.center().x, y + 80.0),
                Align2::CENTER_CENTER,
                label,
                FontId::proportional(11.0),
                fg,
            );
            *n += 1;
        };

    // 第一行：Dock 内置、首字占位、条带、喇叭。
    cell(p, &mut n, "Start", &|p, r| {
        glyph::paint_tile(p, r, Builtin::Start, accent, dark)
    });
    cell(p, &mut n, "Launchpad", &|p, r| {
        glyph::paint_tile(p, r, Builtin::Launchpad, accent, dark)
    });
    cell(p, &mut n, "Monogram", &|p, r| {
        glyph::paint_monogram(p, r, 'M', accent, dark)
    });
    cell(p, &mut n, "Sliders", &|p, r| glyph::sliders(p, r, fg));
    cell(p, &mut n, "Speaker", &|p, r| {
        glyph::speaker(p, r, fg, false)
    });
    cell(p, &mut n, "Muted", &|p, r| glyph::speaker(p, r, fg, true));
    cell(p, &mut n, "Start · pink", &|p, r| {
        glyph::paint_tile(p, r, Builtin::Start, Color32::from_rgb(224, 86, 138), dark)
    });

    // 第二行：控制中心 9 个磁贴图标。
    for (tile, label) in tiles {
        cell(p, &mut n, label, &|p, r| {
            glyph::control_tile(p, r, tile, fg)
        });
    }

    // 第三行：小尺寸（24px，缩放一致性）与开关态（白字强调色底）。
    cell(p, &mut n, "Wifi · 24px", &|p, r| {
        let small = Rect::from_center_size(r.center(), egui::vec2(24.0, 24.0));
        p.rect_filled(small, CornerRadius::same(4), tonal(dark));
        glyph::control_tile(p, small, Tile::Wifi, fg)
    });
    cell(p, &mut n, "Settings · 24px", &|p, r| {
        let small = Rect::from_center_size(r.center(), egui::vec2(24.0, 24.0));
        p.rect_filled(small, CornerRadius::same(4), tonal(dark));
        glyph::control_tile(p, small, Tile::Settings, fg)
    });
    cell(p, &mut n, "Start · 24px", &|p, r| {
        let small = Rect::from_center_size(r.center(), egui::vec2(24.0, 24.0));
        glyph::paint_tile(p, small, Builtin::Start, accent, dark)
    });
    cell(p, &mut n, "Wifi · ON", &|p, r| {
        let chip = Rect::from_center_size(r.center(), egui::vec2(48.0, 48.0));
        p.rect_filled(chip, CornerRadius::same(14), accent);
        glyph::control_tile(p, chip, Tile::Wifi, Color32::WHITE)
    });
    cell(p, &mut n, "Gear · ON", &|p, r| {
        let chip = Rect::from_center_size(r.center(), egui::vec2(48.0, 48.0));
        p.rect_filled(chip, CornerRadius::same(14), accent);
        glyph::control_tile(p, chip, Tile::Settings, Color32::WHITE)
    });
}

fn tonal(dark: bool) -> Color32 {
    if dark {
        Color32::from_rgba_unmultiplied(255, 255, 255, 40)
    } else {
        Color32::from_rgba_unmultiplied(255, 255, 255, 200)
    }
}

fn raw_input(time: f64) -> egui::RawInput {
    let mut raw = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(
            Pos2::ZERO,
            egui::vec2(W as f32, H as f32),
        )),
        time: Some(time),
        ..Default::default()
    };
    raw.viewports.insert(
        egui::ViewportId::ROOT,
        egui::ViewportInfo {
            native_pixels_per_point: Some(1.0),
            ..Default::default()
        },
    );
    raw
}

/// 与 `settings_shot` 同一套 BMP 写出（回读缓冲是 RGBA，BMP 像素序 BGR）。
fn write_bmp(path: &str, rgba: &[u8], bg: [u8; 3]) -> std::io::Result<()> {
    let row_stride = ((W as usize) * 3).div_ceil(4) * 4;
    let mut buf = Vec::with_capacity(54 + row_stride * H as usize);
    buf.extend_from_slice(b"BM");
    buf.extend_from_slice(&((54 + row_stride * H as usize) as u32).to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&54u32.to_le_bytes());
    buf.extend_from_slice(&40u32.to_le_bytes());
    buf.extend_from_slice(&(W as i32).to_le_bytes());
    buf.extend_from_slice(&(H as i32).to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&24u16.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&((row_stride * H as usize) as u32).to_le_bytes());
    buf.extend_from_slice(&2835i32.to_le_bytes());
    buf.extend_from_slice(&2835i32.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    for y in (0..H).rev() {
        let mut written = 0;
        for x in 0..W {
            let i = ((y * W + x) * 4) as usize;
            let (r, g, b, a) = (rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]);
            let inv = 255 - a as u32;
            let blend =
                |c: u8, bgc: u8| -> u8 { (c as u32 + bgc as u32 * inv / 255).min(255) as u8 };
            buf.push(blend(b, bg[2]));
            buf.push(blend(g, bg[1]));
            buf.push(blend(r, bg[0]));
            written += 3;
        }
        buf.extend(std::iter::repeat_n(0u8, row_stride - written));
    }
    std::fs::File::create(path)?.write_all(&buf)
}
