//! 设置界面层。
//!
//! 这一层**只有 UI 逻辑**：没有 wgpu、没有 Win32、没有线程。
//! 它是本路线里"可测试性"的全部来源 —— kittest 直接驱动这些控件，
//! 断言 `Config` 的变化，而不需要开窗口。
//!
//! 分工：
//! - `minidock-core`：配置模型、键表、取值域、INI 序列化（已全部有单测）
//! - `minidock-settings`（本 crate）：把键表渲染成控件，产出 `Command`
//! - `minidock-gpu`：窗口、GPU 提交、输入与命中
//! - `minidock-platform`：路径、显示器、注册表、托盘

#![deny(missing_debug_implementations)]

pub mod fonts;
pub mod pages;

pub use pages::{settings_ui, Command, Outcome, Page, SettingsState};
