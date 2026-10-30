//! 图标提取的诊断工具：把某个路径的图标 dump 成原始 RGBA，便于外部核对。
//!
//! 用法：
//!
//! ```text
//! cargo run -p minidock-platform --example icon_dump -- <路径> [输出前缀]
//! ```
//!
//! 输出两个文件：`<前缀>.rgba`（行优先 RGBA）与 `<前缀>.txt`（宽高与统计）。
//! 不生成 PNG 是刻意的：platform 层不引图像库，换算交给外部工具。

use minidock_core::icon::{fit_icon, FitMode};
use minidock_platform::icon::{resource_icon, shell_icon, IconBitmap};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("缺少路径参数")?;
    let prefix = args.next().unwrap_or_else(|| "icon".to_owned());

    println!("路径: {path}");

    let shell = shell_icon(&path);
    report("shell", shell.as_ref());

    let res = resource_icon(&path, 0);
    report("resource[0]", res.as_ref());

    // 优先 dump shell 的（Dock 正常路径），没有就用资源里的。
    let Some(bmp) = shell.or(res) else {
        println!("两种方式都没取到图标");
        return Ok(());
    };
    dump(&bmp, &prefix)?;
    println!("已写出 {prefix}.rgba / {prefix}.txt");

    // 适配结果：这一项直接决定 Dock 上画出来是"整块底板"还是"中间图形"。
    for (name, mode) in [
        ("自动", FitMode::Auto),
        ("完整", FitMode::Full),
        ("裁切", FitMode::Square),
    ] {
        // 遮罩要改像素，先克隆一份。
        let mut work = bmp.clone();
        let fit = fit_icon(&work.rgba, work.width, work.height, mode);
        if let Some(r) = fit.rounded {
            minidock_core::icon::apply_round_mask(&mut work.rgba, work.width, fit.bbox, r);
        }
        let visible = work
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] != 0)
            .count();
        println!(
            "  {name:<4} bbox=({},{})-({},{}) {}x{}  圆角={:?}  fill={:.2}  遮罩后非透明 {}",
            fit.bbox.x0,
            fit.bbox.y0,
            fit.bbox.x1,
            fit.bbox.y1,
            fit.bbox.width(),
            fit.bbox.height(),
            fit.rounded,
            fit.fill,
            visible
        );
    }
    Ok(())
}

fn report(how: &str, bmp: Option<&IconBitmap>) {
    match bmp {
        Some(b) => {
            let visible = b
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[3] != 0)
                .count();
            let total = b.rgba.len() / 4;
            println!(
                "  {how:<12} {}x{}  非透明 {visible}/{total}（{:.1}%）",
                b.width,
                b.height,
                visible as f32 * 100.0 / total as f32
            );
        }
        None => println!("  {how:<12} 未取到"),
    }
}

fn dump(b: &IconBitmap, prefix: &str) -> std::io::Result<()> {
    std::fs::write(format!("{prefix}.rgba"), &b.rgba)?;
    std::fs::write(
        format!("{prefix}.txt"),
        format!("{} {}\n", b.width, b.height),
    )?;
    Ok(())
}
