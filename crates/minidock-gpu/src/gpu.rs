//! wgpu 设备与颜色格式。
//!
//! M0 的结论改变了这里的设计：**不再创建 `wgpu::Surface`**。
//! wgpu 在 Windows 上没有带 alpha 的交换链（DX12/Vulkan 都只报 `Opaque`），
//! 所以渲染目标改成离屏纹理，呈现交给 `present.rs` 的 `UpdateLayeredWindow`
//! （ADR-008）。好处是逐像素透明与逐像素命中都回来了。
//!
//! 适配器选择（ADR-012）：
//! - `""`（自动）→ `PowerPreference::LowPower`，让后端挑省电的那块（通常是核显）。
//! - 非空 → 在 DX12 实例里按**名字**匹配。匹配不到就回退自动并在日志里说清楚，
//!   而不是让程序起不来 —— 换机器、换驱动都会让名字变。

use egui_wgpu::wgpu;

/// GPU 初始化错误。
#[derive(Debug)]
pub enum GpuError {
    Wgpu(String),
    Win32(&'static str),
}

impl std::fmt::Display for GpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Wgpu(s) => write!(f, "wgpu: {s}"),
            Self::Win32(s) => write!(f, "Win32: {s}"),
        }
    }
}

impl std::error::Error for GpuError {}

/// 这次到底用了哪块卡，以及为什么。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    /// 配置为自动。
    Auto,
    /// 按名字命中。
    Pinned,
    /// 配置了名字但没找到，已回退到自动。
    Fallback {
        wanted: String,
        available: Vec<String>,
    },
}

impl Selection {
    /// 日志与界面用的一句话说明。
    ///
    /// 回退时只列前三个候选：界面里这行是橙色提示，列全会把布局撑爆
    /// （本机有 4 个候选，其中 NVIDIA 还被 wgpu 重复枚举了一次）。
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Auto => "自动".to_owned(),
            Self::Pinned => "按配置指定".to_owned(),
            Self::Fallback { wanted, available } => {
                const SHOW: usize = 3;
                let head: Vec<&str> = available.iter().take(SHOW).map(String::as_str).collect();
                let more = available.len().saturating_sub(SHOW);
                let tail = if more > 0 {
                    format!(" 等 {} 项", available.len())
                } else {
                    String::new()
                };
                format!(
                    "配置的「{wanted}」未找到，已回退自动；可选：{}{tail}",
                    head.join(" / ")
                )
            }
        }
    }

    /// 是否发生了回退。界面据此把提示标黄。
    #[must_use]
    pub const fn fell_back(&self) -> bool {
        matches!(self, Self::Fallback { .. })
    }
}

pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub info: wgpu::AdapterInfo,
    pub selection: Selection,
}

impl std::fmt::Debug for Gpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gpu")
            .field("backend", &self.info.backend)
            .field("device", &self.info.name)
            .field("selection", &self.selection)
            .finish_non_exhaustive()
    }
}

impl Gpu {
    /// 离屏渲染用的颜色格式。
    ///
    /// **必须是 sRGB 目标**。第一版用了 `Rgba8Unorm`，结果 egui 在非 sRGB
    /// 目标上自己做了一次 gamma 编码，颜色被提亮（实测面板 24 灰变成 81 灰，
    /// 肉眼就是"磨砂变成半透明灰雾"）。换成 `Rgba8UnormSrgb` 后由硬件完成
    /// 编码，回读出来的字节就是屏幕值。
    pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

    /// 设备限制里**唯一被收紧**的项：非采样绑定预算。
    ///
    /// wgpu 默认 1_000_000。官方文档原话：这个值只影响 DX12 后端，
    /// 它按此**在创建设备时预分配**全局描述符堆；核显与 CPU 共享内存，
    /// 所以直接吃掉系统 RAM。
    ///
    /// 实测（AMD 610M，空设备工作集）：1_000_000 → 164 MB，4096 → 133 MB，
    /// 再往下（256/64）没有额外收益。Dock 每帧只有 1–2 个 bind group，
    /// 4096 绰绰有余。
    pub const DESCRIPTOR_BUDGET: u32 = 4096;

    /// 按适配器能力构造限制，只把描述符预算压下来。
    ///
    /// **不要用 `Limits::downlevel_defaults()`**：它的
    /// `max_texture_dimension_2d` 只有 2048，而 2560 宽的 Dock 在 150% DPI 下
    /// 就需要 3840 px 的离屏纹理 —— 会在超宽屏上建纹理直接失败。
    /// 这里用适配器自报的能力（通常 8192/16384），既安全又不额外吃内存
    /// （实测三档限制的内存量完全一致）。
    fn device_limits(adapter: &wgpu::Adapter) -> wgpu::Limits {
        wgpu::Limits {
            max_non_sampler_bindings: Self::DESCRIPTOR_BUDGET,
            ..adapter.limits()
        }
    }

    /// 建实例 + 按 `wanted` 挑适配器 + 请求设备。
    ///
    /// `wanted` 是适配器名字，空串表示自动。
    pub fn new(wanted: &str) -> Result<Self, GpuError> {
        // **强制 DX12**：默认实例在装了 Vulkan 驱动的机器上会选 Vulkan。
        // 虽然两条路都不支持 alpha 交换链，但 DX12 的驱动行为更可控、
        // 且与 DWM 合成路径一致（M0 首跑实测差异）。
        // 另外 Vulkan 与 DX12 报出来的适配器名字格式不同，按名字匹配会失效。
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12,
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
        });

        let wanted = wanted.trim();
        let (adapter, selection) = if wanted.is_empty() {
            (pick_auto(&instance)?, Selection::Auto)
        } else {
            match pick_by_name(&instance, wanted) {
                Some(adapter) => (adapter, Selection::Pinned),
                None => {
                    let available: Vec<String> =
                        pollster::block_on(instance.enumerate_adapters(wgpu::Backends::DX12))
                            .into_iter()
                            .map(|a| a.get_info().name)
                            .collect();
                    (
                        pick_auto(&instance)?,
                        Selection::Fallback {
                            wanted: wanted.to_owned(),
                            available,
                        },
                    )
                }
            }
        };

        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("minidock"),
            required_features: wgpu::Features::empty(),
            required_limits: Self::device_limits(&adapter),
            // Dock 每帧只有一两个 bind group，不需要分配器替性能多留余量。
            // 实测（AMD 610M 核显，空设备）：Performance 164 MB → MemoryUsage 102 MB。
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            ..Default::default()
        }))
        .map_err(|e| GpuError::Wgpu(format!("请求设备失败: {e}")))?;

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            info,
            selection,
        })
    }
}

/// 自动：交给后端按功耗偏好挑。Dock 常驻，别唤醒独显。
fn pick_auto(instance: &wgpu::Instance) -> Result<wgpu::Adapter, GpuError> {
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .map_err(|e| GpuError::Wgpu(format!("找不到可用适配器: {e}")))
}

/// 按名字挑。
///
/// 先精确（忽略大小写），再退化到互相包含 —— wgpu 有时会在名字后追加
/// 厂商后缀，而用户在列表里选的是 DXGI 的原名。
fn pick_by_name(instance: &wgpu::Instance, wanted: &str) -> Option<wgpu::Adapter> {
    let needle = wanted.to_lowercase();
    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::DX12));

    let exact = adapters
        .iter()
        .position(|a| a.get_info().name.to_lowercase() == needle);
    let index = exact.or_else(|| {
        adapters.iter().position(|a| {
            let name = a.get_info().name.to_lowercase();
            name.contains(&needle) || needle.contains(&name)
        })
    })?;

    adapters.into_iter().nth(index)
}

#[cfg(test)]
mod tests {
    use super::Selection;

    #[test]
    fn selection_describes_itself() {
        assert_eq!(Selection::Auto.describe(), "自动");
        assert!(!Selection::Auto.fell_back());

        let fb = Selection::Fallback {
            wanted: "某个不存在的卡".to_owned(),
            available: vec!["A".to_owned(), "B".to_owned()],
        };
        assert!(fb.fell_back());
        let text = fb.describe();
        assert!(text.contains("某个不存在的卡"));
        assert!(text.contains("A / B"));
    }

    #[test]
    fn descriptor_budget_stays_small() {
        // 4096 是实测拐点：再往下（256/64）不会更省，往上就是白送内存。
        // 改回 wgpu 默认的 1_000_000 会白丢 31 MB（164 vs 133）。
        let budget = super::Gpu::DESCRIPTOR_BUDGET;
        assert!(
            (1024..=65536).contains(&budget),
            "描述符预算 {budget} 不在实测区间内，改之前先重跑 probe"
        );
    }

    #[test]
    fn fallback_lists_at_most_three_candidates() {
        let fb = Selection::Fallback {
            wanted: "X".to_owned(),
            available: vec![
                "A".to_owned(),
                "B".to_owned(),
                "C".to_owned(),
                "D".to_owned(),
                "E".to_owned(),
            ],
        };
        let text = fb.describe();
        assert!(text.contains("A / B / C"));
        assert!(!text.contains('D'), "第四个不应出现: {text}");
        assert!(text.contains("等 5 项"), "总数量要交代清楚: {text}");
    }
}
