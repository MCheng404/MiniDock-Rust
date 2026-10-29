//! DPI 与像素换算。
//!
//! 原版到处是 `Sc(v)` = `MulDiv(v, g_dpi, 96)`。这里保留同样的语义
//! （四舍五入、远离零），但把它集中成一个函数，避免"某处忘了缩放"
//! 这类只有在 150% 缩放下才暴露的缺陷。

/// `v * dpi / 96`，四舍五入。
///
/// 与 Win32 `MulDiv` 对齐：内部用 64 位中间值防止 `v * dpi` 溢出，
/// 结果向远离零的方向取整（`f64::round` 的行为）。
#[must_use]
pub fn scale(v: i32, dpi: i32) -> i32 {
    if dpi == 96 {
        return v;
    }
    let num = i64::from(v) * i64::from(dpi);
    // 用整数运算保持可预测性，避免浮点在极大值上的平台差异。
    let half = 96 / 2;
    let r = if num >= 0 {
        (num + half) / 96
    } else {
        (num - half) / 96
    };
    r.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// 物理像素 → 96 DPI 逻辑像素（用于日志/诊断时把数字还原成设计值）。
#[must_use]
pub fn unscale(v: i32, dpi: i32) -> i32 {
    if dpi == 96 {
        return v;
    }
    scale(v, 96) * 96 / dpi.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_at_96() {
        assert_eq!(scale(48, 96), 48);
    }

    #[test]
    fn rounds_like_muldiv() {
        assert_eq!(scale(48, 120), 60);
        assert_eq!(scale(48, 144), 72);
        assert_eq!(scale(48, 192), 96);
        // 25 * 120 / 96 = 31.25 -> 31
        assert_eq!(scale(25, 120), 31);
        // 1 * 120 / 96 = 1.25 -> 1；1 * 144/96 = 1.5 -> 2
        assert_eq!(scale(1, 120), 1);
        assert_eq!(scale(1, 144), 2);
    }

    #[test]
    fn negative_rounds_away_from_zero() {
        assert_eq!(scale(-1, 144), -2);
        assert_eq!(scale(-48, 120), -60);
    }

    #[test]
    fn no_overflow_on_extremes() {
        // C 版 MulDiv 在 32 位中间值上会溢出；这里必须是饱和而不是回绕。
        assert_eq!(scale(i32::MAX, 480), i32::MAX);
        assert_eq!(scale(i32::MIN, 480), i32::MIN);
    }
}
