//! 系统资源采样：CPU 占用率、内存占用、磁盘占用。
//!
//! 对应上游 v1.6.0 的 `ResCpuPercent()` 与 `ResSample()`
//! （minidock.c:6505、6533）。上游的三个数据源：
//!
//! - **CPU**：`GetSystemTimes` 的 idle / kernel / user 三个 FILETIME，
//!   取两次采样之间的差值算占比。idle 计入分母，占比 = 1 - idle/total。
//! - **内存**：`GlobalMemoryStatusEx` 的 `dwMemoryLoad`（系统直接给的
//!   百分比）与 `ullTotalPhys` / `ullAvailPhys`（字节数）。
//! - **磁盘**：`GetDiskFreeSpaceExW` 取 C 盘剩余 / 总容量。
//!
//! 全部是只读调用，不改系统状态。

use windows::Win32::Foundation::FILETIME;
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::GetSystemTimes;

/// 一次采样的结果。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct SysInfo {
    /// CPU 占用率（百分比，0..=100）。
    pub cpu_percent: u8,
    /// 内存占用率（百分比，0..=100）。
    pub mem_percent: u8,
    /// 磁盘占用率（百分比，0..=100）。
    pub disk_percent: u8,
    /// 已用内存（MB）。
    pub mem_used_mb: u64,
    /// 总内存（MB）。
    pub mem_total_mb: u64,
    /// 磁盘剩余（GB）。
    pub disk_free_gb: u64,
    /// 磁盘总量（GB）。
    pub disk_total_gb: u64,
}

/// CPU 占用率的计算器。
///
/// `GetSystemTimes` 给的是**累计值**，必须拿两次采样的差值才能算出占用率。
/// 所以这个类型要留着自己上次的值 —— 上游用三个 `static` 变量做同样的事，
/// 这里做成显式的状态，避免全局可变状态。
#[derive(Copy, Clone, Debug, Default)]
pub struct CpuSampler {
    /// 上一次采样的 idle / kernel / user 累计值（100 ns 单位）。
    prev: Option<(u64, u64, u64)>,
}

impl CpuSampler {
    #[must_use]
    pub const fn new() -> Self {
        Self { prev: None }
    }

    /// 采样一次，返回 CPU 占用率（百分比）。
    ///
    /// 第一次调用返回 0 —— 没有上一次的基线，算不出差值。
    /// 这与上游一致（上游首次 `pi == 0` 时返回 0）。
    pub fn sample(&mut self) -> u8 {
        let Some((idle, kernel, user)) = system_times() else {
            return 0;
        };
        let Some((p_idle, p_kernel, p_user)) = self.prev else {
            self.prev = Some((idle, kernel, user));
            return 0;
        };
        self.prev = Some((idle, kernel, user));

        // 累计值只增不减；时钟回拨或计数器重置时差值会变成巨数，
        // 那种情况下宁可报 0 也不要给出一个离谱的百分比。
        let Some(d_idle) = idle.checked_sub(p_idle) else {
            return 0;
        };
        let Some(d_kernel) = kernel.checked_sub(p_kernel) else {
            return 0;
        };
        let Some(d_user) = user.checked_sub(p_user) else {
            return 0;
        };
        // 分母是 kernel + user（idle 已包含在 kernel 里）。
        // 上游写的是 `tot = dk + du`，其中 dk 是 kernel 的**全量**
        // （含 idle），这个口径照抄 —— 差一个 idle 会让占用率偏高。
        let total = d_kernel + d_user;
        if total == 0 {
            return 0;
        }
        let busy = total.saturating_sub(d_idle);
        let pct = busy.saturating_mul(100) / total;
        u8::try_from(pct.min(100)).unwrap_or(100)
    }
}

/// 读 `GetSystemTimes`，返回 (idle, kernel, user) 的 100 ns 累计值。
fn system_times() -> Option<(u64, u64, u64)> {
    let mut idle = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.ok()?;
    Some((
        filetime_value(idle),
        filetime_value(kernel),
        filetime_value(user),
    ))
}

/// FILETIME 的 100 ns 计数转成 u64。
fn filetime_value(ft: FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
}

/// 采样内存与磁盘信息。
///
/// CPU 部分不在这里 —— 它需要前后两次采样，由 [`CpuSampler`] 负责。
pub fn sample_memory_disk() -> SysInfo {
    let mut out = SysInfo::default();

    let mut ms = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    if unsafe { GlobalMemoryStatusEx(&mut ms) }.is_ok() {
        out.mem_percent = ms.dwMemoryLoad.min(100) as u8;
        out.mem_total_mb = ms.ullTotalPhys / (1024 * 1024);
        out.mem_used_mb = ms.ullTotalPhys.saturating_sub(ms.ullAvailPhys) / (1024 * 1024);
    }

    // 上游取的是 C 盘。
    let mut free_to_caller = 0_u64;
    let mut total = 0_u64;
    let mut total_free = 0_u64;
    let path: Vec<u16> = "C:\\".encode_utf16().chain(std::iter::once(0)).collect();
    if unsafe {
        GetDiskFreeSpaceExW(
            windows::core::PCWSTR(path.as_ptr()),
            Some(&mut free_to_caller),
            Some(&mut total),
            Some(&mut total_free),
        )
    }
    .is_ok()
    {
        const GB: u64 = 1024 * 1024 * 1024;
        out.disk_free_gb = free_to_caller / GB;
        out.disk_total_gb = total / GB;
        out.disk_percent = if total > 0 {
            let used = total.saturating_sub(free_to_caller);
            u8::try_from(used.saturating_mul(100) / total).unwrap_or(100)
        } else {
            0
        };
    }

    out
}

/// 一次完整采样：CPU + 内存 + 磁盘。
pub fn sample(cpu: &mut CpuSampler) -> SysInfo {
    let mut out = sample_memory_disk();
    out.cpu_percent = cpu.sample();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_sampler_first_call_is_zero() {
        let mut s = CpuSampler::new();
        // 第一次没有基线，算不出占用率。
        assert_eq!(s.sample(), 0);
    }

    #[test]
    fn cpu_percent_stays_in_range() {
        let mut s = CpuSampler::new();
        for _ in 0..5 {
            let v = s.sample();
            assert!(v <= 100, "占用率越界：{v}");
        }
    }

    #[test]
    fn memory_report_is_plausible() {
        let info = sample_memory_disk();
        assert!(info.mem_percent <= 100);
        assert!(info.mem_total_mb > 0, "应当读出总内存");
        assert!(info.mem_used_mb <= info.mem_total_mb);
    }

    #[test]
    fn disk_report_is_plausible() {
        let info = sample_memory_disk();
        assert!(info.disk_percent <= 100);
        assert!(info.disk_total_gb > 0, "应当读出磁盘容量");
        assert!(info.disk_free_gb <= info.disk_total_gb);
    }
}
