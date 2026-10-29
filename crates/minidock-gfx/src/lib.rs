//! GDI+ 的 RAII 封装 + 分层窗口画布。
//!
//! # 为什么单独一个 crate
//!
//! v2.0.0 的崩溃（`gdiplus.dll!GdipDrawLine+0x9C`，0xC0000005）不是参数校验
//! 问题 —— 实测传 NULL 只会返回 `InvalidParameter`。真正的原因是
//! **`GpGraphics` / `GpPen` 已经失效却还在用**：跨帧缓存了 graphics，surface
//! 重建后没重新取；或者 pen 删了没置空。这类 bug 在 C 里靠 review 抓不住，
//! 在 Rust 里可以靠**类型系统**直接消灭：
//!
//! - [`Graphics<'a>`] 带生命周期参数，借用它来源的 [`Bitmap`]；
//!   surface 一换，旧 graphics 就借不到，编译期报错。
//! - 每个对象都是 newtype + `Drop`，不存在"删了没置空"的中间态。
//! - 构造函数返回 [`Result`]，GDI+ 的 `Status` 不再被静默忽略。
//!
//! # 刻意不做的事
//!
//! 不做对象池 / 全局缓存。原版的 `g_font` / `g_attr` 这类跨帧缓存是性能
//! 优化，也确实有用 —— 但那是**第二阶段**的事，且必须由 `minidock-ui`
//! 显式持有（生命周期与窗口一致），而不是藏在 gfx 内部的全局变量里。
//! 先保证正确，再谈省几次 `GdipCreateSolidFill`。

#![deny(missing_debug_implementations)]

pub mod canvas;
pub mod objects;

pub use canvas::Canvas;
pub use objects::{
    Bitmap, Brush, Font, FontFamily, GdiPlus, Graphics, ImageAttributes, Path, Pen, RectF,
    StringFormat,
};

use std::fmt;

/// GDI+ 的 `Status`。`Ok == 0`。
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct GpStatus(pub i32);

impl GpStatus {
    #[must_use]
    pub const fn is_ok(self) -> bool {
        self.0 == 0
    }
}

impl fmt::Debug for GpStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GpStatus({})", self.0)
    }
}

impl fmt::Display for GpStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 常见状态码的语义，便于从日志直接定位是"参数非法"还是"内存不足"。
        let name = match self.0 {
            0 => "Ok",
            1 => "GenericError",
            2 => "InvalidParameter",
            3 => "OutOfMemory",
            4 => "ObjectBusy",
            5 => "InsufficientBuffer",
            6 => "NotImplemented",
            7 => "Win32Error",
            8 => "WrongState",
            9 => "Aborted",
            10 => "FileNotFound",
            11 => "ValueOverflow",
            12 => "AccessDenied",
            13 => "UnknownImageFormat",
            14 => "FontFamilyNotFound",
            15 => "FontStyleNotFound",
            16 => "NotTrueTypeFont",
            17 => "UnsupportedGdiplusVersion",
            18 => "GdiplusNotInitialized",
            19 => "PropertyNotFound",
            20 => "PropertyNotSupported",
            _ => "Unrecognized",
        };
        write!(f, "GDI+ {name} ({})", self.0)
    }
}

/// gfx 层的错误。只有两种：GDI+ 返回非 Ok，或 GDI 句柄创建失败。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GfxError {
    /// GDI+ 调用失败。
    Gp(GpStatus),
    /// Win32 GDI 调用失败（DIB / DC / 位图）。
    Win32(&'static str),
    /// 尺寸非法（宽或高 <= 0）。原版会在这种输入下创建 0 尺寸 surface，
    /// 之后 `GdipGetImageGraphicsContext` 返回的对象一画就崩。
    BadSize { width: i32, height: i32 },
}

impl fmt::Display for GfxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gp(s) => write!(f, "{s}"),
            Self::Win32(what) => write!(f, "Win32 调用失败: {what}"),
            Self::BadSize { width, height } => {
                write!(f, "非法画布尺寸 {width}x{height}")
            }
        }
    }
}

impl std::error::Error for GfxError {}

impl From<GpStatus> for GfxError {
    fn from(s: GpStatus) -> Self {
        Self::Gp(s)
    }
}

pub type Result<T> = std::result::Result<T, GfxError>;

/// 检查 GDI+ 返回值。
pub(crate) fn check(status: GpStatus) -> Result<()> {
    if status.is_ok() {
        Ok(())
    } else {
        Err(GfxError::Gp(status))
    }
}

/// `&str` → UTF-16 + NUL。GDI+ 要 `PCWSTR`，长度传 -1 表示按 NUL 截断。
pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
