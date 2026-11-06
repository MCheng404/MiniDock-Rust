//! Shared GPU compute runtime for all pixel shaders.
//!
//! One runtime per `FrameRenderer`; pipelines are cached by (shader, entry).
//! Each effect owns its bind group layout and uses the helpers here for
//! buffers, output textures, dispatch, egui registration and readback.

use std::collections::HashMap;

use egui_wgpu::{wgpu, Renderer};

pub struct ShaderRuntime {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pipelines: HashMap<(&'static str, &'static str), wgpu::ComputePipeline>,
}

impl std::fmt::Debug for ShaderRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShaderRuntime")
            .field("pipelines", &self.pipelines.len())
            .finish_non_exhaustive()
    }
}

impl ShaderRuntime {
    #[must_use]
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self {
            device: device.clone(),
            queue: queue.clone(),
            pipelines: HashMap::new(),
        }
    }

    /// Compile once per (shader name, entry point). Layout is derived from the
    /// shader; effects use `get_bind_group_layout(0)`.
    pub fn pipeline(
        &mut self,
        name: &'static str,
        wgsl: &'static str,
        entry: &'static str,
    ) -> &wgpu::ComputePipeline {
        let device = &self.device;
        self.pipelines.entry((name, entry)).or_insert_with(|| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(name),
                source: wgpu::ShaderSource::Wgsl(wgsl.into()),
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(name),
                layout: None,
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        })
    }

    #[must_use]
    pub fn buffer(&self, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage,
            mapped_at_creation: false,
        })
    }

    /// Storage buffer filled with `data` in one upload.
    #[must_use]
    pub fn upload(&self, label: &str, data: &[u8]) -> wgpu::Buffer {
        let buf = self.buffer(
            label,
            data.len() as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        self.queue.write_buffer(&buf, 0, data);
        buf
    }

    /// `Rgba8Unorm` texture usable as a compute target, egui source and copy source.
    #[must_use]
    pub fn output(&self, w: u32, h: u32) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shader-output"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    pub fn dispatch(&self, pipeline: &wgpu::ComputePipeline, group: &wgpu::BindGroup, work: u32) {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("shader-dispatch"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("shader-dispatch"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(work.div_ceil(64), 1, 1);
        }
        self.queue.submit(Some(encoder.finish()));
    }

    /// Register (or refresh) a compute output as an egui user texture.
    pub fn register(
        &self,
        renderer: &mut Renderer,
        view: &wgpu::TextureView,
        id: Option<egui::TextureId>,
    ) -> egui::TextureId {
        match id {
            Some(id) => {
                renderer.update_egui_texture_from_wgpu_texture(
                    &self.device,
                    view,
                    wgpu::FilterMode::Linear,
                    id,
                );
                id
            }
            None => renderer.register_native_texture(&self.device, view, wgpu::FilterMode::Linear),
        }
    }

    /// Copy an output texture back to CPU. Used by the icon path and tests;
    /// the render paths never read back.
    #[must_use]
    pub fn readback(&self, texture: &wgpu::Texture, w: u32, h: u32) -> Option<Vec<u8>> {
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let stride = (w * 4).div_ceil(align) * align;
        let buf = self.buffer(
            "shader-readback",
            u64::from(stride) * u64::from(h),
            wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        );
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("shader-readback"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));

        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        rx.try_recv().ok()?.ok()?;
        let data = slice.get_mapped_range().ok()?;
        let row = w as usize * 4;
        let mut out = Vec::with_capacity(row * h as usize);
        for y in 0..h as usize {
            let start = y * stride as usize;
            out.extend_from_slice(&data[start..start + row]);
        }
        drop(data);
        buf.unmap();
        Some(out)
    }
}

pub mod frost;
pub mod icon;
pub mod puff;
pub mod smoke;
