//! 进程内存自采样。
//!
//! 为什么在进程内采样，而不是外部 `Get-Process`：
//!
//! 1. **峰值只有进程内读得准。** 外部轮询是采点，必然漏掉瞬时峰值，
//!    而显卡驱动初始化的峰值往往就是全程最高点。
//! 2. **口径必须唯一。** 界面上的 RSS、文档里的数字、探针输出的数字
//!    必须是同一个函数算出来的，否则讨论"271 MB 还是 429 MB"时各说各话。
//! 3. **可无人值守。** 探针在 RDP / CI 里跑时不该依赖外部脚本。
//!
//! 三个口径的差别（讨论内存时必须写清楚用的是哪个）：
//!
//! - `working_set`：物理内存里实际驻留的页。任务管理器的"内存"列。
//! - `private`：提交的私有字节，含被换出到页面文件的部分。
//!   驱动常把大量内存标成私有但不驻留，所以这个数通常明显大于工作集。
//! - `peak_working_set`：启动以来的工作集峰值。判断"是不是有一瞬间爆掉"看它。

use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::Threading::GetCurrentProcess;

/// 一次内存采样。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemStats {
    /// 当前工作集（字节）。
    pub working_set: usize,
    /// 工作集峰值（字节）。
    pub peak_working_set: usize,
    /// 私有提交（字节）。
    pub private: usize,
}

impl MemStats {
    /// 读取当前进程的内存计数。查询失败时返回全零（不 panic：
    /// 采样失败不该让被测程序崩溃，否则就更测不出来了）。
    #[must_use]
    pub fn now() -> Self {
        let mut counters = PROCESS_MEMORY_COUNTERS_EX {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            ..Default::default()
        };
        let queried = unsafe {
            GetProcessMemoryInfo(GetCurrentProcess(), (&raw mut counters).cast(), counters.cb)
        };
        if queried.is_err() {
            return Self::default();
        }
        Self {
            working_set: counters.WorkingSetSize,
            peak_working_set: counters.PeakWorkingSetSize,
            private: counters.PrivateUsage,
        }
    }

    /// 工作集（MB）。
    #[must_use]
    pub fn ws_mb(self) -> f32 {
        mb(self.working_set)
    }

    /// 工作集峰值（MB）。
    #[must_use]
    pub fn peak_ws_mb(self) -> f32 {
        mb(self.peak_working_set)
    }

    /// 私有提交（MB）。
    #[must_use]
    pub fn private_mb(self) -> f32 {
        mb(self.private)
    }

    /// 单行摘要，供日志与探针输出复用。
    #[must_use]
    pub fn summary(self) -> String {
        format!(
            "WS {:.1} MB / 峰值 {:.1} MB / 私有 {:.1} MB",
            self.ws_mb(),
            self.peak_ws_mb(),
            self.private_mb()
        )
    }
}

fn mb(bytes: usize) -> f32 {
    bytes as f32 / (1024.0 * 1024.0)
}

/// 两个采样之间的增量，用来回答"这一步花了多少内存"。
#[derive(Clone, Copy, Debug, Default)]
pub struct Delta {
    pub ws_mb: f32,
    pub private_mb: f32,
}

impl Delta {
    #[must_use]
    pub fn between(before: MemStats, after: MemStats) -> Self {
        Self {
            ws_mb: after.ws_mb() - before.ws_mb(),
            private_mb: after.private_mb() - before.private_mb(),
        }
    }

    #[must_use]
    pub fn summary(self) -> String {
        format!("+{:.1} MB WS / +{:.1} MB 私有", self.ws_mb, self.private_mb)
    }
}

#[cfg(test)]
mod tests {
    use super::MemStats;

    #[test]
    fn 采样到的数值大于零() {
        // 任何活着的进程都有工作集；为 0 说明查询路径写错了。
        let s = MemStats::now();
        assert!(s.working_set > 0, "工作集为 0，查询失败: {s:?}");
        assert!(s.peak_working_set >= s.working_set);
    }

    #[test]
    fn 摘要包含三个口径() {
        let s = MemStats::now();
        let text = s.summary();
        assert!(text.contains("WS"));
        assert!(text.contains("峰值"));
        assert!(text.contains("私有"));
    }
}
