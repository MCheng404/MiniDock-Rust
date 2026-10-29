//! ARGB 颜色与色彩空间转换。
//!
//! 约定与 GDI+ 完全一致：`0xAARRGGBB`，**非预乘** alpha。
//! DIB section 用 `PixelFormat32bppPARGB`，预乘由 GDI+ 在写入时完成，
//! 调用方始终传非预乘值 —— 这一点与 C 版 `ARGBC()` 宏相同。

/// `0xAARRGGBB`。
pub type Argb = u32;

/// 构造不透明颜色。
#[must_use]
pub const fn rgb(r: u8, g: u8, b: u8) -> Argb {
    argb(255, r, g, b)
}

/// 构造带 alpha 的颜色。对应 C 版 `ARGBC(a, r, g, b)`。
#[must_use]
pub const fn argb(a: u8, r: u8, g: u8, b: u8) -> Argb {
    ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

#[must_use]
pub const fn alpha(c: Argb) -> u8 {
    (c >> 24) as u8
}
#[must_use]
pub const fn red(c: Argb) -> u8 {
    (c >> 16) as u8
}
#[must_use]
pub const fn green(c: Argb) -> u8 {
    (c >> 8) as u8
}
#[must_use]
pub const fn blue(c: Argb) -> u8 {
    c as u8
}

/// 只替换 alpha，保留 RGB。
#[must_use]
pub const fn with_alpha(c: Argb, a: u8) -> Argb {
    (c & 0x00FF_FFFF) | ((a as u32) << 24)
}

/// 按 alpha 比例缩放（用于"整体变淡"，例如隐藏动画中的淡出）。
///
/// `scale` 取 `0.0..=1.0`；越界会夹紧而不是回绕 —— 原版 `ARGBC` 直接
/// 把 `op * 255` 塞进 `int`，`Opacity=1000` 时 alpha 溢出进颜色通道，
/// 是 issue 里记录过的真实缺陷。
#[must_use]
pub fn scale_alpha(c: Argb, scale: f32) -> Argb {
    let s = scale.clamp(0.0, 1.0);
    let a = (alpha(c) as f32 * s).round().clamp(0.0, 255.0) as u8;
    with_alpha(c, a)
}

/// 线性插值（用于渐变端点、强调色混色）。
#[must_use]
pub fn mix(a: Argb, b: Argb, t: f32) -> Argb {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| -> u8 { (x as f32 + (y as f32 - x as f32) * t).round() as u8 };
    argb(
        f(alpha(a), alpha(b)),
        f(red(a), red(b)),
        f(green(a), green(b)),
        f(blue(a), blue(b)),
    )
}

/// HSV → RGB，`h` 单位为度（任意实数，内部取模），`s`/`v` 取 `0.0..=1.0`。
///
/// 对应 C 版 `HsvRgb()`：`AccentTone` 开启时用系统强调色的**色相**生成
/// Material You 风格的两种色调，饱和度/明度由深/浅色主题决定。
#[must_use]
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let h = h.rem_euclid(360.0) / 60.0;
    let s = s.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);
    let i = h.floor() as i32;
    let f = h - h.floor();
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    let (r, g, b) = match i % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    let to = |x: f32| (x * 255.0).round().clamp(0.0, 255.0) as u8;
    (to(r), to(g), to(b))
}

/// RGB → 色相（度，`0.0..360.0`）。对应 `RefreshAccent()` 里的那段反算。
///
/// 灰阶（`d <= 0.001`）返回 0.0，与原版一致。
#[must_use]
pub fn hue_of(c: Argb) -> f32 {
    let r = red(c) as f32 / 255.0;
    let g = green(c) as f32 / 255.0;
    let b = blue(c) as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    if d <= 0.001 {
        return 0.0;
    }
    let mut h = if (max - r).abs() < f32::EPSILON {
        ((g - b) / d).rem_euclid(6.0)
    } else if (max - g).abs() < f32::EPSILON {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    h *= 60.0;
    if h < 0.0 {
        h += 360.0;
    }
    h
}

/// Win32 `COLORREF`（`0x00BBGGRR`）→ ARGB。注册表/系统 API 的颜色都是这个字节序。
#[must_use]
pub const fn from_colorref(c: u32) -> Argb {
    argb(
        255,
        (c & 0xFF) as u8,
        ((c >> 8) & 0xFF) as u8,
        ((c >> 16) & 0xFF) as u8,
    )
}

/// ARGB → `COLORREF`。`BgColor1/2` 在 ini 里存的是 `COLORREF` 十进制值。
#[must_use]
pub const fn to_colorref(c: Argb) -> u32 {
    ((blue(c) as u32) << 16) | ((green(c) as u32) << 8) | (red(c) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argb_layout_matches_gdiplus() {
        // 0xAARRGGBB：alpha 在高字节，与 GDI+ / Windows 一致。
        assert_eq!(argb(0x12, 0x34, 0x56, 0x78), 0x1234_5678);
        assert_eq!(alpha(0x1234_5678), 0x12);
        assert_eq!(red(0x1234_5678), 0x34);
        assert_eq!(green(0x1234_5678), 0x56);
        assert_eq!(blue(0x1234_5678), 0x78);
    }

    #[test]
    fn scale_alpha_clamps_instead_of_overflowing() {
        // 原版缺陷复现点：alpha 溢出绝不能污染颜色通道。
        let c = argb(200, 10, 20, 30);
        assert_eq!(scale_alpha(c, 1.5), c);
        assert_eq!(red(scale_alpha(c, 1.5)), 10);
        assert_eq!(green(scale_alpha(c, 1.5)), 20);
        assert_eq!(blue(scale_alpha(c, 1.5)), 30);
        assert_eq!(alpha(scale_alpha(c, -1.0)), 0);
        assert_eq!(alpha(scale_alpha(c, 0.5)), 100);
    }

    #[test]
    fn colorref_roundtrip() {
        let c = argb(255, 0x11, 0x22, 0x33);
        assert_eq!(to_colorref(c), 0x0033_2211);
        assert_eq!(from_colorref(to_colorref(c)), c);
    }

    #[test]
    fn hsv_primaries() {
        assert_eq!(hsv_to_rgb(0.0, 1.0, 1.0), (255, 0, 0));
        assert_eq!(hsv_to_rgb(120.0, 1.0, 1.0), (0, 255, 0));
        assert_eq!(hsv_to_rgb(240.0, 1.0, 1.0), (0, 0, 255));
        assert_eq!(hsv_to_rgb(360.0, 1.0, 1.0), (255, 0, 0));
        assert_eq!(hsv_to_rgb(0.0, 0.0, 0.5), (128, 128, 128));
    }

    #[test]
    fn hue_roundtrip_for_saturated_colors() {
        for hue in [0.0_f32, 30.0, 90.0, 180.0, 270.0, 350.0] {
            let (r, g, b) = hsv_to_rgb(hue, 1.0, 1.0);
            let back = hue_of(rgb(r, g, b));
            // HSV→RGB 的 8 位量化会引入最多 ~1° 的误差。
            assert!((back - hue).abs() < 1.5, "hue {hue} -> {back}");
        }
    }
}
