//! GDI+ 对象的 RAII newtype。
//!
//! 每个类型都只有唯一所有者、`Drop` 时释放、不可 `Clone`。
//! 需要"共享"的（字体、格式）由上层持有 `&`，不复制句柄。

use crate::{check, wide, GfxError, GpStatus, Result};
use std::marker::PhantomData;
use std::ptr::NonNull;
use windows::core::PCWSTR;

/// GDI+ 的浮点矩形。直接复用系统定义，避免两套类型来回转换。
pub use windows::Win32::Graphics::GdiPlus::RectF;
use windows::Win32::Graphics::GdiPlus::{
    ColorAdjustTypeBitmap, ColorMatrix, ColorMatrixFlagsDefault, CombineModeReplace,
    FillModeAlternate, GdipAddPathArc, GdipClosePathFigure, GdipCreateBitmapFromScan0,
    GdipCreateFont, GdipCreateFontFamilyFromName, GdipCreateImageAttributes,
    GdipCreateLineBrushFromRect, GdipCreatePath, GdipCreatePen1, GdipCreateSolidFill,
    GdipCreateStringFormat, GdipDeleteBrush, GdipDeleteFont, GdipDeleteFontFamily,
    GdipDeleteGraphics, GdipDeletePath, GdipDeletePen, GdipDeleteStringFormat, GdipDisposeImage,
    GdipDisposeImageAttributes, GdipDrawImageRectRect, GdipDrawLine, GdipDrawPath, GdipDrawString,
    GdipFillPath, GdipFillRectangle, GdipGetGenericFontFamilySansSerif,
    GdipGetImageGraphicsContext, GdipGraphicsClear, GdipMeasureString, GdipResetClip,
    GdipResetWorldTransform, GdipSetClipPath, GdipSetImageAttributesColorMatrix,
    GdipSetImageAttributesWrapMode, GdipSetInterpolationMode, GdipSetPixelOffsetMode,
    GdipSetSmoothingMode, GdipSetStringFormatAlign, GdipSetStringFormatLineAlign,
    GdipSetStringFormatTrimming, GdipSetTextRenderingHint, GdipTranslateWorldTransform,
    GdiplusShutdown, GdiplusStartup, GdiplusStartupInput, GpBitmap, GpBrush, GpFont, GpFontFamily,
    GpGraphics, GpImage, GpImageAttributes, GpLineGradient, GpPath, GpPen, GpSolidFill,
    GpStringFormat, InterpolationModeHighQualityBicubic, LinearGradientMode, MatrixOrderPrepend,
    PixelOffsetModeHalf, SmoothingModeAntiAlias, StringAlignment, StringAlignmentCenter,
    StringAlignmentFar, StringTrimming, StringTrimmingEllipsisCharacter,
    TextRenderingHintAntiAlias, UnitPixel, WrapModeClamp, WrapModeTile,
};

/// `PixelFormat32bppPARGB`。DIB 是预乘 alpha，GDI+ 写入时自动处理。
const PXF_PARGB: i32 = 0x000E_200B;

/// GDI+ 运行时。
///
/// 进程内**必须**在创建任何 GDI+ 对象之前构造，且只能有一个；把它放在
/// `main` 的栈上，作用域结束自动 `GdiplusShutdown`。这样"忘了 shutdown"
/// 和"shutdown 之后还用对象"都变成了所有权问题。
#[derive(Debug)]
pub struct GdiPlus {
    token: usize,
}

impl GdiPlus {
    pub fn startup() -> Result<Self> {
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            DebugEventCallback: 0,
            SuppressBackgroundThread: false.into(),
            SuppressExternalCodecs: false.into(),
        };
        let mut token = 0usize;
        let status = unsafe { GdiplusStartup(&mut token, &input, std::ptr::null_mut()) };
        check(GpStatus(status.0))?;
        Ok(Self { token })
    }
}

impl Drop for GdiPlus {
    fn drop(&mut self) {
        unsafe { GdiplusShutdown(self.token) };
    }
}

/// 一块 GDI+ 位图。可以是 DIB 的视图（Canvas），也可以是独立分配的缓存。
#[derive(Debug)]
pub struct Bitmap {
    ptr: NonNull<GpBitmap>,
    width: u32,
    height: u32,
}

impl Bitmap {
    /// 从外部像素缓冲创建视图（`scan0` 由调用方保证生命周期）。
    ///
    /// `stride` 传 0 表示按 `width * 4` 紧密排列。原版 `NewCache()` 就这么用。
    pub fn from_scan0(width: i32, height: i32, stride: i32, scan0: *mut u8) -> Result<Self> {
        if width <= 0 || height <= 0 {
            return Err(GfxError::BadSize { width, height });
        }
        let scan = if scan0.is_null() {
            None
        } else {
            Some(scan0 as *const u8)
        };
        let mut ptr: *mut GpBitmap = std::ptr::null_mut();
        let status =
            unsafe { GdipCreateBitmapFromScan0(width, height, stride, PXF_PARGB, scan, &mut ptr) };
        check(GpStatus(status.0))?;
        let ptr = NonNull::new(ptr).ok_or(GfxError::Gp(GpStatus(1)))?;
        Ok(Self {
            ptr,
            width: width as u32,
            height: height as u32,
        })
    }

    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// 取得绘制上下文。
    ///
    /// 返回的 [`Graphics`] **借用** `self`：位图一移动/释放，旧 graphics
    /// 立刻不可用。这正是 issue.md 里 `GdipDrawLine` 崩溃的根因所在 ——
    /// 原版把 graphics 存进组件结构体跨帧复用，surface 重建后仍用旧的。
    pub fn graphics(&self) -> Result<Graphics<'_>> {
        let mut ptr: *mut GpGraphics = std::ptr::null_mut();
        let status =
            unsafe { GdipGetImageGraphicsContext(self.ptr.as_ptr() as *mut GpImage, &mut ptr) };
        check(GpStatus(status.0))?;
        let ptr = NonNull::new(ptr).ok_or(GfxError::Gp(GpStatus(1)))?;
        let g = Graphics {
            ptr,
            _source: PhantomData,
        };
        g.apply_defaults()?;
        Ok(g)
    }

    pub(crate) fn as_image_ptr(&self) -> *mut GpImage {
        self.ptr.as_ptr() as *mut GpImage
    }
}

impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe { GdipDisposeImage(self.as_image_ptr()) };
    }
}

/// 绘制上下文。生命周期绑定到来源 [`Bitmap`]。
#[derive(Debug)]
pub struct Graphics<'a> {
    ptr: NonNull<GpGraphics>,
    _source: PhantomData<&'a Bitmap>,
}

impl Graphics<'_> {
    /// 原版 `SetupG()` 的四项设置。
    ///
    /// 数值必须与 C 版一致（4 / 7 / 4 / 4），否则亚像素渲染、图标缩放的
    /// 观感会与原版不同：
    /// - `SmoothingModeAntiAlias`：圆角与折线抗锯齿
    /// - `InterpolationModeHighQualityBicubic`：图标缩放
    /// - `PixelOffsetModeHalf`：半像素偏移，避免 1px 描边糊成 2px
    /// - `TextRenderingHintAntiAlias`：文字抗锯齿（非 ClearType）
    fn apply_defaults(&self) -> Result<()> {
        unsafe {
            check(GpStatus(
                GdipSetSmoothingMode(self.ptr.as_ptr(), SmoothingModeAntiAlias).0,
            ))?;
            check(GpStatus(
                GdipSetInterpolationMode(self.ptr.as_ptr(), InterpolationModeHighQualityBicubic).0,
            ))?;
            check(GpStatus(
                GdipSetPixelOffsetMode(self.ptr.as_ptr(), PixelOffsetModeHalf).0,
            ))?;
            check(GpStatus(
                GdipSetTextRenderingHint(self.ptr.as_ptr(), TextRenderingHintAntiAlias).0,
            ))?;
        }
        Ok(())
    }

    /// 用指定颜色清空整块画布。`0` = 完全透明。
    pub fn clear(&self, color: u32) -> Result<()> {
        unsafe { check(GpStatus(GdipGraphicsClear(self.ptr.as_ptr(), color).0)) }
    }

    /// 平移世界坐标（隐藏动画用）。与原版一致，单位是像素。
    pub fn translate(&self, dx: f32, dy: f32) -> Result<()> {
        unsafe {
            check(GpStatus(
                GdipTranslateWorldTransform(self.ptr.as_ptr(), dx, dy, MatrixOrderPrepend).0,
            ))
        }
    }

    pub fn reset_transform(&self) -> Result<()> {
        unsafe { check(GpStatus(GdipResetWorldTransform(self.ptr.as_ptr()).0)) }
    }

    pub fn set_clip(&self, path: &Path) -> Result<()> {
        unsafe {
            check(GpStatus(
                GdipSetClipPath(self.ptr.as_ptr(), path.ptr.as_ptr(), CombineModeReplace).0,
            ))
        }
    }

    pub fn reset_clip(&self) -> Result<()> {
        unsafe { check(GpStatus(GdipResetClip(self.ptr.as_ptr()).0)) }
    }

    pub fn draw_line(&self, pen: &Pen, x1: f32, y1: f32, x2: f32, y2: f32) -> Result<()> {
        unsafe {
            check(GpStatus(
                GdipDrawLine(self.ptr.as_ptr(), pen.ptr.as_ptr(), x1, y1, x2, y2).0,
            ))
        }
    }

    pub fn draw_path(&self, pen: &Pen, path: &Path) -> Result<()> {
        unsafe {
            check(GpStatus(
                GdipDrawPath(self.ptr.as_ptr(), pen.ptr.as_ptr(), path.ptr.as_ptr()).0,
            ))
        }
    }

    pub fn fill_path(&self, brush: &Brush, path: &Path) -> Result<()> {
        unsafe {
            check(GpStatus(
                GdipFillPath(self.ptr.as_ptr(), brush.as_ptr(), path.ptr.as_ptr()).0,
            ))
        }
    }

    pub fn fill_rectangle(&self, brush: &Brush, x: f32, y: f32, w: f32, h: f32) -> Result<()> {
        unsafe {
            check(GpStatus(
                GdipFillRectangle(self.ptr.as_ptr(), brush.as_ptr(), x, y, w, h).0,
            ))
        }
    }

    /// 画一张位图到目标矩形（源矩形取整张图）。`attrs` 可为 `None`。
    pub fn draw_bitmap(
        &self,
        bitmap: &Bitmap,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        attrs: Option<&ImageAttributes>,
    ) -> Result<()> {
        let attr_ptr = attrs.map_or(std::ptr::null(), |a| a.ptr.as_ptr());
        unsafe {
            check(GpStatus(
                GdipDrawImageRectRect(
                    self.ptr.as_ptr(),
                    bitmap.as_image_ptr(),
                    x,
                    y,
                    w,
                    h,
                    0.0,
                    0.0,
                    bitmap.width() as f32,
                    bitmap.height() as f32,
                    UnitPixel,
                    attr_ptr,
                    0,
                    std::ptr::null_mut(),
                )
                .0,
            ))
        }
    }

    /// 在矩形内绘制文字。`-1` 长度表示读到 NUL 为止（与 C 版一致）。
    pub fn draw_text(
        &self,
        text: &str,
        font: &Font,
        rect: RectF,
        format: &StringFormat,
        brush: &Brush,
    ) -> Result<()> {
        let w = wide(text);
        unsafe {
            check(GpStatus(
                GdipDrawString(
                    self.ptr.as_ptr(),
                    PCWSTR(w.as_ptr()),
                    -1,
                    font.ptr.as_ptr(),
                    &rect,
                    format.ptr.as_ptr(),
                    brush.as_ptr(),
                )
                .0,
            ))
        }
    }

    /// 量文字宽度（用于气泡自适应）。原版 `TextWidth()`。
    pub fn measure_text(&self, text: &str, font: &Font, format: &StringFormat) -> Result<RectF> {
        let w = wide(text);
        let layout = RectF {
            X: 0.0,
            Y: 0.0,
            Width: 600.0,
            Height: 40.0,
        };
        let mut bounds = RectF::default();
        let mut chars = 0i32;
        let mut lines = 0i32;
        unsafe {
            check(GpStatus(
                GdipMeasureString(
                    self.ptr.as_ptr(),
                    PCWSTR(w.as_ptr()),
                    -1,
                    font.ptr.as_ptr(),
                    &layout,
                    format.ptr.as_ptr(),
                    &mut bounds,
                    &mut chars,
                    &mut lines,
                )
                .0,
            ))?;
        }
        Ok(bounds)
    }
}

impl Drop for Graphics<'_> {
    fn drop(&mut self) {
        unsafe { GdipDeleteGraphics(self.ptr.as_ptr()) };
    }
}

/// 画刷。实心与线性渐变两种，对外统一成 `&Brush`。
#[derive(Debug)]
pub struct Brush {
    kind: BrushKind,
}

#[derive(Debug)]
enum BrushKind {
    Solid(NonNull<GpSolidFill>),
    Gradient(NonNull<GpLineGradient>),
}

impl Brush {
    pub fn solid(color: u32) -> Result<Self> {
        let mut ptr: *mut GpSolidFill = std::ptr::null_mut();
        let status = unsafe { GdipCreateSolidFill(color, &mut ptr) };
        check(GpStatus(status.0))?;
        let ptr = NonNull::new(ptr).ok_or(GfxError::Gp(GpStatus(1)))?;
        Ok(Self {
            kind: BrushKind::Solid(ptr),
        })
    }

    /// 矩形内的线性渐变。`mode` 决定方向：水平 = `LinearGradientModeHorizontal`。
    ///
    /// 原版 `DrawBar()` 用 `Horiz() ? 2 : 1`，即 **ForwardDiagonal / Vertical**。
    /// 注意 2 是 `ForwardDiagonal`（左上→右下），不是水平 —— 这是刻意的
    /// 材质感来源，改成水平会失去斜向高光。
    pub fn linear(rect: RectF, color1: u32, color2: u32, mode: LinearGradientMode) -> Result<Self> {
        let mut ptr: *mut GpLineGradient = std::ptr::null_mut();
        let status = unsafe {
            GdipCreateLineBrushFromRect(&rect, color1, color2, mode, WrapModeTile, &mut ptr)
        };
        check(GpStatus(status.0))?;
        let ptr = NonNull::new(ptr).ok_or(GfxError::Gp(GpStatus(1)))?;
        Ok(Self {
            kind: BrushKind::Gradient(ptr),
        })
    }

    fn as_ptr(&self) -> *mut GpBrush {
        match &self.kind {
            // GpSolidFill / GpLineGradient 都是 GpBrush 的具体类型，
            // GDI+ 的 C API 在这里靠 void* 多态，Rust 侧显式转换。
            BrushKind::Solid(p) => p.as_ptr() as *mut GpBrush,
            BrushKind::Gradient(p) => p.as_ptr() as *mut GpBrush,
        }
    }
}

impl Drop for Brush {
    fn drop(&mut self) {
        unsafe { GdipDeleteBrush(self.as_ptr()) };
    }
}

/// 画笔。
#[derive(Debug)]
pub struct Pen {
    ptr: NonNull<GpPen>,
}

impl Pen {
    /// 宽度单位固定为像素（`UnitPixel`），与 C 版的 `2` 一致。
    pub fn new(color: u32, width: f32) -> Result<Self> {
        let mut ptr: *mut GpPen = std::ptr::null_mut();
        let status = unsafe { GdipCreatePen1(color, width, UnitPixel, &mut ptr) };
        check(GpStatus(status.0))?;
        let ptr = NonNull::new(ptr).ok_or(GfxError::Gp(GpStatus(1)))?;
        Ok(Self { ptr })
    }
}

impl Drop for Pen {
    fn drop(&mut self) {
        unsafe { GdipDeletePen(self.ptr.as_ptr()) };
    }
}

/// 路径。
#[derive(Debug)]
pub struct Path {
    ptr: NonNull<GpPath>,
}

impl Path {
    pub fn new() -> Result<Self> {
        let mut ptr: *mut GpPath = std::ptr::null_mut();
        // FillModeAlternate = 0，与原版 GdipCreatePath(0, ...) 一致。
        let status = unsafe { GdipCreatePath(FillModeAlternate, &mut ptr) };
        check(GpStatus(status.0))?;
        let ptr = NonNull::new(ptr).ok_or(GfxError::Gp(GpStatus(1)))?;
        Ok(Self { ptr })
    }

    pub fn add_arc(
        &self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        start_angle: f32,
        sweep: f32,
    ) -> Result<()> {
        unsafe {
            check(GpStatus(
                GdipAddPathArc(self.ptr.as_ptr(), x, y, w, h, start_angle, sweep).0,
            ))
        }
    }

    pub fn close_figure(&self) -> Result<()> {
        unsafe { check(GpStatus(GdipClosePathFigure(self.ptr.as_ptr()).0)) }
    }

    /// 圆角矩形路径。四段 90° 弧，顺序与 C 版 `DrawRoundRectPath()` 完全一致。
    ///
    /// 直径先夹到不超过宽高，且不小于 0.5 —— 圆角为 0 时退化成一个极小的
    /// 圆角矩形而不是报错，这是原版的行为，视觉上等同直角。
    pub fn round_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Result<Self> {
        let p = Self::new()?;
        let mut d = r * 2.0;
        if d > w {
            d = w;
        }
        if d > h {
            d = h;
        }
        if d < 0.5 {
            d = 0.5;
        }
        p.add_arc(x, y, d, d, 180.0, 90.0)?;
        p.add_arc(x + w - d, y, d, d, 270.0, 90.0)?;
        p.add_arc(x + w - d, y + h - d, d, d, 0.0, 90.0)?;
        p.add_arc(x, y + h - d, d, d, 90.0, 90.0)?;
        p.close_figure()?;
        Ok(p)
    }
}

impl Drop for Path {
    fn drop(&mut self) {
        unsafe { GdipDeletePath(self.ptr.as_ptr()) };
    }
}

/// 字体族。
#[derive(Debug)]
pub struct FontFamily {
    ptr: NonNull<GpFontFamily>,
}

impl FontFamily {
    /// 按名字创建；失败时回落到系统无衬线族（原版同样逻辑）。
    pub fn new(name: &str) -> Result<Self> {
        let w = wide(name);
        let mut ptr: *mut GpFontFamily = std::ptr::null_mut();
        let status = unsafe {
            GdipCreateFontFamilyFromName(PCWSTR(w.as_ptr()), std::ptr::null_mut(), &mut ptr)
        };
        if status.0 != 0 || ptr.is_null() {
            let st = unsafe { GdipGetGenericFontFamilySansSerif(&mut ptr) };
            check(GpStatus(st.0))?;
        }
        let ptr = NonNull::new(ptr).ok_or(GfxError::Gp(GpStatus(14)))?;
        Ok(Self { ptr })
    }
}

impl Drop for FontFamily {
    fn drop(&mut self) {
        unsafe { GdipDeleteFontFamily(self.ptr.as_ptr()) };
    }
}

/// 字体。
#[derive(Debug)]
pub struct Font {
    ptr: NonNull<GpFont>,
}

impl Font {
    /// `style_bold = true` 对应 GDI+ 的 FontStyleBold(1)。
    pub fn new(family: &FontFamily, em_size: f32, style_bold: bool) -> Result<Self> {
        let mut ptr: *mut GpFont = std::ptr::null_mut();
        let style = i32::from(style_bold);
        let status =
            unsafe { GdipCreateFont(family.ptr.as_ptr(), em_size, style, UnitPixel, &mut ptr) };
        check(GpStatus(status.0))?;
        let ptr = NonNull::new(ptr).ok_or(GfxError::Gp(GpStatus(1)))?;
        Ok(Self { ptr })
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        unsafe { GdipDeleteFont(self.ptr.as_ptr()) };
    }
}

/// 排版格式（对齐 / 行对齐 / 截断方式）。
#[derive(Debug)]
pub struct StringFormat {
    ptr: NonNull<GpStringFormat>,
}

impl StringFormat {
    /// `flags` 传 0 即默认；`0x1000` = `StringFormatFlagsNoWrap`（气泡用）。
    pub fn new(flags: i32) -> Result<Self> {
        let mut ptr: *mut GpStringFormat = std::ptr::null_mut();
        let status = unsafe { GdipCreateStringFormat(flags, 0, &mut ptr) };
        check(GpStatus(status.0))?;
        let ptr = NonNull::new(ptr).ok_or(GfxError::Gp(GpStatus(1)))?;
        Ok(Self { ptr })
    }

    pub fn align(self, align: StringAlignment) -> Result<Self> {
        unsafe {
            check(GpStatus(
                GdipSetStringFormatAlign(self.ptr.as_ptr(), align).0,
            ))?
        };
        Ok(self)
    }

    pub fn line_align(self, align: StringAlignment) -> Result<Self> {
        unsafe {
            check(GpStatus(
                GdipSetStringFormatLineAlign(self.ptr.as_ptr(), align).0,
            ))?
        };
        Ok(self)
    }

    pub fn trimming(self, trimming: StringTrimming) -> Result<Self> {
        unsafe {
            check(GpStatus(
                GdipSetStringFormatTrimming(self.ptr.as_ptr(), trimming).0,
            ))?
        };
        Ok(self)
    }

    /// 原版 `g_fmtC`：水平居中 + 垂直居中 + 省略号截断。
    pub fn centered() -> Result<Self> {
        Self::new(0)?
            .align(StringAlignmentCenter)?
            .line_align(StringAlignmentCenter)?
            .trimming(StringTrimmingEllipsisCharacter)
    }

    /// 原版 `g_fmtL`：左对齐 + 垂直居中 + 省略号截断（NoWrap）。
    pub fn leading() -> Result<Self> {
        Self::new(0x1000)?
            .line_align(StringAlignmentCenter)?
            .trimming(StringTrimmingEllipsisCharacter)
    }

    /// 原版 `g_fmtR`：右对齐 + 垂直居中 + 省略号截断。
    pub fn trailing() -> Result<Self> {
        Self::new(0)?
            .align(StringAlignmentFar)?
            .line_align(StringAlignmentCenter)?
            .trimming(StringTrimmingEllipsisCharacter)
    }
}

impl Drop for StringFormat {
    fn drop(&mut self) {
        unsafe { GdipDeleteStringFormat(self.ptr.as_ptr()) };
    }
}

/// 图像属性（wrap mode + 颜色矩阵）。
///
/// 原版用它做三件事：图标平铺裁剪（`WrapModeClamp`）、淡出（alpha 0.78）、
/// 倒影的两级渐隐（alpha 0.30 / 0.11）、磨砂整体透明度。
#[derive(Debug)]
pub struct ImageAttributes {
    ptr: NonNull<GpImageAttributes>,
}

impl ImageAttributes {
    pub fn new() -> Result<Self> {
        let mut ptr: *mut GpImageAttributes = std::ptr::null_mut();
        let status = unsafe { GdipCreateImageAttributes(&mut ptr) };
        check(GpStatus(status.0))?;
        let ptr = NonNull::new(ptr).ok_or(GfxError::Gp(GpStatus(1)))?;
        let attrs = Self { ptr };
        // 三项都设成 Clamp：图标/磨砂在边缘不能平铺出重复像素。
        unsafe {
            check(GpStatus(
                GdipSetImageAttributesWrapMode(attrs.ptr.as_ptr(), WrapModeClamp, 0, false).0,
            ))?;
        }
        Ok(attrs)
    }

    /// 设置 alpha 缩放（只改颜色矩阵的第 19 项）。
    pub fn set_alpha_scale(&self, alpha: f32) -> Result<()> {
        let mut m = [0.0f32; 25];
        for (i, slot) in m.iter_mut().enumerate() {
            *slot = match i {
                0 | 6 | 12 | 18 | 24 => 1.0,
                _ => 0.0,
            };
        }
        m[18] = alpha;
        let matrix = ColorMatrix { m };
        unsafe {
            check(GpStatus(
                GdipSetImageAttributesColorMatrix(
                    self.ptr.as_ptr(),
                    ColorAdjustTypeBitmap,
                    true,
                    &matrix,
                    std::ptr::null(),
                    ColorMatrixFlagsDefault,
                )
                .0,
            ))
        }
    }
}

impl Drop for ImageAttributes {
    fn drop(&mut self) {
        unsafe { GdipDisposeImageAttributes(self.ptr.as_ptr()) };
    }
}
