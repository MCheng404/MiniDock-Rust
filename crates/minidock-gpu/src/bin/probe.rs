//! 设备内存探针。
//!
//! 回答一个具体问题：**只创建一个图形设备（不建窗口、不跑 egui、不画一帧），
//! 进程要付多少内存？** 这个数字决定了 GPU 路线的"轻量"上限，也决定了
//! 换后端到底能省多少。
//!
//! 用法：
//!
//! ```text
//! PROBE_BACKEND=d3d11 PROBE_ALLOC=1 cargo run --release -p minidock-gpu --bin probe
//! PROBE_LIST=1  target/release/probe.exe     # 先看这台机器上有哪些适配器
//! ```
//!
//! | 环境变量 | 取值 | 说明 |
//! |---|---|---|
//! | `PROBE_BACKEND` | `dx12`（默认）/ `vulkan` / `wgl` / `d3d11` / `d3d11-warp` / `gl` | 图形后端 |
//! | `PROBE_ADAPTER` | 名字片段，如 `AMD` / `NVIDIA` | **钉住同一块显卡**；空=后端默认 |
//! | `PROBE_ALLOC` | `1` / `0` | 是否额外分配与真实应用等量的缓冲 |
//! | `PROBE_HOLD` | 秒数，默认 45 | 采样窗口 |
//! | `PROBE_LIST` | `1` | 只枚举适配器后退出 |
//! | `PROBE_NON_SAMPLER` | 整数，默认 `1000000` | `max_non_sampler_bindings`，只影响 DX12 |
//! | `PROBE_HINTS` | `perf`（默认）/ `memory` | `MemoryHints`：分配器留不留余量 |
//! | `PROBE_LIMITS` | `downlevel`（默认）/ `default` / `wide` / `webgl2` | 限制档位 |
//!
//! `wgl` 走 **wgpu 自带的 GL 后端**（`wgpu-hal/gles/wgl.rs`，Windows 上不需要
//! ANGLE），用来回答"保留 egui-wgpu 只换后端能省多少"；`gl` 是手写 WGL
//! 上下文，代表"完全不用 wgpu"的下界。
//!
//! `PROBE_ALLOC=1` 分配的三块，与 `offscreen.rs` + `present.rs` 完全一致：
//! 1320×1080 渲染目标（8.4 MB）+ 同尺寸回读缓冲（5.4 MB）+ 同尺寸 DIB（5.4 MB）。
//! 这样"设备 + 缓冲"就是真实应用的内存底盘，可以直接和 spike 的实测值对账。
//!
//! **必须钉住适配器再比较后端。** 混合显卡笔记本上 wgpu 与 D3D11 的默认
//! 选择可能落在不同 GPU 上（实测：wgpu 选 iGPU、D3D11 选独显），
//! 那样 42 MB vs 164 MB 的差距里有一半是"换了块显卡"，不是后端差异。
//!
//! `d3d11-warp` 是纯软件光栅器，代表"完全没有 GPU 加速时的最小开销"，
//! 作为 D3D11 基线的下界参考。

use std::time::Duration;

use minidock_gpu::mem::{Delta, MemStats};
use minidock_gpu::present::LayeredPresenter;

/// 渲染目标的物理尺寸。与 spike 的窗口客户区一致（880×720 逻辑 @150%）。
const TEX_W: u32 = 1320;
const TEX_H: u32 = 1080;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if env_or("PROBE_LIST", "0") == "1" {
        list_adapters();
        return Ok(());
    }

    let backend = env_or("PROBE_BACKEND", "dx12");
    let want = env_or("PROBE_ADAPTER", "");
    let alloc = env_or("PROBE_ALLOC", "0") == "1";
    let hold: u64 = env_or("PROBE_HOLD", "45").parse().unwrap_or(45);

    println!(
        "[probe] 后端={backend} 限定适配器={} 附加缓冲={} 采样窗口={hold}s",
        if want.is_empty() {
            "<后端默认>"
        } else {
            &want
        },
        if alloc { "是" } else { "否" }
    );

    let start = MemStats::now();
    println!("[probe] ① 进程启动   {}", start.summary());

    let (device, desc) = create_device(&backend, &want)?;
    let after_device = MemStats::now();
    println!("[probe] ② 设备就绪   {desc}");
    println!(
        "[probe]    设备开销   {}",
        Delta::between(start, after_device).summary()
    );

    // 必须活到进程结束：提前释放等于没测。
    let mut keep = Guard::default();
    if alloc {
        let before = MemStats::now();
        keep = allocate(&device)?;
        // 驱动常常异步提交显存，等一拍再采样才不会把增量算到下一步。
        std::thread::sleep(Duration::from_millis(1500));
        let after = MemStats::now();
        println!("[probe] ③ 缓冲就绪   {}", after.summary());
        println!(
            "[probe]    缓冲开销   {}",
            Delta::between(before, after).summary()
        );
    }

    let steady = MemStats::now();
    println!("[probe] ④ 稳态       {}", steady.summary());
    // 机器可读的一行，供脚本与文档引用；不要在中间插入别的字段。
    println!(
        "[probe] RESULT backend={backend} alloc={} ws_mb={:.1} private_mb={:.1} peak_ws_mb={:.1}",
        u8::from(alloc),
        steady.ws_mb(),
        steady.private_mb(),
        steady.peak_ws_mb()
    );

    // 采样窗口内每 5 秒重打一次：若数字仍在爬升，说明驱动还在提交，
    // 单点采样会低估。结束时的峰值才是判断"会不会爆"的依据。
    let mut left = hold;
    while left > 0 {
        let step = left.min(5);
        std::thread::sleep(Duration::from_secs(step));
        left -= step;
        println!(
            "[probe]    t+{}s -> {}",
            hold - left,
            MemStats::now().summary()
        );
    }

    println!(
        "[probe] ⑤ 结束       峰值 {:.1} MB",
        MemStats::now().peak_ws_mb()
    );
    drop(keep);
    Ok(())
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

/// 枚举两套 API 看到的适配器。混合显卡机器上先看这个，再决定钉哪块。
fn list_adapters() {
    let instance = wgpu_instance(egui_wgpu::wgpu::Backends::all());
    println!("[adapter] —— wgpu ——");
    for adapter in pollster::block_on(instance.enumerate_adapters(egui_wgpu::wgpu::Backends::all()))
    {
        let info = adapter.get_info();
        println!(
            "[adapter] wgpu {:?} | {} | VEN {:04X} DEV {:04X} | {:?} | driver {}",
            info.backend, info.name, info.vendor, info.device, info.device_type, info.driver
        );
    }
    println!("[adapter] —— DXGI（D3D11 用）——");
    match d3d11_device::adapters() {
        Ok(rows) => {
            for row in &rows {
                println!(
                    "[adapter] dxgi #{} | {} | VEN {:04X} DEV {:04X} | 专用显存 {} MB",
                    row.index, row.name, row.vendor, row.device_id, row.dedicated_mb
                );
            }
        }
        Err(e) => println!("[adapter] DXGI 枚举失败: {e}"),
    }
}

/// 设备与队列/上下文。两种后端分开存，因为句柄类型毫无共同点。
enum Device {
    Wgpu(wgpu_device::Device),
    D3d11(d3d11_device::Device),
    Gl(gl_device::Device),
}

/// 保持资源存活。用 `Box<dyn Any>` 是因为这里唯一的用途就是"别被 drop"。
#[derive(Default)]
struct Guard(Vec<Box<dyn std::any::Any>>);

impl Guard {
    fn push<T: 'static>(&mut self, value: T) {
        self.0.push(Box::new(value));
    }
}

fn create_device(
    backend: &str,
    want: &str,
) -> Result<(Device, String), Box<dyn std::error::Error>> {
    match backend {
        "dx12" | "vulkan" | "wgl" => wgpu_device::create(backend, want),
        "d3d11" | "d3d11-warp" => d3d11_device::create(backend == "d3d11-warp", want),
        "gl" => gl_device::create(),
        other => {
            Err(format!("未知后端 `{other}`，可选 dx12 / vulkan / d3d11 / d3d11-warp / gl").into())
        }
    }
}

fn allocate(device: &Device) -> Result<Guard, Box<dyn std::error::Error>> {
    let mut keep = Guard::default();
    match device {
        Device::Wgpu(d) => d.allocate(&mut keep),
        Device::D3d11(d) => d.allocate(&mut keep)?,
        Device::Gl(d) => d.allocate(&mut keep),
    }
    // 呈现路径的 DIB：复用真实代码，避免"探针测的和应用用的不是一个东西"。
    keep.push(LayeredPresenter::new(TEX_W as i32, TEX_H as i32)?);
    Ok(keep)
}

fn wgpu_instance(backends: egui_wgpu::wgpu::Backends) -> egui_wgpu::wgpu::Instance {
    egui_wgpu::wgpu::Instance::new(egui_wgpu::wgpu::InstanceDescriptor {
        backends,
        flags: egui_wgpu::wgpu::InstanceFlags::default(),
        memory_budget_thresholds: egui_wgpu::wgpu::MemoryBudgetThresholds::default(),
        backend_options: egui_wgpu::wgpu::BackendOptions::default(),
        display: None,
    })
}

/// wgpu 侧：`dx12` 与 `vulkan` 共用同一套创建逻辑，只是后端位不同。
mod wgpu_device {
    use egui_wgpu::wgpu;

    use super::{Guard, TEX_H, TEX_W};

    pub struct Device {
        pub device: wgpu::Device,
        /// 队列不参与绘制，但要留着：丢队列会让设备进入空闲态，
        /// 驱动可能顺手回收一部分显存，测出来的数字就偏小。
        #[allow(dead_code)]
        pub queue: wgpu::Queue,
    }

    impl Device {
        pub fn allocate(&self, keep: &mut Guard) {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("probe-rt"),
                size: wgpu::Extent3d {
                    width: TEX_W,
                    height: TEX_H,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                // 与 `Gpu::COLOR_FORMAT` 保持一致，否则测的不是同一条管线。
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            keep.push(texture);

            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("probe-readback"),
                size: u64::from(TEX_W) * u64::from(TEX_H) * 4,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            keep.push(buffer);
        }
    }

    pub fn create(
        which: &str,
        want: &str,
    ) -> Result<(super::Device, String), Box<dyn std::error::Error>> {
        let backends = match which {
            "vulkan" => wgpu::Backends::VULKAN,
            // wgpu 自带 WGL 实现（wgpu-hal/gles/wgl.rs），Windows 上不需要 ANGLE。
            "wgl" => wgpu::Backends::GL,
            _ => wgpu::Backends::DX12,
        };
        let instance = super::wgpu_instance(backends);

        // 钉适配器时不能走 `request_adapter`：它的选择权在驱动手里，
        // 混合显卡上会挑 iGPU，和 D3D11 的默认选择根本不是同一块芯片。
        let adapter = if want.is_empty() {
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            }))?
        } else {
            let needle = want.to_lowercase();
            let all = pollster::block_on(instance.enumerate_adapters(backends));
            let names: Vec<String> = all.iter().map(|a| a.get_info().name).collect();
            all.into_iter()
                .find(|a| {
                    let info = a.get_info();
                    // 按名字匹配；`driver` 兜底，因为部分后端把厂商名只写在驱动串里。
                    info.name.to_lowercase().contains(&needle)
                        || info.driver.to_lowercase().contains(&needle)
                })
                .ok_or_else(|| {
                    format!(
                        "没有匹配 `{want}` 的 wgpu 适配器，可选: {}",
                        names.join(" / ")
                    )
                })?
        };

        let info = adapter.get_info();
        // 两个开关对应 wgpu 官方文档点名的两个"能吃内存"的地方：
        // - `max_non_sampler_bindings`：DX12 后端按它预分配全局描述符堆，
        //   默认 1_000_000，核显上直接吃系统内存（wgpu 文档原话）。
        // - `memory_hints`：默认 Performance 会让分配器多留块。
        let non_sampler: u32 = std::env::var("PROBE_NON_SAMPLER")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1_000_000);
        let hints = if std::env::var("PROBE_HINTS").as_deref() == Ok("memory") {
            wgpu::MemoryHints::MemoryUsage
        } else {
            wgpu::MemoryHints::Performance
        };
        // PROBE_LIMITS=webgl2 用最激进的档位（纹理上限 2048 等），
        // 用来判断"限制项本身"还能不能继续榨出内存。
        let which_limits = std::env::var("PROBE_LIMITS").unwrap_or_default();
        let limits = if which_limits == "default" {
            // WebGPU 标准默认：纹理 8192，但描述符预算仍是 1M。
            wgpu::Limits {
                max_non_sampler_bindings: non_sampler,
                ..wgpu::Limits::default()
            }
        } else if which_limits == "wide" {
            // downlevel + 把纹理上限抬到 8192（超宽多屏 Dock 需要）。
            wgpu::Limits {
                max_non_sampler_bindings: non_sampler,
                max_texture_dimension_2d: 8192,
                ..wgpu::Limits::downlevel_defaults()
            }
        } else if std::env::var("PROBE_LIMITS").as_deref() == Ok("webgl2") {
            wgpu::Limits {
                max_non_sampler_bindings: non_sampler,
                ..wgpu::Limits::downlevel_webgl2_defaults()
            }
        } else {
            wgpu::Limits {
                max_non_sampler_bindings: non_sampler,
                ..wgpu::Limits::downlevel_defaults()
            }
        };
        eprintln!("[probe]    非采样绑定上限={non_sampler} hints={hints:?}");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("probe"),
                required_limits: limits,
                memory_hints: hints,
                ..Default::default()
            }))?;
        let desc = format!(
            "{:?} / {} / {:?} / driver {}",
            info.backend, info.name, info.device_type, info.driver
        );
        Ok((super::Device::Wgpu(Device { device, queue }), desc))
    }
}

/// D3D11 侧：直接调 `D3D11CreateDevice`，不经 wgpu（wgpu 没有 D3D11 后端）。
mod d3d11_device {
    use windows::core::Interface;
    use windows::Win32::Graphics::Direct3D::{
        D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL,
        D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
    };
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
        D3D11_BIND_RENDER_TARGET, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
    };
    use windows::Win32::Graphics::Dxgi::Common::{
        DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, DXGI_SAMPLE_DESC,
    };
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory1};

    use super::{Guard, TEX_H, TEX_W};

    /// 一条 DXGI 适配器记录。
    pub struct Row {
        pub index: u32,
        pub name: String,
        pub vendor: u32,
        pub device_id: u32,
        pub dedicated_mb: usize,
        pub adapter: IDXGIAdapter,
    }

    pub struct Device {
        pub device: ID3D11Device,
        /// 立即上下文必须留着：D3D11 的设备与上下文是一对，
        /// 只留设备会让驱动认为上下文已废弃。
        #[allow(dead_code)]
        pub context: ID3D11DeviceContext,
    }

    impl Device {
        pub fn allocate(&self, keep: &mut Guard) -> Result<(), Box<dyn std::error::Error>> {
            let rt = D3D11_TEXTURE2D_DESC {
                Width: TEX_W,
                Height: TEX_H,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let mut texture: Option<ID3D11Texture2D> = None;
            unsafe { self.device.CreateTexture2D(&rt, None, Some(&mut texture))? };
            keep.push(texture);

            // 回读用的 staging 纹理：D3D11 里 Map 只能读 staging 资源。
            let staging = D3D11_TEXTURE2D_DESC {
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                ..rt
            };
            let mut readback: Option<ID3D11Texture2D> = None;
            unsafe {
                self.device
                    .CreateTexture2D(&staging, None, Some(&mut readback))?
            };
            keep.push(readback);
            Ok(())
        }
    }

    /// 枚举系统的 DXGI 适配器。
    pub fn adapters() -> Result<Vec<Row>, Box<dyn std::error::Error>> {
        let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1()? };
        let mut rows = Vec::new();
        // 走到 `DXGI_ERROR_NOT_FOUND` 为止；枚举不提供总数。
        for index in 0..16u32 {
            let Ok(adapter1) = (unsafe { factory.EnumAdapters1(index) }) else {
                break;
            };
            let Ok(desc) = (unsafe { adapter1.GetDesc1() }) else {
                break;
            };
            let end = desc
                .Description
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(desc.Description.len());
            rows.push(Row {
                index,
                name: String::from_utf16_lossy(&desc.Description[..end]),
                vendor: desc.VendorId,
                device_id: desc.DeviceId,
                dedicated_mb: desc.DedicatedVideoMemory / (1024 * 1024),
                adapter: adapter1.cast()?,
            });
        }
        Ok(rows)
    }

    pub fn create(
        warp: bool,
        want: &str,
    ) -> Result<(super::Device, String), Box<dyn std::error::Error>> {
        // 显式指定适配器时，驱动类型必须是 UNKNOWN：
        // 传 HARDWARE 会被 D3D11 判为参数非法（E_INVALIDARG）。
        let (driver, adapter) = if warp {
            (D3D_DRIVER_TYPE_WARP, None)
        } else if want.is_empty() {
            (D3D_DRIVER_TYPE_HARDWARE, None)
        } else {
            let needle = want.to_lowercase();
            let rows = adapters()?;
            let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
            let row = rows
                .iter()
                .find(|r| r.name.to_lowercase().contains(&needle))
                .ok_or_else(|| {
                    format!(
                        "没有匹配 `{want}` 的 DXGI 适配器，可选: {}",
                        names.join(" / ")
                    )
                })?;
            (D3D_DRIVER_TYPE_UNKNOWN, Some(row.adapter.clone()))
        };

        let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        let mut level = D3D_FEATURE_LEVEL::default();
        unsafe {
            D3D11CreateDevice(
                adapter.as_ref(),
                driver,
                // 不走 D3D11 的软件光栅器旁路（WARP 由 driver 参数决定）。
                Default::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                Some(&mut level),
                Some(&mut context),
            )?;
        }
        let device = device.ok_or("D3D11CreateDevice 未返回设备")?;
        let context = context.ok_or("D3D11CreateDevice 未返回上下文")?;

        let desc = format!(
            "D3D11{} / feature level 0x{:x} / {}",
            if warp { " (WARP)" } else { "" },
            level.0,
            adapter_of(&device)
        );
        Ok((super::Device::D3d11(Device { device, context }), desc))
    }

    /// 取实际生效的适配器名字。拿不到不算错误 —— 探针的首要任务是量内存。
    fn adapter_of(device: &ID3D11Device) -> String {
        let Ok(dxgi) = device.cast::<windows::Win32::Graphics::Dxgi::IDXGIDevice>() else {
            return "未知".to_owned();
        };
        let Ok(adapter) = (unsafe { dxgi.GetAdapter() }) else {
            return "未知".to_owned();
        };
        let Ok(desc) = (unsafe { adapter.GetDesc() }) else {
            return "未知".to_owned();
        };
        let end = desc
            .Description
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(desc.Description.len());
        let name = String::from_utf16_lossy(&desc.Description[..end]);
        format!(
            "{name} (VEN {:04X} DEV {:04X}, 专用显存 {} MB)",
            desc.VendorId,
            desc.DeviceId,
            desc.DedicatedVideoMemory / (1024 * 1024)
        )
    }
}

/// OpenGL 侧：用 WGL 在隐藏窗口上建一个兼容上下文。
///
/// 为什么专门测 GL：`egui_glow` 是 egui 的**官方**后端，与 `egui-wgpu` 同级，
/// 且不引入 D3D12。如果 GL 驱动的基线明显低于 D3D12，就能"保住 egui 与
/// kittest，同时砍掉一百多 MB" —— 那比换框架划算得多。
///
/// 只用到 GL 1.1（opengl32.dll 直接导出），不加载任何扩展，
/// 这样测出来的就是驱动的最小上下文成本，而不是某个加载器的开销。
mod gl_device {
    use windows::core::w;
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::Graphics::Gdi::{GetDC, HDC};
    use windows::Win32::Graphics::OpenGL::{
        wglCreateContext, wglMakeCurrent, ChoosePixelFormat, SetPixelFormat, HGLRC,
        PFD_DRAW_TO_WINDOW, PFD_SUPPORT_OPENGL, PFD_TYPE_RGBA, PIXELFORMATDESCRIPTOR,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, RegisterClassW, CS_OWNDC, WINDOW_EX_STYLE, WNDCLASSW,
        WS_POPUP,
    };

    use super::{Guard, TEX_H, TEX_W};

    const GL_TEXTURE_2D: u32 = 0x0DE1;
    const GL_RGBA: u32 = 0x1908;
    const GL_RGBA8: i32 = 0x8058;
    const GL_UNSIGNED_BYTE: u32 = 0x1401;
    const GL_RENDERER: u32 = 0x1F01;
    const GL_VERSION: u32 = 0x1F02;

    // opengl32.dll 只直接导出 GL 1.1；这里不碰任何扩展。
    #[link(name = "opengl32")]
    unsafe extern "system" {
        fn glGetString(name: u32) -> *const u8;
        fn glGenTextures(n: i32, textures: *mut u32);
        fn glBindTexture(target: u32, texture: u32);
        fn glTexImage2D(
            target: u32,
            level: i32,
            internalformat: i32,
            width: i32,
            height: i32,
            border: i32,
            format: u32,
            ty: u32,
            pixels: *const std::ffi::c_void,
        );
    }

    pub struct Device {
        pub _hwnd: HWND,
        pub _hdc: HDC,
        pub _hglrc: HGLRC,
    }

    impl Device {
        pub fn allocate(&self, keep: &mut Guard) {
            let mut texture = 0u32;
            unsafe {
                glGenTextures(1, &mut texture);
                glBindTexture(GL_TEXTURE_2D, texture);
                // 传空指针：驱动只分配存储，不做上传。真实应用也是先建后画。
                glTexImage2D(
                    GL_TEXTURE_2D,
                    0,
                    GL_RGBA8,
                    TEX_W as i32,
                    TEX_H as i32,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    std::ptr::null(),
                );
            }
            keep.push(texture);
            // 回读目标。真实路径用 PBO（需要 GL 1.5 扩展加载），
            // 这里用等量堆内存代替 —— 对"多占多少内存"这个问题等价。
            keep.push(vec![0u8; (TEX_W as usize) * (TEX_H as usize) * 4]);
        }
    }

    /// 窗口过程。`DefWindowProcW` 在 `windows` crate 里是安全的 Rust 包装，
    /// 不能直接当 `WNDPROC` 函数指针用，必须自己套一层。
    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    pub fn create() -> Result<(super::Device, String), Box<dyn std::error::Error>> {
        let instance = unsafe { GetModuleHandleW(None)? };
        let class = w!("MiniDockProbeGL");
        let wc = WNDCLASSW {
            // CS_OWNDC：像素格式绑定到窗口的 DC，必须独立。
            style: CS_OWNDC,
            lpfnWndProc: Some(wndproc),
            hInstance: HINSTANCE(instance.0),
            lpszClassName: class,
            ..Default::default()
        };
        // 重复注册同类名不算致命错误（多后端连续跑时会出现）。
        unsafe { RegisterClassW(&wc) };

        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                w!("probe"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(instance.into()),
                None,
            )?
        };
        let hdc = unsafe { GetDC(Some(hwnd)) };

        let pfd = PIXELFORMATDESCRIPTOR {
            nSize: std::mem::size_of::<PIXELFORMATDESCRIPTOR>() as u16,
            nVersion: 1,
            dwFlags: PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL,
            iPixelType: PFD_TYPE_RGBA,
            cColorBits: 32,
            cAlphaBits: 8,
            ..Default::default()
        };
        let format = unsafe { ChoosePixelFormat(hdc, &pfd) };
        if format == 0 {
            return Err("ChoosePixelFormat 失败：没有可用的 GL 像素格式".into());
        }
        unsafe { SetPixelFormat(hdc, format, &pfd)? };

        let hglrc = unsafe { wglCreateContext(hdc)? };
        unsafe { wglMakeCurrent(hdc, hglrc)? };

        let desc = format!(
            "OpenGL / {} / GL {}",
            gl_string(GL_RENDERER).unwrap_or_else(|| "未知渲染器".to_owned()),
            gl_string(GL_VERSION).unwrap_or_else(|| "?".to_owned())
        );
        Ok((
            super::Device::Gl(Device {
                _hwnd: hwnd,
                _hdc: hdc,
                _hglrc: hglrc,
            }),
            desc,
        ))
    }

    /// 读 GL 的字符串常量。上下文未生效时返回 `None`。
    fn gl_string(name: u32) -> Option<String> {
        let ptr = unsafe { glGetString(name) };
        if ptr.is_null() {
            return None;
        }
        Some(
            unsafe { std::ffi::CStr::from_ptr(ptr.cast()) }
                .to_string_lossy()
                .into_owned(),
        )
    }
}
