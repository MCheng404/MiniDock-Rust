//! 离屏渲染目标与同步回读。
//!
//! 为什么需要它：wgpu 在 Windows 上**没有带 alpha 的交换链**
//! （DX12/Vulkan 都只报 `Opaque`，见 M0 实测与 ADR-008）。
//! 所以 Dock 的透明走这条链路：
//!
//! ```text
//! wgpu → MSAA 纹理（4 采样，RENDER_ATTACHMENT）
//!      → resolve → 纹理（RENDER_ATTACHMENT | COPY_SRC）
//!      → buffer（COPY_DST | MAP_READ）
//!      → CPU 回读（同步）
//!      → DIB → UpdateLayeredWindow
//! ```
//!
//! 代价是每帧一次回读。以 Dock 常见尺寸 1320×220 计约 1.1 MB，
//! 桌面级内存带宽完全吃得下；真正需要零拷贝时再上 DirectComposition（M3）。
//!
//! MSAA：egui 自带的抗锯齿是"羽化"——把每条边画成 1px 的透明渐变，
//! 直边效果很好，但**圆角弧段**在低 DPI 下仍看得出折线感（弧被展平成
//! 直线段，每段只有一次覆盖）。4 重采样对几何边缘做 4 份覆盖加权，
//! 弧段的过渡级数翻倍。格式不支持时回落到 1 采样（无 MSAA）。

use egui_wgpu::wgpu;

/// 一块离屏颜色目标 + 对应的回读缓冲。
#[derive(Debug)]
pub struct Offscreen {
    /// 解析后的颜色纹理（1 采样）。MSAA 开启时它是 resolve 目标，
    /// 回读永远从这里走。
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    /// MSAA 颜色附件（`samples > 1` 时才存在）。只做渲染_attachment，
    /// 内容不需要读回。
    msaa: Option<(wgpu::Texture, wgpu::TextureView)>,
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    /// 每行字节数按 256 对齐（`copy_texture_to_buffer` 的硬要求）。
    padded_row: u32,
    samples: u32,
}

impl Offscreen {
    pub fn new(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Self {
        // 解析目标：渲染通道要拿它当 resolve_target，所以也要 RENDER_ATTACHMENT。
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("minidock-offscreen"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let msaa = (samples > 1).then(|| {
            let t = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("minidock-offscreen-msaa"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let v = t.create_view(&wgpu::TextureViewDescriptor::default());
            (t, v)
        });

        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_row = (width * 4).div_ceil(align) * align;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("minidock-readback"),
            size: u64::from(padded_row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Self {
            texture,
            view,
            msaa,
            buffer,
            width,
            height,
            padded_row,
            samples,
        }
    }

    /// 渲染通道的颜色附件：`(view, resolve_target)`。
    ///
    /// MSAA 开启时颜色画进多采样纹理、硬件自动解析到 `self.texture`；
    /// 关闭时两个值指向同一块纹理、无 resolve。渲染通道的
    /// `msaa_samples` 必须与这里的采样数一致（管线由调用方配）。
    #[must_use]
    pub fn color_target(&self) -> (&wgpu::TextureView, Option<&wgpu::TextureView>) {
        match &self.msaa {
            Some((_, v)) => (v, Some(&self.view)),
            None => (&self.view, None),
        }
    }

    #[must_use]
    pub const fn samples(&self) -> u32 {
        self.samples
    }

    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// 把纹理拷进回读缓冲。必须在提交命令前调用。
    ///
    /// 读的是**解析后**的 `self.texture` —— resolve 在渲染通道里已经完成。
    pub fn copy_to_buffer(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
    }

    /// 同步回读，返回**紧密排列**的 RGBA（预乘 alpha）。
    ///
    /// 同步（`PollType::Wait`）而不是异步流水线：Dock 大部分时间是静止的，
    /// 用 1–2 ms 的阻塞换"代码简单、不会积压帧"更划算。真正 60fps 连续
    /// 动画时再改双缓冲 + 延迟一帧，这一点已记在 PORTING.md。
    pub fn read_pixels(&self, device: &wgpu::Device) -> Result<Vec<u8>, String> {
        let slice = self.buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| format!("poll 失败: {e}"))?;
        match rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(format!("map_async 失败: {e}")),
            Err(e) => return Err(format!("回读通道断开: {e}")),
        }

        let data = slice
            .get_mapped_range()
            .map_err(|e| format!("映射回读缓冲失败: {e}"))?;
        let row_bytes = (self.width * 4) as usize;
        let mut out = Vec::with_capacity(row_bytes * self.height as usize);
        for row in 0..self.height as usize {
            let start = row * self.padded_row as usize;
            out.extend_from_slice(&data[start..start + row_bytes]);
        }
        drop(data);
        self.buffer.unmap();
        Ok(out)
    }
}
