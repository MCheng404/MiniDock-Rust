//! 把两个新面板（分组 / 速记）离屏渲染成 BMP。
//!
//! ```text
//! cargo run -p nooksill-gpu --example panels_shot -- out_dir [dark]
//! ```
//!
//! 走的是与真实窗口**同一份**绘制函数（`group_win::paint_preview` /
//! `note_win::paint_preview`），只是不建 Win32 窗口、不 `UpdateLayeredWindow`。
//! 这两个面板此前只有编译检查与几何单测，没有任何像素级证据；
//! 有了这个工具，加字段、改配色之后能直接出图核对。
//!
//! 分组面板的图标传空表，因此画的是"首字占位"退化路径 —— 这是有意的，
//! 离屏拿不到 shell 图标，正好顺带覆盖这条分支。

#[path = "common/bmp.rs"]
mod bmp;

use nooksill_core::item::ItemKind;
use nooksill_core::note::NoteBook;
use nooksill_gpu::frame::FrameRenderer;
use nooksill_gpu::gpu::Gpu;
use nooksill_gpu::group_win::{self, GroupKid};
use nooksill_gpu::note_win;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let dir = args.next().unwrap_or_else(|| "scratch".to_owned());
    let dark = args.next().as_deref() != Some("light");
    let ppp = 1.25_f32;
    let gpu = Gpu::new("")?;

    // ① 分组面板：5 个子项（跨两行），第 2 项悬停 → 显示 × 与 ← →。
    let kids: Vec<GroupKid> = ["记事本", "VS Code", "GitHub", "终端", "画图"]
        .iter()
        .enumerate()
        .map(|(i, name)| GroupKid {
            name: (*name).to_owned(),
            path: format!(r"C:\app{i}.exe"),
            kind: ItemKind::File,
            running: i == 1,
        })
        .collect();
    let (gw, gh) = group_win::logical_size(kids.len());
    shot(
        &gpu,
        &format!("{dir}/panel_group.bmp"),
        gw,
        gh,
        ppp,
        dark,
        |ui| group_win::paint_preview(ui, dark, "开发工具", &kids, Some(1)),
    )?;

    // ② 速记面板：7 条（跨两页），第 1 条悬停。
    let mut book = NoteBook::default();
    for (i, (t, b)) in [
        ("购物清单", "牛奶\n面包\n鸡蛋"),
        ("周会要点", "先把门禁跑绿，再谈渲染"),
        ("读书笔记", "第七章：生命周期与型变"),
        ("待确认", "托盘菜单在 150% DPI 下会不会跑出屏幕"),
        ("地址", "北京市海淀区"),
        ("临时", "随手记的"),
        ("第七条", "用来验证分页"),
    ]
    .iter()
    .enumerate()
    {
        let _ = i;
        book.add(t, b).expect("测试数据量没到上限");
    }
    let (nw, nh) = note_win::SIZE;
    shot(
        &gpu,
        &format!("{dir}/panel_notes.bmp"),
        nw,
        nh,
        ppp,
        dark,
        |ui| note_win::paint_preview(ui, dark, &book, 0, Some(0)),
    )?;

    // ③ 空态：两条路径都要能画，不能因为列表为空就 panic。
    shot(
        &gpu,
        &format!("{dir}/panel_notes_empty.bmp"),
        nw,
        nh,
        ppp,
        dark,
        |ui| note_win::paint_preview(ui, dark, &NoteBook::default(), 0, None),
    )?;
    Ok(())
}

fn shot(
    gpu: &Gpu,
    path: &str,
    lw: f32,
    lh: f32,
    ppp: f32,
    dark: bool,
    draw: impl FnMut(&mut egui::Ui),
) -> Result<(), Box<dyn std::error::Error>> {
    let w = (lw * ppp).ceil() as u32;
    let h = (lh * ppp).ceil() as u32;
    let mut frame = FrameRenderer::new(gpu, w, h, ppp)?;
    let (rgba, _) = frame.render_rgba(gpu, raw_input(w, h, ppp), draw)?;
    let bg = if dark { [18, 19, 23] } else { [238, 239, 243] };
    bmp::write_bmp(path, w, h, &rgba, move |_, _| bg)?;
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
