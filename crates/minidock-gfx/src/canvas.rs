//! 分层窗口画布：DIB section + GDI+ 视图 + `UpdateLayeredWindow` 呈现。
//!
//! 一个 `Canvas` 对应一次 surface 分配；尺寸变化就整体重建（与原版
//! `MakeSurface` / `FreeSurface` 的策略一致）。**不允许**在重建之后继续
//! 使用旧的 `Graphics` —— 这一点由 [`Canvas::graphics`] 的借用生命周期
//! 保证，而原版正是栽在这里（issue.md 的 0xC0000005）。

use crate::objects::{Bitmap, Graphics};
use crate::{GfxError, Result};
use std::ffi::c_void;
use windows::Win32::Foundation::{COLORREF, HWND, POINT, SIZE};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, DIB_RGB_COLORS,
    HBITMAP, HDC, HGDIOBJ,
};
use windows::Win32::UI::WindowsAndMessaging::{UpdateLayeredWindow, ULW_ALPHA};

/// 一块可绘制的 32 位预乘 ARGB 表面，可直接作为分层窗口的像素源。
pub struct Canvas {
    dc: HDC,
    bmp: HBITMAP,
    /// `SelectObject` 之前 DC 里的原位图，Drop 时要还回去。
    old: HGDIOBJ,
    bits: *mut u8,
    bitmap: Bitmap,
    width: i32,
    height: i32,
}

impl std::fmt::Debug for Canvas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Canvas")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl Canvas {
    pub fn new(width: i32, height: i32) -> Result<Self> {
        if width <= 0 || height <= 0 {
            return Err(GfxError::BadSize { width, height });
        }

        let screen = unsafe { GetDC(None) };
        if screen.0.is_null() {
            return Err(GfxError::Win32("GetDC(NULL)"));
        }
        let dc = unsafe { CreateCompatibleDC(Some(screen)) };
        unsafe { ReleaseDC(None, screen) };
        if dc.0.is_null() {
            return Err(GfxError::Win32("CreateCompatibleDC"));
        }

        // 负的 biHeight = 自上而下的行序，与 GDI+ / UpdateLayeredWindow 一致。
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
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
                return Err(GfxError::Win32("CreateDIBSection"));
            }
        };
        let old = unsafe { SelectObject(dc, HGDIOBJ(bmp.0)) };

        let bitmap = match Bitmap::from_scan0(width, height, width * 4, bits as *mut u8) {
            Ok(b) => b,
            Err(e) => {
                unsafe {
                    SelectObject(dc, old);
                    let _ = DeleteObject(HGDIOBJ(bmp.0));
                    let _ = DeleteDC(dc);
                }
                return Err(e);
            }
        };

        Ok(Self {
            dc,
            bmp,
            old,
            bits: bits as *mut u8,
            bitmap,
            width,
            height,
        })
    }

    #[must_use]
    pub const fn width(&self) -> i32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> i32 {
        self.height
    }

    /// 可用于绘制的像素缓冲（`width * height * 4` 字节，预乘 ARGB）。
    ///
    /// 磨砂背景需要把截屏结果写进这里，所以必须暴露裸缓冲；调用方要保证
    /// 不在绘制中途持有它（借用了 `&mut self`，与 [`Self::graphics`] 互斥）。
    pub fn pixels_mut(&mut self) -> &mut [u8] {
        let len = (self.width as usize) * (self.height as usize) * 4;
        unsafe { std::slice::from_raw_parts_mut(self.bits, len) }
    }

    /// 取得本帧的绘制上下文。借用期间不能重建画布。
    pub fn graphics(&self) -> Result<Graphics<'_>> {
        self.bitmap.graphics()
    }

    /// 把画布内容推到分层窗口的 `(x, y)`。
    ///
    /// 必须是 alpha 混合（`ULW_ALPHA` + `AC_SRC_ALPHA`），否则圆角外的
    /// 透明区域会变成黑色。
    pub fn present(&self, hwnd: HWND, x: i32, y: i32) -> Result<()> {
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
            .map_err(|_| GfxError::Win32("UpdateLayeredWindow"))
        }
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        // 顺序很重要：先解绑位图，再删位图，最后删 DC。
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(HGDIOBJ(self.bmp.0));
            let _ = DeleteDC(self.dc);
        }
    }
}
