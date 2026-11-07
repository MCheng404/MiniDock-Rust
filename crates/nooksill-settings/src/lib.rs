//! 设置界面层。
//!
//! 这一层**只有 UI 逻辑**：没有 wgpu、没有 Win32、没有线程。
//! 它是本路线里"可测试性"的全部来源 —— kittest 直接驱动这些控件，
//! 断言 `Config` 的变化，而不需要开窗口。
//!
//! 分工：
//! - `nooksill-core`：配置模型、键表、取值域、INI 序列化（已全部有单测）
//! - `nooksill-settings`（本 crate）：把键表渲染成控件，产出 `Command`
//! - `nooksill-gpu`：窗口、GPU 提交、输入与命中
//! - `nooksill-platform`：路径、显示器、注册表、托盘

#![deny(missing_debug_implementations)]

pub mod fonts;
pub mod pages;
pub mod text_field;

pub use pages::{settings_ui, Command, ItemEdit, Outcome, Page, SettingsState, UpdateView};

/// 左侧导航栏宽度（逻辑像素）。**写死**，不随标签内容变化。
///
/// 公开给窗口层：顶栏的品牌区要占同宽的一格并把文字居中，和下方导航项
/// 对齐；两处必须读同一个数字，否则改宽度就会错位。
pub const NAV_W: f32 = 193.0;
