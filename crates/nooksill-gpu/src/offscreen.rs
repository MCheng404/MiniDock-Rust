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
            label: Some("nooksill-offscreen"),
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
                label: Some("nooksill-offscreen-msaa"),
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
            label: Some("nooksill-readback"),
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

    /// Synchronous readback into tightly packed premultiplied RGBA.
    /// Blocking 1-2 ms keeps the pipeline simple; revisit for continuous 60 fps.
    pub fn read_pixels(&self, device: &wgpu::Device) -> Result<Vec<u8>, String> {
        self.with_mapped_pixels(device, |data, stride| {
            unpad_rows(data, self.width, self.height, stride)
        })
    }

    /// Map the readback buffer and run `f` with the padded bytes and row stride.
    /// Unmaps on every path, including `f` returning an error. The present path
    /// uses this to write straight into the DIB without a full-frame copy.
    pub fn with_mapped_pixels<T>(
        &self,
        device: &wgpu::Device,
        f: impl FnOnce(&[u8], u32) -> Result<T, String>,
    ) -> Result<T, String> {
        let slice = self.buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        // `PollType::Wait` returns only after callbacks ran, so the channel
        // already holds a result. `try_recv` cannot hang the UI thread.
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| format!("poll 失败: {e}"))?;
        match rx.try_recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(format!("map_async 失败: {e}")),
            Err(e) => return Err(format!("回读回调未投递: {e}")),
        }

        // The buffer is mapped from here on. Every path must `unmap`, or the
        // next frame's `map_async` fails.
        let result = (|| {
            let data = slice
                .get_mapped_range()
                .map_err(|e| format!("映射回读缓冲失败: {e}"))?;
            f(&data[..], self.padded_row)
        })();
        self.buffer.unmap();
        result
    }
}

/// Remove the 256-byte row padding added by `copy_texture_to_buffer`.
/// Kept as a pure function so the copy can be tested without a GPU.
fn unpad_rows(data: &[u8], width: u32, height: u32, padded_row: u32) -> Result<Vec<u8>, String> {
    let row_bytes = width as usize * 4;
    let stride = padded_row as usize;
    if stride < row_bytes {
        return Err(format!("行距 {stride} 小于行宽 {row_bytes}"));
    }
    let rows = height as usize;
    if rows == 0 {
        return Ok(Vec::new());
    }
    let needed = stride
        .checked_mul(rows - 1)
        .and_then(|n| n.checked_add(row_bytes))
        .ok_or_else(|| "回读尺寸溢出".to_owned())?;
    if data.len() < needed {
        return Err(format!("回读缓冲过短：{} < {needed}", data.len()));
    }

    let mut out = Vec::with_capacity(row_bytes * rows);
    for row in 0..rows {
        let start = row * stride;
        out.extend_from_slice(&data[start..start + row_bytes]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::unpad_rows;

    #[test]
    fn unpad_rows_removes_alignment_padding() {
        // 1 px wide = 4 valid bytes, stride padded to 8.
        let data = [1, 2, 3, 4, 0, 0, 0, 0, 5, 6, 7, 8, 0, 0, 0, 0];
        assert_eq!(
            unpad_rows(&data, 1, 2, 8).unwrap(),
            vec![1, 2, 3, 4, 5, 6, 7, 8]
        );
    }

    #[test]
    fn unpad_rows_handles_already_aligned_rows() {
        // 64 px = 256 bytes, already aligned; no byte may be dropped.
        let data = vec![0xABu8; 256 * 2];
        let out = unpad_rows(&data, 64, 2, 256).unwrap();
        assert_eq!(out.len(), 256 * 2);
        assert!(out.iter().all(|&b| b == 0xAB));
    }

    #[test]
    fn unpad_rows_rejects_bad_geometry() {
        assert!(unpad_rows(&[0; 8], 4, 2, 4).is_err(), "行距小于行宽");
        assert!(unpad_rows(&[0; 8], 1, 3, 8).is_err(), "缓冲比声明的高度短");
    }

    #[test]
    fn unpad_rows_zero_height_is_empty() {
        assert!(unpad_rows(&[], 0, 0, 256).unwrap().is_empty());
        assert!(unpad_rows(&[], 10, 0, 256).unwrap().is_empty());
    }
}
