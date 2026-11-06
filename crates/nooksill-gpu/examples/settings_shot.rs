//! 把设置界面离屏渲染成 BMP，用来和 `docs/settings-prototype.html` 对照。
//!
//! 为什么需要它：设置窗口是分层窗口，只有真跑起来才看得到；而"布局改对了
//! 没有"这件事又不该只能靠人肉截图。这个例子走**与真实窗口完全相同的**
//! 渲染路径（`FrameRenderer` + `settings_win::shell`），只是最后一步不调
//! `UpdateLayeredWindow`，改成把像素写成文件。
//!
//! ```text
//! cargo run -p nooksill-gpu --example settings_shot -- 输出.bmp [页名] [深色]
//! ```
//!
//! - `页名`：`常规` / `外观` / `位置` / `图标` / `交互` / `附件` / `效果` /
//!   `性能` / `图标库` / `添加`，省略则为「外观」。
//! - `深色`：填 `dark` 走深色主题。
//!
//! 不需要窗口，也不需要显示器 —— 有可用的 DX12 适配器就行。

#[path = "common/bmp.rs"]
mod bmp;

use nooksill_core::config::Config;
use nooksill_core::config::Key;
use nooksill_gpu::frame::FrameRenderer;
use nooksill_gpu::gpu::Gpu;
use nooksill_gpu::settings_win::{shell_with_frost, LOGICAL_SIZE};
use nooksill_settings::{Outcome, Page, SettingsState};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let out_path = args.next().unwrap_or_else(|| "settings.bmp".to_owned());
    let page_name = args.next();
    // 剩下的参数是开关（顺序随意）：`dark` 深色主题，`tip` 展开一张说明
    // 气泡（气泡由悬停/点击驱动，离屏渲染没有指针，只能直接塞状态）。
    let rest: Vec<String> = args.collect();
    let dark = rest.iter().any(|a| a == "dark");
    let tip = rest.iter().any(|a| a == "tip");
    // `focus`：模拟搜索框获得焦点，核对焦点光晕。
    let focus = rest.iter().any(|a| a == "focus");
    // 界面缩放（自定义 DPI）：裸数字按百分比（150 -> 1.5）。验证 UiScale
    // 键的渲染 —— 字体与所有元素一起放大，逻辑布局不变。
    let scale = rest
        .iter()
        .filter_map(|a| a.parse::<f32>().ok())
        .next()
        .map_or(1.0, |v| (v / 100.0).clamp(0.8, 3.0));

    let page = match page_name.as_deref() {
        None => Page::Appearance,
        Some(name) => Page::ALL
            .into_iter()
            .find(|p| p.title() == name)
            .ok_or_else(|| format!("没有叫「{name}」的页；可选：{:?}", titles()))?,
    };

    let (lw, lh) = LOGICAL_SIZE;
    let ppp = scale;
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
        config_dir: r"C:\Users\you\AppData\Roaming\NookSill".to_owned(),
        monitors: vec!["DISPLAY1  1920×1080".to_owned()],
        adapters: vec!["AMD Radeon(TM) 610M".to_owned()],
        active_adapter: "AMD Radeon(TM) 610M".to_owned(),
        stats: "空闲 · 0 帧/秒 · 上一次出帧 42 ms 前".to_owned(),
        ..SettingsState::default()
    };
    // `q=文字`：预填搜索框内容，核对输入文字与图标/提示的对齐。
    if let Some(text) = rest.iter().find_map(|a| a.strip_prefix("q=")) {
        state.nav_filter = text.to_owned();
    }
    // `accent=RRGGBB`：自定义主题色出图（默认 -1 = 跟随系统）。
    if let Some(hex) = rest.iter().find_map(|a| a.strip_prefix("accent=")) {
        if let Ok(v) = u32::from_str_radix(hex, 16) {
            // COLORREF 是 0x00BBGGRR，参数是 0xRRGGBB。
            let r = (v >> 16) & 0xFF;
            let g = (v >> 8) & 0xFF;
            let b = v & 0xFF;
            cfg.apply_i32(Key::UI_ACCENT, (b << 16 | g << 8 | r) as i32);
        }
    }
    if tip {
        state.tip = Some(nooksill_settings::pages::Tip {
            id: egui::Id::unique("shot"),
            text: "根据 Windows 强调色生成 Material You 风格色调".to_owned(),
            anchor: egui::Rect::from_min_size(egui::pos2(560.0, 300.0), egui::vec2(120.0, 20.0)),
            pinned: true,
        });
    }
    let mut out = Outcome::default();

    if focus {
        let id = egui::Id::unique("nav-search");
        frame.ctx().memory_mut(|m| m.request_focus(id));
    }
    // 实验开关：只在离屏出图时覆盖 TessellationOptions，用来挑出
    // 真正影响 RectShape 圆角/圆点质量的官方参数。
    let feather = rest.iter().find_map(|a| a.strip_prefix("feather="));
    let no_discs = rest.iter().any(|a| a == "nodiscs");
    let no_round = rest.iter().any(|a| a == "noroundrect");
    if feather.is_some() || no_discs || no_round {
        frame.ctx().tessellation_options_mut(|opt| {
            if let Some(v) = feather.and_then(|x| x.parse::<f32>().ok()) {
                opt.feathering_size_in_pixels = v;
            }
            if no_discs {
                opt.prerasterized_discs = false;
            }
            if no_round {
                opt.round_rects_to_pixels = false;
            }
        });
    }

    // `NOOKSILL_SHOT_FROST=1`：按毛玻璃模式出图，并把面板合成到一张
    // 模拟壁纸上（左亮右暗）。毛玻璃的可读性只有压在"不知道多亮"的底上
    // 才看得出问题，纯色底会把它掩盖掉。
    let frost = std::env::var("NOOKSILL_SHOT_FROST").is_ok();

    // 三帧出图：字体图集(0s) → 动画推进(0.4s) → 稳定画面(10s)。
    //
    // 时间必须往前走：egui 的浮层（如说明气泡）有淡入 + 滑入动画，靠
    // `RawInput::time` 推进 —— 时间不动的话动画永远停在第一帧，气泡整个
    // 层只有 ~8% 不透明度、位置也停在半路，截出来是"半透明脏块"。
    for t in [0.0_f64, 0.4] {
        let _ = frame.render_rgba(&gpu, raw_input((w, h), ppp, t), |ui| {
            shell_with_frost(ui, &mut state, &mut cfg, &mut out, frost)
        })?;
    }
    // 第三帧才是"字体已就绪、动画已落定"的稳定画面。
    let (pixels, _) = frame.render_rgba(&gpu, raw_input((w, h), ppp, 10.0), |ui| {
        shell_with_frost(ui, &mut state, &mut cfg, &mut out, frost)
    })?;

    // 毛玻璃模式：底换成"模拟壁纸"—— 左亮右暗的斜向渐变。真实桌面可能是
    // 纯白照片，也可能是一片深色；同一个面板要在两端都读得清才算过关。
    let wallpaper = |x: u32, y: u32| -> [u8; 3] {
        let fx = x as f32 / w.max(1) as f32;
        let fy = y as f32 / h.max(1) as f32;
        let t = (fx * 0.75 + fy * 0.25).clamp(0.0, 1.0);
        let lerp = |a: f32, b: f32| (a + (b - a) * t).round() as u8;
        if dark {
            [lerp(236.0, 26.0), lerp(232.0, 32.0), lerp(214.0, 46.0)]
        } else {
            [lerp(250.0, 22.0), lerp(238.0, 30.0), lerp(210.0, 44.0)]
        }
    };
    let base = if dark { [24, 25, 30] } else { [246, 247, 250] };
    bmp::write_bmp(&out_path, w, h, &pixels, |x, y| {
        if frost {
            wallpaper(x, y)
        } else {
            base
        }
    })?;
    println!(
        "已写出 {out_path}（{}×{}，{:?}，{}，MSAA {}x）",
        w,
        h,
        page,
        if dark { "深色" } else { "浅色" },
        frame.samples()
    );
    Ok(())
}

fn titles() -> Vec<&'static str> {
    Page::ALL.iter().map(|p| p.title()).collect()
}

/// 单帧输入。与 `bin/dock.rs` 里的 `raw_input` 同口径。
fn raw_input(size: (u32, u32), ppp: f32, time: f64) -> egui::RawInput {
    let (w, h) = size;
    let mut raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(w as f32 / ppp, h as f32 / ppp),
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
