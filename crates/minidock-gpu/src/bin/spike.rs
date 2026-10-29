//! M0 尖刺：透明 GPU 窗口 + egui 设置界面 + 逐像素穿透。
//!
//! 验证四件事（对应 PORTING.md 的 M0 验收）：
//! 1. D3D12 交换链能否带 alpha（启动日志里 `透明=true`）
//! 2. 面板外的点击是否穿透到下面的窗口
//! 3. egui 中文界面能否正常绘制
//! 4. Esc 退出、空闲时不重绘

use minidock_core::config::Config;
use minidock_core::ini::Ini;
use minidock_gpu::Overlay;
use minidock_platform::gpu::adapters;
use minidock_platform::single_instance::{find_window, SingleInstance};
use minidock_platform::{display, Paths};
use minidock_settings::SettingsState;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 单实例。之前漏了这一步：双开会叠出两个分层窗口，两个进程各自重绘，
    // 外部看起来就是"空闲还占 8% CPU"，排查了半天才发现是两份在跑。
    //
    // 互斥体失败**不阻止启动**：宁可多一份 Dock，也不要起不来。
    let _guard = match SingleInstance::acquire() {
        Ok(Some(guard)) => Some(guard),
        Ok(None) => {
            if let Some(hwnd) = find_window(minidock_gpu::overlay::CLASS_NAME) {
                let _ =
                    unsafe { windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow(hwnd) };
            }
            eprintln!("MiniDock 已在运行，本次启动退出。");
            return Ok(());
        }
        Err(e) => {
            eprintln!("单实例互斥体失败：{e}（继续启动）");
            None
        }
    };

    let paths = Paths::resolve()?;
    let dpi = display::system_dpi();
    let ini = Ini::load(&paths.ini)?;
    let cfg = Config::from_ini(&ini, dpi);

    // 列表由 platform 层给（DXGI 枚举），选择与回退由 GPU 层做。
    // 软件光栅器不进下拉框：选它只会得到一帧几十毫秒的结果。
    let adapters: Vec<String> = adapters()
        .into_iter()
        .filter(|a| a.kind != minidock_platform::AdapterKind::Software)
        .map(|a| a.name)
        .collect();
    let monitors = display::monitors();
    let settings = SettingsState {
        config_dir: paths.dir.display().to_string(),
        monitors: monitors
            .iter()
            .enumerate()
            .map(|(i, _)| {
                if i == 0 {
                    "主显示器".to_owned()
                } else {
                    format!("显示器 {}", i + 1)
                }
            })
            .collect(),
        status: format!("{} 个显示器 · DPI {dpi}", monitors.len()),
        adapters,
        ..Default::default()
    };

    eprintln!("配置目录: {}", paths.dir.display());
    let overlay = Overlay::open(880.0, 720.0, cfg, settings)?;
    overlay.run()?;
    Ok(())
}
