//! GPU 呈现层：wgpu 表面 + egui 绘制 + Win32 输入/命中。
//!
//! 这一层**只做窗口与提交**，不含任何业务逻辑：
//! - 画什么 → `minidock-settings`（可被 kittest 无头测试）
//! - 系统信息 → `minidock-platform`
//! - 配置模型 → `minidock-core`
//!
//! 依赖里**没有 eframe / winit**：消息循环由我们自己持有，因为托盘、
//! 全局热键、AppBar 预留、逐像素命中都需要在 `WndProc` 里说话。
//! 代价是输入翻译（`input.rs`）和 IME（M1）要自己写。

#![deny(missing_debug_implementations)]

pub mod dock;
pub mod gpu;
pub mod input;
pub mod mem;
pub mod offscreen;
pub mod overlay;
pub mod present;

pub use dock::{Dock, DockItem};
pub use gpu::{Gpu, GpuError, Selection};
pub use mem::MemStats;
pub use offscreen::Offscreen;
pub use overlay::Overlay;
pub use present::LayeredPresenter;
