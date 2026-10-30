//! 从路径取图标（HICON → RGBA）。
//!
//! 原版走 GDI+（`HiconToGp`）：`GdipCreateBitmapFromHICON` 会给很多图标留下
//! 一块黑底，所以它先手工 `GetIconInfo` + `GetDIBits` 取像素，取不到才回退。
//! 这里保留同样的**三层顺序**，只是不再经 GDI+ 中转 —— 我们只需要一块 RGBA：
//!
//! 1. `GetIconInfo` + `GetDIBits(hbmColor)`：32 位图标自带 alpha，首选；
//! 2. 全透明（老式 24 位图标，alpha 全 0）→ 用 `hbmMask` 造 alpha；
//! 3. 仍拿不到 → `DrawIconEx` 画进 32 位 DIB（等价原版的
//!    `GdipCreateBitmapFromHICON` 兜底）。
//!
//! 尺寸选择照抄原版：先 JUMBO（256），再 EXTRALARGE（48）。Dock 图标在
//! 150% DPI 下最大要 88 逻辑点 ≈ 132 像素，48 像素放大后会糊。

use windows::core::PCWSTR;
use windows::Win32::Graphics::Gdi::{
    CreateDIBSection, DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO,
    BITMAPINFOHEADER, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
};
use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
use windows::Win32::UI::Shell::{
    SHGetFileInfoW, SHGetImageList, SHFILEINFOW, SHGFI_SYSICONINDEX, SHIL_EXTRALARGE, SHIL_JUMBO,
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

/// 按原版顺序取 shell 图标：JUMBO（256 px）→ EXTRALARGE（48 px）。
///
/// `path` 可以是 exe、文件夹、`.lnk`、网址 —— 只要是 Explorer 能给图标的都行。
#[must_use]
pub fn shell_icon(path: &str) -> Option<IconBitmap> {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();

    let trace = std::env::var_os("MINIDOCK_ICON_TRACE").is_some();
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
/// 对应原版 `LnkIconBitmap` 的 `PrivateExtractIconsW` 路径：快捷方式里存的
/// `iconLoc` + `iconIdx` 指向的常是 exe 内的资源，shell 的图像列表覆盖不到。
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

    // 释放 GetIconInfo 复制出来的两个位图（原版同样在这里 DeleteObject）。
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

    // BGRA → RGBA。顺带把全透明像素的 RGB 清零：
    // 预乘 alpha 的绘制路径里，残留的 RGB 会在边缘透出脏色。
    let mut rgba = vec![0u8; pixels * 4];
    for i in 0..pixels {
        let s = i * 4;
        let a = bgra[s + 3];
        if a == 0 {
            continue;
        }
        rgba[s] = bgra[s + 2];
        rgba[s + 1] = bgra[s + 1];
        rgba[s + 2] = bgra[s];
        rgba[s + 3] = a;
    }
    Some(IconBitmap {
        width: w,
        height: h,
        rgba,
    })
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

/// 第三层：把图标直接画进 32 位 DIB。`DrawIconEx` 会正确处理掩码与 alpha。
fn draw_into_dib(hicon: HICON) -> Option<IconBitmap> {
    // 先问图标本身多大；问不到就按 256 试（jumbo 的常见尺寸）。
    let size = icon_size(hicon).unwrap_or(256);

    let mut bi = BITMAPINFO::default();
    bi.bmiHeader.biSize = u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).ok()?;
    bi.bmiHeader.biWidth = i32::try_from(size).ok()?;
    bi.bmiHeader.biHeight = -i32::try_from(size).ok()?;
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

    let dc = unsafe { GetDC(None) };
    if dc.is_invalid() {
        let _ = unsafe { DeleteObject(HGDIOBJ(dib.0)) };
        return None;
    }
    let (sw, sh) = match (i32::try_from(size), i32::try_from(size)) {
        (Ok(a), Ok(b)) => (a, b),
        _ => return None,
    };
    let drawn = unsafe { DrawIconEx(dc, 0, 0, hicon, sw, sh, 0, None, DI_NORMAL) };
    unsafe {
        ReleaseDC(None, dc);
    }
    if drawn.is_err() {
        let _ = unsafe { DeleteObject(HGDIOBJ(dib.0)) };
        return None;
    }

    let pixels = usize::try_from(size).ok()? * usize::try_from(size).ok()?;
    let raw = unsafe { std::slice::from_raw_parts(bits.cast::<u8>(), pixels * 4) };
    let mut rgba = vec![0u8; pixels * 4];
    for i in 0..pixels {
        let s = i * 4;
        let a = raw[s + 3];
        if a == 0 {
            continue;
        }
        rgba[s] = raw[s + 2];
        rgba[s + 1] = raw[s + 1];
        rgba[s + 2] = raw[s];
        rgba[s + 3] = a;
    }
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
