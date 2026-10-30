//! MiniDock 的纯逻辑层。
//!
//! 这里**不允许**出现任何 Win32 / GDI+ 依赖。原因不是洁癖，而是验证成本：
//! 原版 C 代码 4799 行里，真正容易出错的是配置解析、条目序列化、布局数学
//! 这三块，它们全部与图形无关。把它们留在无 Win32 依赖的 crate 里，才能在
//! 秒级反馈回路里跑单元测试；而窗口过程、GDI+ 资源这类只能靠"跑起来看"的
//! 部分，则集中在 `minidock-ui` / `minidock-gfx` 里，靠手动验收。
//!
//! 模块划分与 C 版的对应关系见 `PORTING.md`。

#![deny(missing_debug_implementations)]

pub mod clock;
pub mod color;
pub mod config;
pub mod icon;
pub mod ini;
pub mod item;
pub mod layout;
pub mod running;
pub mod store;
pub mod units;

/// 进程级常量。版本号单独维护：它与 `MiniDock.ini` 的兼容性有关，
/// 不跟着 crate 版本走（crate 版本可能因为内部重构跳动）。
pub const APP_NAME: &str = "MiniDock";
pub const APP_VERSION: &str = "1.2.0";
/// 单实例互斥体名（`Local\` 前缀 = 每个登录会话一个实例，与 C 版一致）。
pub const SINGLETON_MUTEX: &str = "Local\\MiniDock_Wangcy_Singleton";

/// 条目上限。这两个数字**必须**与 ini 解析、UI 循环、拖拽索引三处一致，
/// 因此只在 core 里定义一次。原版是 `MAXI` / `MAXK` 宏。
pub const MAX_ITEMS: usize = 128;
pub const MAX_KIDS: usize = 16;
