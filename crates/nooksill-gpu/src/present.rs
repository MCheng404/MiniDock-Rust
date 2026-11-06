//! 分层窗口呈现：DIB + `UpdateLayeredWindow`。
//!
//! 这是 M0 之后 Dock 的**唯一**呈现路径。它把"GPU 渲染"和"逐像素透明"
//! 两件事拆开：GPU 负责画，`UpdateLayeredWindow` 负责把预乘位图交给 DWM。
//!
//! 几个必须踩准的点：
//! 1. 窗口必须带 `WS_EX_LAYERED`，且不能用 `SetLayeredWindowAttributes`
//!    （与 `UpdateLayeredWindow` 互斥）。
//! 2. DIB 是 **BGRA + 预乘 alpha**；egui 的混合结果已经是预乘，只需换通道序。
//! 3. 颜色源必须用 **sRGB 纹理**（`Rgba8UnormSrgb`）。反过来试过
//!    `Rgba8Unorm`：egui 在非 sRGB 目标上自己做一次 gamma 编码，24 灰变 81 灰，
//!    整块面板发白。硬件做编码、回读即屏幕值，这条链路才对。

use std::ffi::c_void;

use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, DIB_RGB_COLORS,
    HBITMAP, HDC, HGDIOBJ,
};
use windows::Win32::UI::WindowsAndMessaging::{
    UpdateLayeredWindow, UpdateLayeredWindowIndirect, ULW_ALPHA, UPDATELAYEREDWINDOWINFO,
};

use crate::gpu::GpuError;

/// 一块可以直接喂给分层窗口的 DIB。
/// 倒影参数。坐标与尺寸都是**物理像素**，相对窗口左上角。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reflection {
    /// 镜像源区顶行（栏体上沿）。
    pub src_top: i32,
    /// 镜像源区高度（栏高）。
    pub src_h: i32,
    /// 栏体下沿与倒影之间的空隙。
    pub gap: i32,
    /// 倒影区总高（决定衰减曲线的长度）。
    pub height: i32,
    /// 起始不透明度，0..1。
    pub opacity: f32,
}

/// 倒影的核心：把 `src_top` 起的 `src_h` 行上下翻转后写到其下 `gap` 行处，
/// 纵向线性衰减。
///
/// 抽成自由函数是为了能单测：`LayeredPresenter` 要建 GPU 设备，测不起。
/// `dst` 是预乘 BGRA 的整帧缓冲，`w` 是宽度（像素）。
pub fn mirror_rows(dst: &mut [u8], w: usize, h: i32, spec: Reflection) {
    let Reflection {
        src_top,
        src_h,
        gap,
        height,
        opacity,
    } = spec;
    let dst_start = src_top + src_h + gap;
    let rows = height.min(h - dst_start);
    if src_h <= 0 || height <= 0 || rows <= 0 || dst_start < 0 || src_top < 0 {
        return;
    }
    let opacity = opacity.clamp(0.0, 1.0);
    if opacity <= 0.0 {
        return;
    }

    for y in 0..rows {
        // 目标第 0 行取源区**最下面**一行，往上依次回溯 —— 镜像。
        let src_y = src_top + src_h - 1 - y;
        if src_y < 0 {
            break;
        }
        let fade = opacity * (1.0 - y as f32 / height as f32);
        let src_row = (src_y as usize) * w;
        let dst_row = ((dst_start + y) as usize) * w;
        for x in 0..w {
            let si = (src_row + x) * 4;
            let di = (dst_row + x) * 4;
            for c in 0..4 {
                let v = f32::from(dst[si + c]) * fade;
                dst[di + c] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

pub struct LayeredPresenter {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    width: i32,
    height: i32,
}

impl std::fmt::Debug for LayeredPresenter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LayeredPresenter")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl LayeredPresenter {
    pub fn new(width: i32, height: i32) -> Result<Self, GpuError> {
        if width <= 0 || height <= 0 {
            return Err(GpuError::Win32("非法 DIB 尺寸"));
        }
        let screen = unsafe { GetDC(None) };
        if screen.0.is_null() {
            return Err(GpuError::Win32("GetDC(NULL)"));
        }
        let dc = unsafe { CreateCompatibleDC(Some(screen)) };
        unsafe {
            let _ = ReleaseDC(None, screen);
        };
        if dc.0.is_null() {
            return Err(GpuError::Win32("CreateCompatibleDC"));
        }

        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // 自上而下
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0, // BI_RGB
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let bmp = match unsafe {
            CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
        } {
            Ok(h) => h,
            Err(_) => {
                unsafe {
                    let _ = DeleteDC(dc);
                };
                return Err(GpuError::Win32("CreateDIBSection"));
            }
        };
        let old = unsafe { SelectObject(dc, HGDIOBJ(bmp.0)) };

        Ok(Self {
            dc,
            bmp,
            old,
            bits: bits as *mut u8,
            width,
            height,
        })
    }

    /// Write padded RGBA rows straight into the DIB, swapping R/B.
    /// The present path never allocates a tightly packed intermediate buffer.
    pub fn upload_rgba_strided(&mut self, src: &[u8], src_stride: usize) {
        let row = self.width as usize * 4;
        let rows = self.height as usize;
        debug_assert!(src_stride >= row);
        debug_assert!(src.len() >= src_stride * rows.saturating_sub(1) + row);

        // SAFETY: `bits` owns rows*row bytes; source and DIB rows are 4-byte
        // aligned (wgpu readback is 256-byte aligned per row).
        let dst = unsafe { std::slice::from_raw_parts_mut(self.bits, row * rows) };
        swap_rb_strided(dst, src, row, rows, src_stride);
    }

    /// 调试用：读 DIB 里的一个像素（BGRA）。
    #[must_use]
    pub fn debug_pixel(&self, x: i32, y: i32) -> [u8; 4] {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return [0; 4];
        }
        let idx = ((y as usize) * (self.width as usize) + x as usize) * 4;
        let slice = unsafe { std::slice::from_raw_parts(self.bits.add(idx), 4) };
        [slice[0], slice[1], slice[2], slice[3]]
    }

    /// 把 `src_top` 起的 `src_h` 行**上下翻转**后画到其下方 `gap` 行处，
    /// 纵向按 `opacity` 线性衰减 —— 倒影就是这么来的。
    ///
    /// DIB 是预乘 BGRA：四个通道一起乘衰减系数等于"乘 alpha"，颜色不会偏。
    /// 源区与目标区必然不重叠（目标从栏体下沿开始），所以可以原地读写。
    pub fn mirror_below(&mut self, spec: Reflection) {
        let w = self.width as usize;
        let h = self.height;
        if w == 0 || h <= 0 {
            return;
        }
        // SAFETY: `bits` 覆盖 width*height 个 BGRA 像素；`mirror_rows` 的所有
        // 下标都在这个范围内（越界行直接 break）。
        let dst = unsafe { std::slice::from_raw_parts_mut(self.bits, w * h as usize * 4) };
        mirror_rows(dst, w, h, spec);
    }

    /// 交给 DWM。`x/y` 是窗口左上角的屏幕坐标。
    pub fn present(&self, hwnd: HWND, x: i32, y: i32) -> Result<(), GpuError> {
        let dst = POINT { x, y };
        let src = POINT { x: 0, y: 0 };
        let size = SIZE {
            cx: self.width,
            cy: self.height,
        };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            UpdateLayeredWindow(
                hwnd,
                None,
                Some(&dst),
                Some(&size),
                Some(self.dc),
                Some(&src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
        }
        .map_err(|_| GpuError::Win32("UpdateLayeredWindow"))
    }

    /// Same as [`Self::present`], but declares `dirty` (DIB pixels) so DWM can
    /// skip uploading unchanged parts. `prcDirty` is a hint, not a contract.
    pub fn present_dirty(&self, hwnd: HWND, x: i32, y: i32, dirty: RECT) -> Result<(), GpuError> {
        let dst = POINT { x, y };
        let src = POINT { x: 0, y: 0 };
        let size = SIZE {
            cx: self.width,
            cy: self.height,
        };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let info = UPDATELAYEREDWINDOWINFO {
            cbSize: std::mem::size_of::<UPDATELAYEREDWINDOWINFO>() as u32,
            hdcDst: HDC::default(),
            pptDst: &dst,
            psize: &size,
            hdcSrc: self.dc,
            pptSrc: &src,
            crKey: COLORREF(0),
            pblend: &blend,
            dwFlags: ULW_ALPHA,
            prcDirty: &dirty,
        };
        // SAFETY: every pointer in `info` refers to a live local binding.
        if unsafe { UpdateLayeredWindowIndirect(hwnd, &info) }.as_bool() {
            Ok(())
        } else {
            Err(GpuError::Win32("UpdateLayeredWindowIndirect"))
        }
    }
}

/// 0xAARRGGBB → 0xAABBGGRR：保留 A/G，交换 R 与 B。
///
/// 抽成纯函数有两个原因：可以在没有窗口/DC 的环境里单测，
/// 也可以直接比对"逐字节赋值"的旧写法来量化收益。
fn swap_rb(dst: &mut [u32], src: &[u32]) {
    debug_assert_eq!(dst.len(), src.len());
    for (d, s) in dst.iter_mut().zip(src) {
        *d = (*s & 0xFF00_FF00) | ((*s & 0x0000_00FF) << 16) | ((*s & 0x00FF_0000) >> 16);
    }
}

/// Swap R/B from padded source rows into a tightly packed destination.
/// Picks AVX2 when the CPU has it; the scalar path stays as reference.
fn swap_rb_strided(dst: &mut [u8], src: &[u8], row: usize, rows: usize, stride: usize) {
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx2") {
        // SAFETY: AVX2 is present, and the slice lengths are the caller's contract.
        unsafe { swap_rb_strided_avx2(dst, src, row, rows, stride) };
        return;
    }
    swap_rb_strided_scalar(dst, src, row, rows, stride);
}

fn swap_rb_strided_scalar(dst: &mut [u8], src: &[u8], row: usize, rows: usize, stride: usize) {
    let pixels = row / 4;
    for y in 0..rows {
        let s = &src[y * stride..y * stride + row];
        let d = &mut dst[y * row..(y + 1) * row];
        let s32 = unsafe { std::slice::from_raw_parts(s.as_ptr().cast::<u32>(), pixels) };
        let d32 = unsafe { std::slice::from_raw_parts_mut(d.as_mut_ptr().cast::<u32>(), pixels) };
        swap_rb(d32, s32);
    }
}

/// Byte shuffle: [b0,b1,b2,b3] -> [b2,b1,b0,b3], i.e. B and R swapped.
/// The pattern repeats every 16 bytes, so it fits one `vpshufb` per 128-bit lane.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn swap_rb_strided_avx2(dst: &mut [u8], src: &[u8], row: usize, rows: usize, stride: usize) {
    use std::arch::x86_64::{
        __m256i, _mm256_loadu_si256, _mm256_setr_epi8, _mm256_shuffle_epi8, _mm256_storeu_si256,
    };
    let pixels = row / 4;
    let mask = _mm256_setr_epi8(
        2, 1, 0, 3, 6, 5, 4, 7, 10, 9, 8, 11, 14, 13, 12, 15, 2, 1, 0, 3, 6, 5, 4, 7, 10, 9, 8, 11,
        14, 13, 12, 15,
    );
    // Rows narrower than one vector fall back to the scalar path.
    if pixels < 8 {
        swap_rb_strided_scalar(dst, src, row, rows, stride);
        return;
    }
    let mut y = 0;
    while y < rows {
        let s = src[y * stride..y * stride + row].as_ptr();
        let d = dst[y * row..(y + 1) * row].as_mut_ptr();
        let mut i = 0;
        while i + 8 <= pixels {
            let v = unsafe { _mm256_loadu_si256(s.add(i * 4).cast::<__m256i>()) };
            let r = _mm256_shuffle_epi8(v, mask);
            unsafe { _mm256_storeu_si256(d.add(i * 4).cast::<__m256i>(), r) };
            i += 8;
        }
        if i < pixels {
            // Overlapping store of the last full vector: cheaper than a scalar tail.
            let j = pixels - 8;
            let v = unsafe { _mm256_loadu_si256(s.add(j * 4).cast::<__m256i>()) };
            let r = _mm256_shuffle_epi8(v, mask);
            unsafe { _mm256_storeu_si256(d.add(j * 4).cast::<__m256i>(), r) };
        }
        y += 1;
    }
}

impl Drop for LayeredPresenter {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(HGDIOBJ(self.bmp.0));
            let _ = DeleteDC(self.dc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{mirror_rows, swap_rb, swap_rb_strided, swap_rb_strided_scalar, Reflection};

    /// 4 像素宽、6 行：源区 0..2（第 0 行 = 100，第 1 行 = 50），
    /// 倒影从第 3 行开始（gap=1），高 3 行。
    fn spec(src_top: i32, src_h: i32, gap: i32, height: i32, opacity: f32) -> Reflection {
        Reflection {
            src_top,
            src_h,
            gap,
            height,
            opacity,
        }
    }

    fn buffer() -> Vec<u8> {
        let mut v = vec![0u8; 4 * 6 * 4];
        for x in 0..4 {
            for c in 0..4 {
                v[x * 4 + c] = 100;
                v[(4 + x) * 4 + c] = 50;
            }
        }
        v
    }

    #[test]
    fn mirrors_and_fades_downwards() {
        let mut v = buffer();
        mirror_rows(&mut v, 4, 6, spec(0, 2, 1, 3, 1.0));
        let row = |y: usize| v[(y * 4) * 4];
        // 第 3 行（倒影第一行）取源的**最后**一行：50，衰减 1 - 0/3 = 1
        assert_eq!(row(3), 50, "第一行该是源区的底行");
        // 第 4 行取源区上一行（100），衰减 1 - 1/3 ≈ 0.667
        assert_eq!(row(4), 67, "第二行往上回溯并按距离衰减");
        // 第 5 行：源已耗尽（只剩一行是 2 行高的源）→ 继续回溯到负数就停
        assert_eq!(row(5), 0, "源区用完之后不该凭空画东西");
    }

    #[test]
    fn keeps_gap_untouched() {
        let mut v = buffer();
        mirror_rows(&mut v, 4, 6, spec(0, 2, 1, 3, 1.0));
        // 第 2 行是空隙（gap=1），必须原样保留。
        assert_eq!(v[(2 * 4) * 4], 0, "空隙行不该被写");
    }

    #[test]
    fn zero_opacity_is_a_no_op() {
        let mut v = buffer();
        let before = v.clone();
        mirror_rows(&mut v, 4, 6, spec(0, 2, 1, 3, 0.0));
        assert_eq!(v, before);
    }

    #[test]
    fn refuses_to_write_past_the_buffer() {
        let mut v = buffer();
        // 倒影区比缓冲还高：只能写到缓冲区末尾，不能越界。
        mirror_rows(&mut v, 4, 6, spec(0, 2, 1, 100, 1.0));
        assert_eq!(v.len(), 4 * 6 * 4);
    }

    /// SIMD path must be bit-identical to the scalar reference, including
    /// tails, the overlapping last vector and rows narrower than one vector.
    #[test]
    fn simd_path_matches_scalar_for_every_width() {
        let rows = 3usize;
        for px in (1..=40usize).chain([391, 1320]) {
            let row = px * 4;
            let stride = row + 12;
            let src: Vec<u8> = (0..stride * rows).map(|i| (i * 37 % 251) as u8).collect();
            let mut fast = vec![0u8; row * rows];
            let mut slow = vec![0u8; row * rows];
            swap_rb_strided(&mut fast, &src, row, rows, stride);
            swap_rb_strided_scalar(&mut slow, &src, row, rows, stride);
            assert_eq!(fast, slow, "{px}px wide");
        }
    }

    #[test]
    fn swap_rb_strided_skips_row_padding() {
        // Two 1-pixel rows, padded to 8 bytes each.
        let row = 4usize;
        let stride = 8usize;
        let pixels = [0xAABB_CCDDu32, 0, 0x1122_3344u32, 0];
        let src: Vec<u8> = pixels.iter().flat_map(|p| p.to_ne_bytes()).collect();
        let mut dst = vec![0u8; row * 2];
        swap_rb_strided(&mut dst, &src, row, 2, stride);

        let mut expected = vec![0u8; row * 2];
        let packed = [0xAABB_CCDDu32, 0x1122_3344u32];
        let expected_u32 =
            unsafe { std::slice::from_raw_parts_mut(expected.as_mut_ptr().cast::<u32>(), 2) };
        swap_rb(expected_u32, &packed);
        assert_eq!(dst, expected);
    }

    /// Old present path (tight copy + swap) vs direct strided write.
    /// Prints numbers only; the result equality is asserted.
    #[test]
    #[ignore = "throughput benchmark; run with --ignored"]
    fn strided_present_throughput() {
        const W: usize = 1320;
        const H: usize = 1080;
        const REPS: usize = 20;
        let row = W * 4;
        let stride = row.div_ceil(256) * 256;
        let padded: Vec<u8> = (0..H * stride).map(|i| (i % 251) as u8).collect();
        let mut dst_old = vec![0u8; row * H];
        let mut dst_new = vec![0u8; row * H];

        // Old path allocates a tight buffer every frame, exactly like
        // `Offscreen::read_pixels` -> `unpad_rows`.
        let t0 = std::time::Instant::now();
        for _ in 0..REPS {
            let mut tight = Vec::with_capacity(row * H);
            for y in 0..H {
                tight.extend_from_slice(&padded[y * stride..y * stride + row]);
            }
            let s = unsafe { std::slice::from_raw_parts(tight.as_ptr().cast::<u32>(), W * H) };
            let d = unsafe {
                std::slice::from_raw_parts_mut(dst_old.as_mut_ptr().cast::<u32>(), W * H)
            };
            swap_rb(d, s);
        }
        let old = t0.elapsed();

        let t1 = std::time::Instant::now();
        for _ in 0..REPS {
            swap_rb_strided(&mut dst_new, &padded, row, H, stride);
        }
        let new = t1.elapsed();

        println!(
            "old (tight copy + swap): {:.2} ms/frame | strided: {:.2} ms/frame | {:.2}x | equal={}",
            old.as_secs_f64() * 1000.0 / REPS as f64,
            new.as_secs_f64() * 1000.0 / REPS as f64,
            old.as_secs_f64() / new.as_secs_f64(),
            dst_old == dst_new
        );
        assert_eq!(dst_old, dst_new);
    }

    #[test]
    fn swap_rb_matches_byte_wise_reference() {
        // 覆盖边界：全 0、全 1、最大 alpha、只剩单通道。
        let cases: [u32; 6] = [
            0x0000_0000,
            0xFFFF_FFFF,
            0xAABB_CCDD,
            0xFF00_0000,
            0x0000_00FF,
            0x0000_FF00,
        ];
        let mut got = [0u32; 6];
        swap_rb(&mut got, &cases);
        for (i, s) in cases.iter().enumerate() {
            let bytes = s.to_le_bytes(); // [r, g, b, a]
            let expect = u32::from_le_bytes([bytes[2], bytes[1], bytes[0], bytes[3]]);
            assert_eq!(got[i], expect, "case {i:#010X}");
        }
    }

    /// 与旧写法（逐字节四次赋值）比对吞吐。**只打印不断言**：
    /// 时间在不同机器/负载下波动，断言会变成 flaky 测试。
    ///
    /// 默认 `#[ignore]`：它要搬 1320×1080×4×20 ≈ 114 MB，是 benchmark 不是
    /// 正确性测试，放进默认的 `cargo test` 只会让每次跑测试都慢一秒多。
    /// 需要数字时显式跑：`cargo test -p nooksill-gpu -- --ignored --nocapture`。
    #[test]
    #[ignore = "吞吐 benchmark，用 --ignored 显式运行"]
    fn swap_rb_throughput() {
        const W: usize = 1320;
        const H: usize = 1080;
        const REPS: usize = 20;
        let n = W * H;
        let src: Vec<u32> = (0..n as u32)
            .map(|i| i.wrapping_mul(2_654_435_761))
            .collect();
        let mut dst = vec![0u32; n];
        let mut slow = vec![0u32; n];

        let t0 = std::time::Instant::now();
        for _ in 0..REPS {
            swap_rb(&mut dst, &src);
        }
        let fast = t0.elapsed();

        // 旧写法**原样**搬过来（逐字节四次赋值），不要用 `to_le_bytes` 构造替身
        // —— 那测的是另一段代码，会夸大收益。
        let src_bytes = unsafe { std::slice::from_raw_parts(src.as_ptr().cast::<u8>(), n * 4) };
        let slow_bytes =
            unsafe { std::slice::from_raw_parts_mut(slow.as_mut_ptr().cast::<u8>(), n * 4) };
        let t1 = std::time::Instant::now();
        for _ in 0..REPS {
            for i in 0..n {
                let s = i * 4;
                slow_bytes[s] = src_bytes[s + 2];
                slow_bytes[s + 1] = src_bytes[s + 1];
                slow_bytes[s + 2] = src_bytes[s];
                slow_bytes[s + 3] = src_bytes[s + 3];
            }
        }
        let old = t1.elapsed();

        let mb = (n * 4 * REPS) as f64 / (1024.0 * 1024.0);
        println!(
            "u32 交换: {:.2} ms/帧 ({:.0} MB/s) | 旧逐字节: {:.2} ms/帧 ({:.0} MB/s) | 提速 {:.1}x | 结果一致={}",
            fast.as_secs_f64() * 1000.0 / REPS as f64,
            mb / fast.as_secs_f64(),
            old.as_secs_f64() * 1000.0 / REPS as f64,
            mb / old.as_secs_f64(),
            old.as_secs_f64() / fast.as_secs_f64(),
            dst == slow,
        );
        assert_eq!(dst, slow, "两种写法必须逐像素一致");
    }
}
