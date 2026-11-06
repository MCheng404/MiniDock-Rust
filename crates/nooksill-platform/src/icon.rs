//! HICON to RGBA, in three fallbacks:
//! 1. `GetIconInfo` + `GetDIBits(hbmColor)` for 32-bit icons with alpha.
//! 2. Rebuild alpha from `hbmMask` for legacy 24-bit icons.
//! 3. `DrawIconEx` into a 32-bit DIB (see `draw_into_dib`).
//!
//! GDI+ is skipped: `GdipCreateBitmapFromHICON` leaves a black background.
//! JUMBO (256) is tried before EXTRALARGE (48) for 150% DPI.

use windows::core::PCWSTR;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush, GetDC, GetDIBits,
    GetObjectW, ReleaseDC, SelectObject, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS,
    HBITMAP, HGDIOBJ,
};
use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
use windows::Win32::UI::Shell::{
    SHGetFileInfoW, SHGetImageList, SHFILEINFOW, SHGFI_DISPLAYNAME, SHGFI_SYSICONINDEX,
    SHIL_EXTRALARGE, SHIL_JUMBO,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, DrawIconEx, GetIconInfo, DI_NORMAL, HICON, ICONINFO,
};

/// 一块直通 alpha 的位图，`rgba` 为行优先、每像素 4 字节。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconBitmap {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl IconBitmap {
    /// 逐像素读 alpha 用的便捷方法（诊断与测试）。
    #[must_use]
    pub fn alpha_at(&self, x: u32, y: u32) -> u8 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        self.rgba[((y * self.width + x) * 4 + 3) as usize]
    }

    /// 是否有任何非透明像素。整块全透明的图等于没取到图标。
    #[must_use]
    pub fn has_visible_pixels(&self) -> bool {
        self.rgba.as_chunks::<4>().0.iter().any(|p| p[3] != 0)
    }
}

/// 文件在资源管理器里显示的名字（`SHGFI_DISPLAYNAME`）。
///
/// 用来给"未固定的运行中应用"起名：`chrome.exe` 的显示名是
/// "Google Chrome"，`code.exe` 是 "Visual Studio Code" —— 比去掉扩展名的
/// 文件名好读得多，而且这份描述本来就随程序一起装好了，不用解析版本资源。
///
/// 取不到就返回 `None`，调用方退回文件名。
#[must_use]
pub fn display_name(path: &str) -> Option<String> {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut sfi = SHFILEINFOW::default();
    // SAFETY: 输出结构是本栈帧上的 `SHFILEINFOW`，尺寸取自它自己。
    let ok = unsafe {
        SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            Default::default(),
            Some(&raw mut sfi),
            u32::try_from(std::mem::size_of::<SHFILEINFOW>()).ok()?,
            SHGFI_DISPLAYNAME,
        )
    };
    if ok == 0 {
        return None;
    }
    let len = sfi
        .szDisplayName
        .iter()
        .position(|c| *c == 0)
        .unwrap_or(sfi.szDisplayName.len());
    if len == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&sfi.szDisplayName[..len]))
}

/// 取 shell 图标：先 JUMBO（256 px），失败再 EXTRALARGE（48 px）。
///
/// `path` 可以是 exe、文件夹、`.lnk`、网址 —— 只要是 Explorer 能给图标的都行。
#[must_use]
pub fn shell_icon(path: &str) -> Option<IconBitmap> {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();

    let trace = std::env::var_os("NOOKSILL_ICON_TRACE").is_some();
    let mut sfi = SHFILEINFOW::default();
    let ok = unsafe {
        SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            Default::default(),
            Some(&raw mut sfi),
            u32::try_from(std::mem::size_of::<SHFILEINFOW>()).ok()?,
            SHGFI_SYSICONINDEX,
        )
    };
    if ok == 0 {
        if trace {
            eprintln!("  [icon] SHGetFileInfoW 失败: {path}");
        }
        return None;
    }
    if trace {
        eprintln!("  [icon] iIcon={} attrs={:#x}", sfi.iIcon, sfi.dwAttributes);
    }

    // 两档都试：JUMBO 在部分老系统上会失败，EXTRALARGE 几乎总在。
    for list in [SHIL_JUMBO, SHIL_EXTRALARGE] {
        let il: windows::core::Result<IImageList> =
            unsafe { SHGetImageList(i32::try_from(list).ok()?) };
        let il = match il {
            Ok(v) => v,
            Err(e) => {
                if trace {
                    eprintln!("  [icon] SHGetImageList({list}) 失败: {e}");
                }
                continue;
            }
        };
        let hicon = match unsafe { il.GetIcon(sfi.iIcon, ILD_TRANSPARENT.0) } {
            Ok(v) => v,
            Err(e) => {
                if trace {
                    eprintln!("  [icon] GetIcon({}, list={list}) 失败: {e}", sfi.iIcon);
                }
                continue;
            }
        };
        let bmp = hicon_to_bitmap(hicon);
        let _ = unsafe { DestroyIcon(hicon) };
        match bmp {
            Some(b) if b.has_visible_pixels() => {
                if trace {
                    eprintln!("  [icon] 取自 list={list}: {}x{}", b.width, b.height);
                }
                return Some(b);
            }
            Some(b) => {
                if trace {
                    eprintln!(
                        "  [icon] list={list} 取到 {}x{} 但全透明",
                        b.width, b.height
                    );
                }
            }
            None => {
                if trace {
                    eprintln!("  [icon] list={list} 的 HICON 转位图失败");
                }
            }
        }
    }
    None
}

/// 从可执行文件 / DLL / `.ico` 的资源里取第 `index` 个图标。
///
/// 快捷方式里存的 `iconLoc` + `iconIdx` 常指向 exe 内的资源，shell 的图像
/// 列表覆盖不到这类位置。
#[must_use]
pub fn resource_icon(file: &str, index: i32) -> Option<IconBitmap> {
    use windows::Win32::UI::WindowsAndMessaging::PrivateExtractIconsW;

    let wide: Vec<u16> = file.encode_utf16().chain(std::iter::once(0)).collect();
    let mut buf = [0u16; 260];
    let n = wide.len().min(buf.len());
    buf[..n].copy_from_slice(&wide[..n]);
    if n == buf.len() {
        // 路径被截断，宁可不取也不猜。
        return None;
    }

    // 从大到小试：256 是 jumbo 的等价尺寸，取不到再退。
    for size in [256, 128, 64, 48] {
        let mut hicon = HICON::default();
        let got = unsafe {
            PrivateExtractIconsW(
                &buf,
                index,
                size,
                size,
                Some(std::slice::from_mut(&mut hicon)),
                None,
                0,
            )
        };
        if got != 1 || hicon.is_invalid() {
            continue;
        }
        let bmp = hicon_to_bitmap(hicon);
        let _ = unsafe { DestroyIcon(hicon) };
        // 尺寸档位已经由 `PrivateExtractIconsW` 指定，拿不到就是文件的
        // 第 `index` 个图标不存在，换下一档没用 —— 但多试几档能兜住
        // 只提供 48 px 的老 exe。
        if let Some(b) = bmp {
            if b.has_visible_pixels() {
                return Some(b);
            }
        }
    }
    None
}

/// HICON → RGBA。三层兜底顺序见模块文档。
#[must_use]
pub fn hicon_to_bitmap(hicon: HICON) -> Option<IconBitmap> {
    if hicon.is_invalid() {
        return None;
    }
    let mut ii = ICONINFO::default();
    if unsafe { GetIconInfo(hicon, &raw mut ii) }.is_err() {
        return None;
    }

    let color = if !ii.hbmColor.is_invalid() {
        from_color_bitmap(ii.hbmColor, ii.hbmMask)
    } else {
        None
    };

    // 释放 GetIconInfo 复制出来的两个位图。
    unsafe {
        if !ii.hbmColor.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(ii.hbmColor.0));
        }
        if !ii.hbmMask.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(ii.hbmMask.0));
        }
    }

    color.or_else(|| draw_into_dib(hicon))
}

/// 第一层：彩色位图 + （必要时）掩码造 alpha。
fn from_color_bitmap(hbm_color: HBITMAP, hbm_mask: HBITMAP) -> Option<IconBitmap> {
    let mut bm = BITMAP::default();
    let got = unsafe {
        GetObjectW(
            HGDIOBJ(hbm_color.0),
            i32::try_from(std::mem::size_of::<BITMAP>()).ok()?,
            Some((&raw mut bm).cast()),
        )
    };
    if got == 0 || bm.bmWidth <= 0 || bm.bmHeight <= 0 {
        return None;
    }
    let w = u32::try_from(bm.bmWidth).ok()?;
    let h = u32::try_from(bm.bmHeight).ok()?;
    let pixels = usize::try_from(w).ok()? * usize::try_from(h).ok()?;

    let mut bgra = read_dib(hbm_color, w, h)?;

    // alpha 全 0 = 老式 24 位图标（或掩码式 alpha）。用掩码补：
    // 掩码里 0 表示"要画"，非 0 表示透明。
    if !bgra.as_chunks::<4>().0.iter().any(|p| p[3] != 0) && !hbm_mask.is_invalid() {
        let mask = read_dib(hbm_mask, w, h)?;
        for i in 0..pixels {
            bgra[i * 4 + 3] = if mask[i * 4] == 0 { 255 } else { 0 };
        }
    }

    let rgba = bgra_to_rgba(&bgra);
    Some(IconBitmap {
        width: w,
        height: h,
        rgba,
    })
}

/// BGRA to RGBA; fully transparent pixels are zeroed.
fn bgra_to_rgba(bgra: &[u8]) -> Vec<u8> {
    let mut rgba = vec![0u8; bgra.len()];
    // Ignore the trailing remainder (callers always pass a multiple of 4).
    for (src, dst) in bgra
        .as_chunks::<4>()
        .0
        .iter()
        .zip(rgba.as_chunks_mut::<4>().0)
    {
        if src[3] == 0 {
            continue;
        }
        dst[0] = src[2];
        dst[1] = src[1];
        dst[2] = src[0];
        dst[3] = src[3];
    }
    rgba
}

/// 读一张 32 位位图的像素，返回 **BGRA**（GDI 的原生顺序）。
fn read_dib(hbm: HBITMAP, w: u32, h: u32) -> Option<Vec<u8>> {
    let pixels = usize::try_from(w).ok()? * usize::try_from(h).ok()?;
    let mut buf = vec![0u8; pixels * 4];

    let mut bi = BITMAPINFO::default();
    bi.bmiHeader.biSize = u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).ok()?;
    bi.bmiHeader.biWidth = i32::try_from(w).ok()?;
    // 负高度 = 自上而下，省掉一次翻转。
    bi.bmiHeader.biHeight = -i32::try_from(h).ok()?;
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    bi.bmiHeader.biCompression = 0; // BI_RGB

    let dc = unsafe { GetDC(None) };
    if dc.is_invalid() {
        return None;
    }
    let lines = unsafe {
        GetDIBits(
            dc,
            hbm,
            0,
            h,
            Some(buf.as_mut_ptr().cast()),
            &raw mut bi,
            DIB_RGB_COLORS,
        )
    };
    unsafe { ReleaseDC(None, dc) };
    if lines == 0 {
        return None;
    }
    Some(buf)
}

/// Fallback: draw the icon into a 32-bit DIB with `DrawIconEx`.
/// The DIB must be selected into a memory DC before drawing, and `GdiFlush`
/// must run before reading `bits` (Microsoft Learn, CreateDIBSection Remarks).
fn draw_into_dib(hicon: HICON) -> Option<IconBitmap> {
    // Resolve every fallible computation before creating GDI objects; an early
    // return past that point would leak the DIB handle.
    let size = icon_size(hicon).unwrap_or(256);
    let side = i32::try_from(size).ok()?;
    let pixels = usize::try_from(size).ok()? * usize::try_from(size).ok()?;

    let mut bi = BITMAPINFO::default();
    bi.bmiHeader.biSize = u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).ok()?;
    bi.bmiHeader.biWidth = side;
    bi.bmiHeader.biHeight = -side;
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    bi.bmiHeader.biCompression = 0;

    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let dib =
        unsafe { CreateDIBSection(None, &raw const bi, DIB_RGB_COLORS, &raw mut bits, None, 0) }
            .ok()?;
    if bits.is_null() {
        let _ = unsafe { DeleteObject(HGDIOBJ(dib.0)) };
        return None;
    }

    // Memory DC compatible with the screen; the DIB is the render target.
    let screen = unsafe { GetDC(None) };
    if screen.0.is_null() {
        let _ = unsafe { DeleteObject(HGDIOBJ(dib.0)) };
        return None;
    }
    let dc = unsafe { CreateCompatibleDC(Some(screen)) };
    unsafe { ReleaseDC(None, screen) };
    if dc.0.is_null() {
        let _ = unsafe { DeleteObject(HGDIOBJ(dib.0)) };
        return None;
    }

    let old = unsafe { SelectObject(dc, HGDIOBJ(dib.0)) };
    let drawn = unsafe { DrawIconEx(dc, 0, 0, hicon, side, side, 0, None, DI_NORMAL) };
    unsafe {
        // `bits` is shared with GDI; flush before reading it.
        let _ = GdiFlush();
        SelectObject(dc, old); // restore before deleting the DIB
        let _ = DeleteDC(dc);
    }
    if drawn.is_err() {
        let _ = unsafe { DeleteObject(HGDIOBJ(dib.0)) };
        return None;
    }

    let raw = unsafe { std::slice::from_raw_parts(bits.cast::<u8>(), pixels * 4) };
    let rgba = bgra_to_rgba(raw);
    let _ = unsafe { DeleteObject(HGDIOBJ(dib.0)) };

    let bmp = IconBitmap {
        width: size,
        height: size,
        rgba,
    };
    bmp.has_visible_pixels().then_some(bmp)
}

/// 图标自身尺寸。`GetIconInfo` 的彩色位图尺寸就是它。
fn icon_size(hicon: HICON) -> Option<u32> {
    let mut ii = ICONINFO::default();
    unsafe { GetIconInfo(hicon, &raw mut ii) }.ok()?;
    let mut bm = BITMAP::default();
    let got = unsafe {
        GetObjectW(
            HGDIOBJ(ii.hbmColor.0),
            i32::try_from(std::mem::size_of::<BITMAP>()).ok()?,
            Some((&raw mut bm).cast()),
        )
    };
    unsafe {
        if !ii.hbmColor.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(ii.hbmColor.0));
        }
        if !ii.hbmMask.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(ii.hbmMask.0));
        }
    }
    if got == 0 || bm.bmWidth <= 0 {
        return None;
    }
    u32::try_from(bm.bmWidth).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{LoadIconW, IDI_APPLICATION};

    #[test]
    fn bgra_conversion_swaps_channels_and_clears_transparent_rgb() {
        let bgra = [
            10, 20, 30, 255, // opaque
            40, 50, 60, 0, // alpha 0 -> zeroed
            1, 2, 3, 128, // alpha preserved
        ];
        assert_eq!(
            bgra_to_rgba(&bgra),
            vec![30, 20, 10, 255, 0, 0, 0, 0, 3, 2, 1, 128]
        );
    }

    #[test]
    fn bgra_conversion_tolerates_empty_and_ragged_input() {
        assert!(bgra_to_rgba(&[]).is_empty());
        // A trailing partial pixel is dropped, not read out of bounds.
        assert_eq!(bgra_to_rgba(&[1, 2, 3, 4, 9, 9]), vec![3, 2, 1, 4, 0, 0]);
    }

    /// Real HICON must produce visible pixels in the DIB.
    /// Run: `cargo test -p nooksill-platform --lib icon -- --ignored`
    #[test]
    #[ignore = "需要桌面会话，显式 --ignored 运行"]
    fn dib_fallback_draws_into_the_dib_not_the_screen() {
        // Shared system icon; do not call DestroyIcon.
        let hicon = unsafe { LoadIconW(None, IDI_APPLICATION) }.expect("加载系统图标");
        let bmp = draw_into_dib(hicon).expect("DIB 里应当有可见像素");
        assert!(bmp.has_visible_pixels());
        assert_eq!(bmp.width, bmp.height);
    }
}
