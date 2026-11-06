//! 示例共用的 BMP 写出。
//!
//! 用 `#[path = "common/bmp.rs"] mod bmp;` 引入 —— 放在 `examples/common/`
//! 下的文件不会被 cargo 当成独立示例（只认 `examples/*.rs` 与
//! `examples/*/main.rs`）。
//!
//! 三份截图工具原先各抄了一遍这个函数（连注释都分叉了）：一处改了行对齐，
//! 另一处不会跟着改。合并之后只有一个实现。

use std::io::Write;

/// 24 位 BMP，把**预乘** RGBA 合成到背景上。
///
/// 背景按像素给：`(x, y)` 返回该点底色。纯色传闭包常量即可，毛玻璃截图要
/// 传一张模拟壁纸 —— 这正是当初三份实现分叉的原因，与其各写一版，不如把
/// 底色变成参数。
pub fn write_bmp(
    path: &str,
    width: u32,
    height: u32,
    rgba: &[u8],
    bg: impl Fn(u32, u32) -> [u8; 3],
) -> std::io::Result<()> {
    let row_stride = ((width as usize) * 3).div_ceil(4) * 4;
    let data_len = row_stride * height as usize;
    let mut buf = Vec::with_capacity(54 + data_len);

    // BITMAPFILEHEADER
    buf.extend_from_slice(b"BM");
    buf.extend_from_slice(&(54 + data_len as u32).to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&54u32.to_le_bytes());
    // BITMAPINFOHEADER
    buf.extend_from_slice(&40u32.to_le_bytes());
    buf.extend_from_slice(&(width as i32).to_le_bytes());
    buf.extend_from_slice(&(height as i32).to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&24u16.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes()); // 无压缩
    buf.extend_from_slice(&(data_len as u32).to_le_bytes());
    buf.extend_from_slice(&2835i32.to_le_bytes()); // 72 DPI
    buf.extend_from_slice(&2835i32.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());

    // BMP 是自下而上存的。
    for y in (0..height).rev() {
        let mut written = 0;
        for x in 0..width {
            let i = ((y * width + x) * 4) as usize;
            let (r, g, b, a) = (rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]);
            let base = bg(x, y);
            // 预乘 alpha：直接加上背景的 (1-a) 部分。
            let inv = 255 - a as u32;
            let blend =
                |c: u8, back: u8| -> u8 { (c as u32 + back as u32 * inv / 255).min(255) as u8 };
            buf.push(blend(b, base[2]));
            buf.push(blend(g, base[1]));
            buf.push(blend(r, base[0]));
            written += 3;
        }
        // 行尾补齐到 4 字节边界（BMP 的 stride 规则）。
        buf.extend(std::iter::repeat_n(0u8, row_stride - written));
    }

    let mut f = std::fs::File::create(path)?;
    f.write_all(&buf)
}
