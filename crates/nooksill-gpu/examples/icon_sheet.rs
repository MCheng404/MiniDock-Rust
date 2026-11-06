//! 图标总览：把 Dock 与控制中心的全部矢量图标离屏排成一张表。
//!
//! 这两处图标在真窗口里分别藏在 Dock 悬停放大动画和控制中心弹层里，
//! 改完形状好不好看、缩放会不会错位，靠真机点很慢。这个例子走与真实
//! 窗口相同的 `FrameRenderer`，只是最后把像素写成 BMP。
//!
//! ```text
//! cargo run -p nooksill-gpu --example icon_sheet -- 出图.bmp [dark] [accent=3b7ddd] [dpi=125]
//!
//! `dpi=125` / `dpi=150` 按 Windows 分数缩放出物理尺寸放大图，用来核对
//! 线宽分档、圆头与像素网格对齐在非整数 DPI 下是否依然清晰。
//! ```

#[path = "common/bmp.rs"]
mod bmp;

use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect};
use nooksill_core::control::Tile;
use nooksill_gpu::frame::FrameRenderer;
use nooksill_gpu::glyph::{self, Builtin, IconState, IconStyle};
use nooksill_gpu::gpu::Gpu;

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
    // Windows 分数缩放：100 / 125 / 150%。逻辑画布不变，物理像素按 ppp 放大。
    let ppp = rest
        .iter()
        .find_map(|a| a.strip_prefix("dpi="))
        .and_then(|v| v.parse::<f32>().ok())
        .map_or(1.0, |v| (v / 100.0).clamp(0.8, 3.0));
    let (pw, ph) = (
        (W as f32 * ppp).round() as u32,
        (H as f32 * ppp).round() as u32,
    );
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
    let mut frame = FrameRenderer::new(&gpu, pw, ph, ppp)?;
    frame.ctx().set_theme(if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });

    let mut pixels = Vec::new();
    for t in [0.0_f64, 0.4, 10.0] {
        let (px, _) = frame.render_rgba(&gpu, raw_input(t, ppp), |ui| {
            sheet(ui, dark, accent);
        })?;
        // 三帧：字体图集 → 动画推进 → 稳定画面，只保留最后一帧。
        pixels = px;
    }
    let bg = if dark { [24, 25, 30] } else { [246, 247, 250] };
    bmp::write_bmp(&out, pw, ph, &pixels, move |_, _| bg)?;
    println!(
        "已写出 {out}（逻辑 {W}×{H} / 物理 {pw}×{ph}，{}，{}% DPI）",
        if dark { "深色" } else { "浅色" },
        (ppp * 100.0).round() as u32
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

    let style = |color: Color32| IconStyle::plain(color);
    let dock = |a: Color32| IconStyle::new(Color32::WHITE, a, IconState::default());
    let st = |state: IconState| IconStyle::new(fg, accent, state);

    // 第一行：Dock 内置、首字占位、条带、喇叭。
    cell(p, &mut n, "Start", &|p, r| {
        glyph::paint_tile(p, r, Builtin::Start, dock(accent), dark)
    });
    cell(p, &mut n, "Launchpad", &|p, r| {
        glyph::paint_tile(p, r, Builtin::Launchpad, dock(accent), dark)
    });
    cell(p, &mut n, "Monogram", &|p, r| {
        glyph::paint_monogram(p, r, 'M', dock(accent), dark)
    });
    cell(p, &mut n, "Sliders", &|p, r| {
        glyph::sliders(p, r, style(fg))
    });
    cell(p, &mut n, "Speaker", &|p, r| {
        glyph::speaker(p, r, style(fg))
    });
    cell(p, &mut n, "Muted", &|p, r| {
        glyph::speaker(
            p,
            r,
            IconStyle::new(
                fg,
                fg,
                IconState {
                    muted: true,
                    ..IconState::default()
                },
            ),
        )
    });
    cell(p, &mut n, "Start · pink", &|p, r| {
        glyph::paint_tile(
            p,
            r,
            Builtin::Start,
            dock(Color32::from_rgb(224, 86, 138)),
            dark,
        )
    });

    // 第二行：控制中心 9 个磁贴图标。
    for (tile, label) in tiles {
        cell(p, &mut n, label, &|p, r| {
            glyph::control_tile(p, r, tile, style(fg))
        });
    }

    // 第三行：小尺寸（24px）与开关态（白图标 + 主题色底）。
    cell(p, &mut n, "Wifi · 24px", &|p, r| {
        let small = Rect::from_center_size(r.center(), egui::vec2(24.0, 24.0));
        p.rect_filled(small, CornerRadius::same(4), tonal(dark));
        glyph::control_tile(p, small, Tile::Wifi, style(fg))
    });
    cell(p, &mut n, "Settings · 24px", &|p, r| {
        let small = Rect::from_center_size(r.center(), egui::vec2(24.0, 24.0));
        p.rect_filled(small, CornerRadius::same(4), tonal(dark));
        glyph::control_tile(p, small, Tile::Settings, style(fg))
    });
    cell(p, &mut n, "Start · 24px", &|p, r| {
        let small = Rect::from_center_size(r.center(), egui::vec2(24.0, 24.0));
        glyph::paint_tile(p, small, Builtin::Start, dock(accent), dark)
    });
    cell(p, &mut n, "Wifi · ON", &|p, r| {
        let chip = Rect::from_center_size(r.center(), egui::vec2(48.0, 48.0));
        p.rect_filled(chip, CornerRadius::same(14), accent);
        glyph::control_tile(p, chip, Tile::Wifi, style(Color32::WHITE))
    });
    cell(p, &mut n, "Gear · ON", &|p, r| {
        let chip = Rect::from_center_size(r.center(), egui::vec2(48.0, 48.0));
        p.rect_filled(chip, CornerRadius::same(14), accent);
        glyph::control_tile(p, chip, Tile::Settings, style(Color32::WHITE))
    });

    // 第四行：动态状态（level / phase）与主题色开关对比 —— 以后接动画
    // 只改 IconState，绘制代码不用动。
    cell(p, &mut n, "Wifi · lv .33", &|p, r| {
        glyph::control_tile(p, r, Tile::Wifi, st(IconState::default().with_level(0.33)))
    });
    cell(p, &mut n, "Speaker · lv .5", &|p, r| {
        glyph::speaker(p, r, st(IconState::default().with_level(0.5)))
    });
    cell(p, &mut n, "Energy · lv .4", &|p, r| {
        glyph::control_tile(p, r, Tile::Energy, st(IconState::default().with_level(0.4)))
    });
    cell(p, &mut n, "Launch · lv .55", &|p, r| {
        glyph::paint_tile(
            p,
            r,
            Builtin::Launchpad,
            IconStyle::new(
                Color32::WHITE,
                accent,
                IconState::default().with_level(0.55),
            ),
            dark,
        )
    });
    cell(p, &mut n, "Gear · phase", &|p, r| {
        glyph::control_tile(
            p,
            r,
            Tile::Settings,
            st(IconState::default().with_phase(0.5)),
        )
    });
    cell(p, &mut n, "Hotspot · phase", &|p, r| {
        glyph::control_tile(
            p,
            r,
            Tile::Hotspot,
            st(IconState::default().with_phase(0.25)),
        )
    });
    cell(p, &mut n, "accent off / on", &|p, r| {
        let left = Rect::from_center_size(
            Pos2::new(r.center().x - 15.0, r.center().y),
            egui::vec2(26.0, 26.0),
        );
        let right = left.translate(egui::vec2(30.0, 0.0));
        // 关：中性前景色；开：主题色。两个独立开关只影响 color。
        glyph::control_tile(p, left, Tile::Wifi, style(fg));
        glyph::control_tile(p, right, Tile::Wifi, style(accent));
    });
}

fn tonal(dark: bool) -> Color32 {
    if dark {
        Color32::from_rgba_unmultiplied(255, 255, 255, 40)
    } else {
        Color32::from_rgba_unmultiplied(255, 255, 255, 200)
    }
}

fn raw_input(time: f64, ppp: f32) -> egui::RawInput {
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
            native_pixels_per_point: Some(ppp),
            ..Default::default()
        },
    );
    raw
}
