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

use windows::Win32::Foundation::{COLORREF, HWND, POINT, SIZE};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, DIB_RGB_COLORS,
    HBITMAP, HDC, HGDIOBJ,
};
use windows::Win32::UI::WindowsAndMessaging::{UpdateLayeredWindow, ULW_ALPHA};

use crate::gpu::GpuError;

/// 一块可以直接喂给分层窗口的 DIB。
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

    /// 把 RGBA（预乘）写成 BGRA 到 DIB。
    ///
    /// 每帧一次、1320×1080 下是 140 万像素，按 u32 搬运而不是四次字节赋值。
    ///
    /// 实测（release，同机）：u32 交换 0.80 ms/帧，原逐字节写法 1.16 ms/帧，
    /// **约 1.45x**。只是省 0.36 ms，不是什么数量级 —— 真要紧的开销在
    /// GPU 回读和 `UpdateLayeredWindow`，这里属于顺手能做对的地方。
    pub fn upload_rgba(&mut self, rgba: &[u8]) {
        let pixels = (self.width as usize) * (self.height as usize);
        debug_assert_eq!(rgba.len(), pixels * 4);

        // SAFETY: `bits` 由 `CreateDIBSection` 分配，长度恰为 pixels*4；
        // 调用方保证 `rgba` 同长（上游固定尺寸 + debug 断言）。
        // 两个切片都对齐到 u32：DIB 按 4 字节对齐分配，rgba 来自 wgpu
        // 回读缓冲（本身就是 4 字节对齐的像素数组）。
        let src = unsafe { std::slice::from_raw_parts(rgba.as_ptr().cast::<u32>(), pixels) };
        let dst = unsafe { std::slice::from_raw_parts_mut(self.bits.cast::<u32>(), pixels) };
        swap_rb(dst, src);
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
}

/// 0xAARRGGBB → 0xAABBGGRR：保留 A/G，交换 R 与 B。
///
/// 抽成纯函数有两个原因：可以在没有窗口/DC 的环境里单测，
/// 也可以直接对拍"逐字节赋值"的旧写法来量化收益。
fn swap_rb(dst: &mut [u32], src: &[u32]) {
    debug_assert_eq!(dst.len(), src.len());
    for (d, s) in dst.iter_mut().zip(src) {
        *d = (*s & 0xFF00_FF00) | ((*s & 0x0000_00FF) << 16) | ((*s & 0x00FF_0000) >> 16);
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
    use super::swap_rb;

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

    /// 与旧写法（逐字节四次赋值）对拍吞吐。**只打印不断言**：
    /// 时间在不同机器/负载下波动，断言会变成 flaky 测试。
    #[test]
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
