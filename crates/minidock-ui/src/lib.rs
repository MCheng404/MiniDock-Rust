//! Dock 窗口层：渲染、输入、动画状态机。
//!
//! 与 `minidock-gfx` 的分工：gfx 只管"怎么画"，ui 管"画什么、什么时候画"。
//! 与 `minidock-platform` 的分工：platform 只管"问系统要东西"，不含任何
//! Dock 语义（比如平台层不知道什么叫 `Item`）。

#![deny(missing_debug_implementations)]

pub mod dock;

pub use dock::Dock;
