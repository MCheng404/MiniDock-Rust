//! Frame pacing helpers.

use egui_wgpu::wgpu::TextureFormatFeatureFlags;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{DwmGetCompositionTimingInfo, DWM_TIMING_INFO};

/// Frame pacing interval. 0 cap = unlimited (1 ms pump); VSync wins.
#[must_use]
pub fn pacing_ms(cap: i32, vsync: bool, vsync_ms: u32) -> u32 {
    if vsync {
        return vsync_ms.clamp(1, 100);
    }
    if cap > 0 {
        return (1000u32 / cap as u32).max(1);
    }
    1
}

/// Auto MSAA by window area; config can force 1/2/4.
#[must_use]
pub const fn auto_msaa_samples(area: u64, pref: u32) -> u32 {
    match pref {
        1 => 1,
        2 => 2,
        4 => 4,
        _ => {
            if area > 2_000_000 {
                2
            } else {
                4
            }
        }
    }
}

/// Resolve a requested sample count against what the **device** accepts.
///
/// The adapter reports driver-level counts (this machine: 1/2/4/8), but 2 and 8
/// are only legal once the device enabled
/// `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES`. Without it the WebGPU spec
/// guarantees just 1 and 4 for `Rgba8UnormSrgb`, and asking for 2 is a
/// validation error rather than a fallback - that is what used to panic on
/// full-screen surfaces.
///
/// When the requested count is unavailable, round **up** to 4 instead of down
/// to 1: 2x was chosen to save memory, but silently dropping antialiasing is a
/// quality change, not a memory change.
#[must_use]
pub fn resolve_samples(
    wanted: u32,
    adapter_flags: TextureFormatFeatureFlags,
    adapter_specific: bool,
) -> u32 {
    if wanted <= 1 {
        return 1;
    }
    if adapter_specific && adapter_flags.sample_count_supported(wanted) {
        return wanted;
    }
    if adapter_flags.sample_count_supported(4) {
        return 4;
    }
    1
}

/// DWM composition interval in milliseconds. Falls back to 60 Hz on error.
#[must_use]
pub fn vsync_interval_ms(hwnd: HWND) -> u32 {
    let mut timing = DWM_TIMING_INFO {
        cbSize: std::mem::size_of::<DWM_TIMING_INFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `timing` is a valid DWM_TIMING_INFO with cbSize set.
    if unsafe { DwmGetCompositionTimingInfo(hwnd, &mut timing) }.is_err() {
        return 16;
    }
    let (num, den) = (
        timing.rateRefresh.uiNumerator,
        timing.rateRefresh.uiDenominator,
    );
    if num == 0 || den == 0 {
        return 16;
    }
    (1000u64 * u64::from(den) / u64::from(num)).clamp(1, 100) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::GetDesktopWindow;

    #[test]
    fn pacing_respects_cap_and_vsync() {
        assert_eq!(pacing_ms(0, false, 16), 1, "unlimited");
        assert_eq!(pacing_ms(60, false, 16), 16);
        assert_eq!(pacing_ms(144, false, 16), 6);
        assert_eq!(pacing_ms(240, false, 16), 4);
        assert_eq!(pacing_ms(30, true, 8), 8, "vsync overrides cap");
    }

    #[test]
    fn auto_msaa_drops_for_large_windows() {
        assert_eq!(auto_msaa_samples(1320 * 1080, 0), 4);
        assert_eq!(auto_msaa_samples(2560 * 1600, 0), 2);
        assert_eq!(auto_msaa_samples(2560 * 1600, 4), 4, "forced 4x");
        assert_eq!(auto_msaa_samples(1320 * 1080, 1), 1, "forced off");
    }

    #[test]
    fn msaa_two_requires_the_adapter_specific_feature() {
        use egui_wgpu::wgpu::TextureFormatFeatureFlags as F;
        let all = F::MULTISAMPLE_X2 | F::MULTISAMPLE_X4 | F::MULTISAMPLE_X8;
        // 没声明特性时 2x 是非法值：升到 4，不能降到 1（那是改画质）。
        assert_eq!(resolve_samples(2, all, false), 4);
        assert_eq!(resolve_samples(2, all, true), 2);
        // 4 是规范保证的档位，两种设备都要给。
        assert_eq!(resolve_samples(4, all, false), 4);
        assert_eq!(resolve_samples(4, all, true), 4);
        // 硬件连 4 都不支持时只能退回单采样。
        assert_eq!(resolve_samples(4, F::MULTISAMPLE_X2, false), 1);
        assert_eq!(resolve_samples(0, all, true), 1);
        assert_eq!(resolve_samples(1, all, true), 1);
    }

    #[test]
    #[ignore = "requires a desktop session; run with --ignored"]
    fn dwm_interval_is_plausible() {
        // SAFETY: no arguments.
        let hwnd = unsafe { GetDesktopWindow() };
        let ms = vsync_interval_ms(hwnd);
        assert!((1..=100).contains(&ms), "unexpected interval {ms} ms");
        eprintln!("DWM refresh interval: {ms} ms");
    }
}
