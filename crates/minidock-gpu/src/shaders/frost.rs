//! GPU separable gaussian blur for the frost layer.
//!
//! Two compute passes replace `frost::blur_rgba`. The result is a
//! `Rgba8Unorm` texture registered with egui, so no GPU->CPU readback is
//! needed. The CPU implementation remains as a fallback and as the golden
//! reference in tests.
//!
//! Layout note: pass 1 writes `mid[x * h + y]` (transposed) so pass 2 can read
//! a contiguous column, matching the CPU reference exactly.

use std::num::NonZeroU64;

use super::ShaderRuntime;
use egui_wgpu::wgpu;
use glass_backdrop::blur::{clamp_radius, normalized_gaussian_weights, sigma_for_radius};

const WGSL: &str = include_str!("frost.wgsl");

const WORKGROUP: u32 = 64;

/// Pipeline and per-size GPU resources for one frost layer.
pub struct GpuBlur {
    rt: ShaderRuntime,
    bgl: wgpu::BindGroupLayout,
    h_pipeline: wgpu::ComputePipeline,
    v_pipeline: wgpu::ComputePipeline,
    target: Option<Target>,
}

struct Target {
    w: u32,
    h: u32,
    radius: u32,
    params: wgpu::Buffer,
    weights: wgpu::Buffer,
    src: wgpu::Buffer,
    #[expect(dead_code, reason = "owned by the bind group; only written on GPU")]
    mid: wgpu::Buffer,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "owned by the registered view; only written on GPU"
        )
    )]
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    group: wgpu::BindGroup,
}

impl std::fmt::Debug for GpuBlur {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuBlur")
            .field("size", &self.target.as_ref().map(|t| (t.w, t.h)))
            .finish_non_exhaustive()
    }
}

impl GpuBlur {
    /// Compile the shader and both pipelines. Called lazily, once per window
    /// that actually enables frost.
    #[must_use]
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("minidock-frost-blur"),
            source: wgpu::ShaderSource::Wgsl(WGSL.into()),
        });
        let storage = |read_only| wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("minidock-frost-blur"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(32),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: storage(true),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: storage(false),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: storage(true),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("minidock-frost-blur"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = |entry_point| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("minidock-frost-blur"),
                layout: Some(&layout),
                module: &module,
                entry_point: Some(entry_point),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        Some(Self {
            rt: ShaderRuntime::new(device, queue),
            bgl,
            h_pipeline: pipeline("h_blur"),
            v_pipeline: pipeline("v_blur"),
            target: None,
        })
    }

    /// The output texture view, registered with egui by the caller.
    #[must_use]
    pub fn view(&self) -> Option<&wgpu::TextureView> {
        self.target.as_ref().map(|t| &t.view)
    }

    /// Upload captured BGRA bytes, run both passes, return the output size.
    /// Returns `None` on invalid input so the caller can fall back to CPU.
    pub fn blur(
        &mut self,
        bits: &[u8],
        w: u32,
        h: u32,
        radius: u32,
        corner_px: f32,
    ) -> Option<(u32, u32)> {
        let pixels = (w as usize).checked_mul(h as usize)?;
        let bytes = pixels.checked_mul(4)?;
        if w == 0 || h == 0 || bits.len() < bytes {
            return None;
        }
        let r = clamp_radius(radius);
        let weights = normalized_gaussian_weights(radius, sigma_for_radius(radius));
        self.ensure(w, h, r)?;

        let t = self.target.as_ref()?;
        let mut uniform = [0u8; 32];
        uniform[0..4].copy_from_slice(&w.to_ne_bytes());
        uniform[4..8].copy_from_slice(&h.to_ne_bytes());
        uniform[8..12].copy_from_slice(&r.to_ne_bytes());
        uniform[12..16].copy_from_slice(&(weights.len() as u32).to_ne_bytes());
        uniform[16..20].copy_from_slice(&corner_px.to_ne_bytes());
        self.rt.queue.write_buffer(&t.params, 0, &uniform);

        let wb: Vec<u8> = weights.iter().flat_map(|f| f.to_ne_bytes()).collect();
        self.rt.queue.write_buffer(&t.weights, 0, &wb);
        self.rt.queue.write_buffer(&t.src, 0, &bits[..bytes]);

        let groups = (pixels as u32).div_ceil(WORKGROUP);
        let mut encoder = self
            .rt
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("minidock-frost-blur"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("minidock-frost-blur"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, &t.group, &[]);
            pass.set_pipeline(&self.h_pipeline);
            pass.dispatch_workgroups(groups, 1, 1);
            pass.set_pipeline(&self.v_pipeline);
            pass.dispatch_workgroups(groups, 1, 1);
        }
        self.rt.queue.submit(Some(encoder.finish()));
        Some((w, h))
    }

    fn ensure(&mut self, w: u32, h: u32, radius: u32) -> Option<()> {
        if let Some(t) = &self.target {
            if t.w == w && t.h == h && t.radius == radius {
                return Some(());
            }
        }
        let pixels = u64::from(w) * u64::from(h);
        let src_size = pixels.checked_mul(4)?;
        let mid_size = pixels.checked_mul(16)?;
        let weights_len = (2 * radius + 1) as u64 * 4;
        let buf = |label, size, usage| {
            self.rt.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let params = buf(
            "frost-params",
            32,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let weights = buf(
            "frost-weights",
            weights_len,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let src = buf(
            "frost-src",
            src_size,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let mid = buf("frost-mid", mid_size, wgpu::BufferUsages::STORAGE);
        let texture = self.rt.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("minidock-frost"),
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
        let group = self
            .rt
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("minidock-frost-blur"),
                layout: &self.bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: src.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: mid.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: weights.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                ],
            });
        self.target = Some(Target {
            w,
            h,
            radius,
            params,
            weights,
            src,
            mid,
            texture,
            view,
            group,
        });
        Some(())
    }

    /// Read the output texture back to CPU. Test-only.
    #[cfg(test)]
    pub fn readback(&self) -> Option<Vec<u8>> {
        let t = self.target.as_ref()?;
        self.rt.readback(&t.texture, t.w, t.h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frost::{apply_round_alpha, blur_rgba};
    use crate::gpu::Gpu;

    /// Deterministic synthetic image with sharp edges and gradients.
    fn sample(w: usize, h: usize) -> Vec<u8> {
        let mut v = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                v[i] = ((x * 13 + y * 7) % 256) as u8;
                v[i + 1] = ((x * 3 + y * 29) % 256) as u8;
                v[i + 2] = if (x / 7 + y / 5) % 2 == 0 { 220 } else { 30 };
                v[i + 3] = 255;
            }
        }
        v
    }

    /// Same 640x400 capture as the live path: CPU blur vs GPU compute.
    #[test]
    #[ignore = "throughput benchmark; run with --ignored"]
    fn blur_throughput_cpu_vs_gpu() {
        use std::time::{Duration, Instant};
        let gpu = Gpu::new("").expect("D3D12 adapter");
        let mut blur = GpuBlur::new(&gpu.device, &gpu.queue).expect("pipeline");
        let (w, h) = (640u32, 400u32);
        let src = sample(w as usize, h as usize);
        const REPS: u32 = 10;

        blur.blur(&src, w, h, 12, 0.0);
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());

        let t0 = Instant::now();
        for _ in 0..REPS {
            std::hint::black_box(blur_rgba(&src, w as usize, h as usize, 12));
        }
        let cpu: Duration = t0.elapsed() / REPS;

        let t1 = Instant::now();
        for _ in 0..REPS {
            blur.blur(&src, w, h, 12, 0.0).unwrap();
        }
        let submit: Duration = t1.elapsed() / REPS;
        let t2 = Instant::now();
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        let drain = t2.elapsed();

        println!(
            "cpu blur {:.2} ms | gpu submit {:.3} ms/frame | {} queued frames drained in {:.2} ms",
            cpu.as_secs_f64() * 1000.0,
            submit.as_secs_f64() * 1000.0,
            REPS,
            drain.as_secs_f64() * 1000.0
        );
    }

    fn frame_input(w: u32, h: u32) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(w as f32, h as f32),
            )),
            ..Default::default()
        }
    }

    /// End-to-end check: the registered compute texture must produce the same
    /// frame as the CPU ColorImage path, including sampling and blending.
    #[test]
    #[ignore = "requires a GPU and a desktop session; run with --ignored"]
    fn gpu_frost_renders_like_cpu_frost() {
        let gpu = Gpu::new("").expect("D3D12 adapter");
        let (w, h) = (64u32, 48u32);
        let src = sample(w as usize, h as usize);
        let spec = crate::frost::Spec {
            radius: 6,
            corner_pt: 8.0,
        };
        let ppp = 1.0f32;

        let mut f = crate::frame::FrameRenderer::new(&gpu, w, h, ppp).expect("renderer");
        assert!(f.set_frost_gpu(&src, w, h, spec, ppp));
        let gpu_pixels = f
            .render_rgba(&gpu, frame_input(w, h), |_| {})
            .expect("gpu frame")
            .0;

        let mut f = crate::frame::FrameRenderer::new(&gpu, w, h, ppp).expect("renderer");
        f.set_frost(crate::frost::compose(&src, w, h, spec, ppp).expect("cpu image"));
        let cpu_pixels = f
            .render_rgba(&gpu, frame_input(w, h), |_| {})
            .expect("cpu frame")
            .0;

        assert_eq!(gpu_pixels.len(), cpu_pixels.len());
        let mut worst = 0i32;
        for (a, b) in gpu_pixels.iter().zip(&cpu_pixels) {
            worst = worst.max((i32::from(*a) - i32::from(*b)).abs());
        }
        assert!(worst <= 1, "worst pixel delta={worst}");
    }

    /// GPU output must match the CPU reference byte-for-byte (tolerance 1 for
    /// f32 accumulation order). Run on a machine with a D3D12 adapter.
    #[test]
    #[ignore = "requires a GPU; run with --ignored"]
    fn gpu_blur_matches_cpu_reference() {
        let gpu = Gpu::new("").expect("D3D12 adapter");
        let mut blur = GpuBlur::new(&gpu.device, &gpu.queue).expect("pipeline");
        let (w, h) = (64u32, 48u32);
        let src = sample(w as usize, h as usize);
        for (radius, corner) in [(1u32, 0.0f32), (6, 0.0), (12, 7.0)] {
            let mut cpu = blur_rgba(&src, w as usize, h as usize, radius);
            apply_round_alpha(&mut cpu, w as usize, h as usize, corner);
            blur.blur(&src, w, h, radius, corner).expect("dispatch");
            let got = blur.readback().expect("readback");
            assert_eq!(got.len(), cpu.len());
            let mut worst = 0i32;
            for (a, b) in got.iter().zip(&cpu) {
                worst = worst.max((i32::from(*a) - i32::from(*b)).abs());
            }
            assert!(
                worst <= 1,
                "radius={radius} corner={corner} worst delta={worst}"
            );
        }
    }
}
