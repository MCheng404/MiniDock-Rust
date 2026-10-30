//! Win32 系统服务。
//!
//! 这一层只做"取系统信息 / 调系统 API"，不含任何 Dock 逻辑。
//! 所有函数都返回安全类型（`String`、`Rect`、`bool`、`PathBuf`），
//! `unsafe` 不外泄。

#![deny(missing_debug_implementations)]

pub mod app_window;
pub mod appbar;
pub mod display;
pub mod gpu;
pub mod icon;
pub mod launch;
pub mod occlusion;
pub mod paths;
pub mod shortcut;
pub mod single_instance;
pub mod theme;
pub mod win;

pub use appbar::AppBar;
pub use display::{monitors, system_dpi, work_area, Monitor};
pub use gpu::{AdapterInfo, AdapterKind};
pub use icon::IconBitmap;
pub use paths::Paths;
pub use single_instance::SingleInstance;
