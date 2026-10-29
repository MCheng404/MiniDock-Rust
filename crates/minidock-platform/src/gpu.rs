//! 显卡适配器枚举。
//!
//! 为什么走 DXGI 而不是 wgpu：设置界面不能依赖 GPU 层（ADR-003），
//! 而 DXGI 正是 wgpu 的 DX12 后端取名字的同源 API —— 两边看到的名字一致，
//! 才能把"用户选的字符串"当成匹配键用。
//!
//! 这个模块**只负责枚举**，不负责挑选。挑选在 `minidock-gpu` 里做，
//! 因为只有那里知道后端如何按名字构造设备。

use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};

/// 适配器类型。判断只是启发式，用于界面排序与提示，不参与任何逻辑分支。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AdapterKind {
    /// 独显：有独立的显存分区。
    Discrete,
    /// 核显：显存与内存共享。
    Integrated,
    /// 软件光栅器（`Microsoft Basic Render Driver`）。不能作为 Dock 的后端，
    /// 但保留在列表里以便解释"为什么没得选"。
    Software,
    /// 判断不出来（虚拟显卡、驱动上报异常值）。
    Other,
}

impl AdapterKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Discrete => "独显",
            Self::Integrated => "核显",
            Self::Software => "软件",
            Self::Other => "其它",
        }
    }
}

/// 一条适配器记录。字段只保留"能显示 + 能匹配"的部分。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterInfo {
    pub name: String,
    pub vendor: u32,
    pub device_id: u32,
    pub dedicated_mb: usize,
    pub kind: AdapterKind,
}

impl AdapterInfo {
    /// 界面上显示的一行：`NVIDIA GeForce RTX 5070 Laptop GPU（独显 · 8 GB）`。
    #[must_use]
    pub fn display_name(&self) -> String {
        if self.dedicated_mb >= 1024 {
            format!(
                "{}（{} · {} GB）",
                self.name,
                self.kind.label(),
                self.dedicated_mb / 1024
            )
        } else {
            format!("{}（{}）", self.name, self.kind.label())
        }
    }
}

/// 枚举系统里的适配器，按"独显 → 核显 → 其它 → 软件"排序。
///
/// DXGI 会把同一块 GPU 枚举两次（接在不同输出上时），这里按
/// `(vendor, device_id, name)` 去重 —— 否则下拉框里会出现两个同名项，
/// 用户选哪个都一样，只会困惑。
#[must_use]
pub fn adapters() -> Vec<AdapterInfo> {
    let Ok(factory): Result<IDXGIFactory1, _> = (unsafe { CreateDXGIFactory1() }) else {
        return Vec::new();
    };

    let mut out: Vec<AdapterInfo> = Vec::new();
    // 枚举没有总数上限，走到 DXGI_ERROR_NOT_FOUND 为止；16 是防御性上界。
    for index in 0..16u32 {
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(index) }) else {
            break;
        };
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
            continue;
        };
        let end = desc
            .Description
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(desc.Description.len());
        let name = String::from_utf16_lossy(&desc.Description[..end]);
        if name.is_empty() {
            continue;
        }
        let dedicated_mb = desc.DedicatedVideoMemory / (1024 * 1024);
        let kind = classify(&name, dedicated_mb);
        let info = AdapterInfo {
            name,
            vendor: desc.VendorId,
            device_id: desc.DeviceId,
            dedicated_mb,
            kind,
        };
        if !out.iter().any(|a| {
            a.vendor == info.vendor && a.device_id == info.device_id && a.name == info.name
        }) {
            out.push(info);
        }
    }

    out.sort_by_key(|a| a.kind);
    out
}

/// 名字 + 显存规模判断类型。
///
/// 阈值取 1 GB：核显也可能有 256–512 MB 的独立划拨（本机 AMD 610M 报 485 MB），
/// 而近十年的独显不会低于 1 GB。判错的代价只是界面上的一个标签。
fn classify(name: &str, dedicated_mb: usize) -> AdapterKind {
    if name.contains("Microsoft Basic Render") || name.contains("Microsoft Basic Display") {
        AdapterKind::Software
    } else if dedicated_mb >= 1024 {
        AdapterKind::Discrete
    } else if dedicated_mb > 0 {
        AdapterKind::Integrated
    } else {
        AdapterKind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::{adapters, classify, AdapterKind};

    #[test]
    fn classify_uses_vendor_neutral_heuristics() {
        assert_eq!(
            classify("AMD Radeon(TM) 610M", 485),
            AdapterKind::Integrated
        );
        assert_eq!(
            classify("NVIDIA GeForce RTX 5070 Laptop GPU", 7891),
            AdapterKind::Discrete
        );
        assert_eq!(
            classify("Microsoft Basic Render Driver", 0),
            AdapterKind::Software
        );
        assert_eq!(classify("Some Virtual GPU", 0), AdapterKind::Other);
    }

    #[test]
    fn adapters_are_available_and_deduplicated() {
        let list = adapters();
        // 没有显卡的 CI 机器允许返回空，但只要有内容就必须是干净的。
        for (i, a) in list.iter().enumerate() {
            assert!(!a.name.is_empty());
            for b in &list[i + 1..] {
                assert!(
                    !(a.vendor == b.vendor && a.device_id == b.device_id && a.name == b.name),
                    "重复适配器: {} / {}",
                    a.name,
                    b.name
                );
            }
        }
    }

    #[test]
    fn software_adapters_sort_last() {
        let list = adapters();
        let first_software = list.iter().position(|a| a.kind == AdapterKind::Software);
        if let Some(i) = first_software {
            assert!(
                list[i..].iter().all(|a| a.kind == AdapterKind::Software),
                "软件光栅器必须排在最后"
            );
        }
    }
}
