//! GPU smoke raster + post. The fluid simulation stays on the CPU; every
//! per-pixel operation (bilinear upscale, three noise taps, shading,
//! premultiply, second colour, gain) runs in one compute pass.

use std::sync::{Arc, Mutex};

use egui_wgpu::wgpu;

use super::ShaderRuntime;

const WGSL: &str = include_str!("smoke.wgsl");
const PARAM_FLOATS: usize = 20;

/// Bridge between `SmokeFx` (owns the simulation) and `FrameRenderer` (owns
/// the egui-wgpu renderer that can register native textures).
#[derive(Debug, Default)]
pub struct SmokeLayer {
    pub view: Option<wgpu::TextureView>,
    pub id: Option<egui::TextureId>,
    pub size: (u32, u32),
}

pub type SharedLayer = Arc<Mutex<SmokeLayer>>;

pub struct SmokeShader {
    rt: ShaderRuntime,
    pipeline: wgpu::ComputePipeline,
    params: wgpu::Buffer,
    noise: Option<wgpu::Buffer>,
    fields: Option<wgpu::Buffer>,
    out: Option<(wgpu::Texture, wgpu::TextureView)>,
    out_size: (u32, u32),
    layer: SharedLayer,
}

impl std::fmt::Debug for SmokeShader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmokeShader")
            .field("size", &self.out_size)
            .finish_non_exhaustive()
    }
}

impl SmokeShader {
    #[must_use]
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let mut rt = ShaderRuntime::new(device, queue);
        let pipeline = rt.pipeline("smoke", WGSL, "raster").clone();
        let params = rt.buffer(
            "smoke-params",
            (PARAM_FLOATS * 4) as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        Self {
            rt,
            pipeline,
            params,
            noise: None,
            fields: None,
            out: None,
            out_size: (0, 0),
            layer: Arc::new(Mutex::new(SmokeLayer::default())),
        }
    }

    #[must_use]
    pub fn layer(&self) -> SharedLayer {
        self.layer.clone()
    }

    /// Noise tables are built once and shared with the CPU simulation.
    pub fn set_noise(&mut self, noise: &[f32]) {
        let bytes = f32_bytes(noise);
        self.noise = Some(self.rt.upload("smoke-noise", bytes));
    }

    /// Shade one frame. `fields` is `al | ox | oy`, each `gw*gh` floats.
    pub fn raster(&mut self, fields: &[f32], params: &[f32; PARAM_FLOATS]) -> Option<(u32, u32)> {
        let noise = self.noise.as_ref()?;
        let (pw, ph) = (params[2] as u32, params[3] as u32);
        if pw == 0 || ph == 0 {
            return None;
        }
        let need = (fields.len() * 4) as u64;
        if self.fields.is_none() {
            self.fields = Some(self.rt.buffer(
                "smoke-fields",
                need,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            ));
        }
        let field_buf = self.fields.as_ref()?;
        self.rt.queue.write_buffer(field_buf, 0, f32_bytes(fields));
        self.rt
            .queue
            .write_buffer(&self.params, 0, f32_bytes(params));

        if self.out_size != (pw, ph) {
            self.out = Some(self.rt.output(pw, ph));
            self.out_size = (pw, ph);
        }
        let (texture, view) = self.out.as_ref()?;
        let group = self
            .rt
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("smoke-raster"),
                layout: &self.pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: field_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: noise.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                ],
            });
        self.rt.dispatch(&self.pipeline, &group, pw * ph);

        if let Ok(mut layer) = self.layer.lock() {
            layer.view = Some(view.clone());
            layer.size = (pw, ph);
        }
        let _ = texture; // kept alive by the view and the bind group
        Some((pw, ph))
    }

    /// Test-only: read the shaded texture back for pixel comparison.
    #[cfg(test)]
    #[must_use]
    pub fn readback(&self) -> Option<Vec<u8>> {
        let (texture, _) = self.out.as_ref()?;
        self.rt.readback(texture, self.out_size.0, self.out_size.1)
    }
}

fn f32_bytes(v: &[f32]) -> &[u8] {
    // SAFETY: f32 is POD; the slice length is scaled by 4.
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), v.len() * 4) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::Gpu;
    use minidock_core::smoke::{Fluid, Frame, Noise, Tuning, CELL, RES};

    #[expect(
        clippy::too_many_arguments,
        reason = "flat shader param bundle mirrors the WGSL P array"
    )]
    fn params(
        gw: f32,
        gh: f32,
        pw: f32,
        ph: f32,
        n: f32,
        t: f32,
        base: [f32; 3],
        second: Option<[f32; 3]>,
        bar: (f32, f32),
        gain: f32,
    ) -> [f32; PARAM_FLOATS] {
        let c2 = second.unwrap_or([0.0; 3]);
        let mut p = [0.0f32; PARAM_FLOATS];
        p[0] = gw;
        p[1] = gh;
        p[2] = pw;
        p[3] = ph;
        p[4] = n;
        p[5] = CELL;
        p[6] = RES as f32;
        p[7] = t;
        p[8..11].copy_from_slice(&base);
        p[11..14].copy_from_slice(&c2);
        p[14] = if second.is_some() { 1.0 } else { 0.0 };
        p[15] = bar.0;
        p[16] = bar.1;
        p[17] = gain;
        p
    }

    /// GPU raster + post must match the CPU reference within one byte.
    #[test]
    #[ignore = "requires a GPU; run with --ignored"]
    fn gpu_raster_matches_cpu_reference() {
        let gpu = Gpu::new("").expect("D3D12 adapter");
        let mut shader = SmokeShader::new(&gpu.device, &gpu.queue);
        let noise = Noise::build_sized(32);
        let table: Vec<f32> = noise.layers().iter().flatten().copied().collect();
        shader.set_noise(&table);

        let (gw, gh) = (32usize, 16usize);
        let mut fluid = Fluid::new();
        let tune = Tuning {
            enabled: true,
            ..Tuning::default()
        };
        fluid.ensure(gw, gh, 0.0, (4.0, 6.0, 24.0, 4.0), tune.thickness);
        for i in 0..8 {
            fluid.step(
                &noise,
                &tune,
                Frame {
                    dt: 0.033,
                    time: i as f32 * 0.033,
                    mouse: (8.0, 8.0),
                    mouse_vel: (0.5, 0.0),
                },
            );
        }
        let (pw, ph) = fluid.bitmap_size();
        let base = [180.0f32, 90.0, 220.0];
        let t = 1.25f32;
        let bar = (4.0f32, 20.0f32);

        for (second, gain) in [(None, 1.0f32), (Some([40.0, 200.0, 120.0]), 1.3)] {
            let (al, ox, oy) = fluid.gpu_fields();
            let fields: Vec<f32> = [al, ox, oy].concat();
            let p = params(
                gw as f32,
                gh as f32,
                pw as f32,
                ph as f32,
                noise.size() as f32,
                t,
                base,
                second,
                bar,
                gain,
            );
            shader.raster(&fields, &p).expect("dispatch");
            let got = shader.readback().expect("readback");

            let (cpw, cph) = fluid.raster(&noise, base, t);
            fluid.post(base, second, bar, gain);
            let want = fluid.pixels();
            assert_eq!((cpw, cph), (pw, ph));
            assert_eq!(got.len(), want.len());
            let mut worst = 0i32;
            for (a, b) in got.iter().zip(want) {
                worst = worst.max((i32::from(*a) - i32::from(*b)).abs());
            }
            assert!(worst <= 1, "second={second:?} gain={gain} worst={worst}");
        }
    }
}
