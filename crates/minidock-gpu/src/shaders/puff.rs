//! GPU-generated soft puff sprite. Built once per window.

use egui_wgpu::wgpu;

use super::ShaderRuntime;

const WGSL: &str = include_str!("puff.wgsl");
pub const SIZE: u32 = 64;

pub struct PuffShader {
    rt: ShaderRuntime,
    pipeline: wgpu::ComputePipeline,
}

impl std::fmt::Debug for PuffShader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PuffShader").finish_non_exhaustive()
    }
}

impl PuffShader {
    #[must_use]
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let mut rt = ShaderRuntime::new(device, queue);
        let pipeline = rt.pipeline("puff", WGSL, "main").clone();
        Self { rt, pipeline }
    }

    #[must_use]
    pub fn build(&self) -> Option<Vec<u8>> {
        let p = [SIZE as f32];
        // SAFETY: f32 is POD.
        let bytes = unsafe { std::slice::from_raw_parts(p.as_ptr().cast::<u8>(), p.len() * 4) };
        let params = self.rt.upload("puff-params", bytes);
        let (texture, view) = self.rt.output(SIZE, SIZE);
        let group = self
            .rt
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("puff"),
                layout: &self.pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                ],
            });
        self.rt.dispatch(&self.pipeline, &group, SIZE * SIZE);
        self.rt.readback(&texture, SIZE, SIZE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::Gpu;

    /// CPU reference for the sprite.
    fn reference() -> Vec<u8> {
        let s = SIZE as usize;
        let mut rgba = vec![0u8; s * s * 4];
        let mut circles = vec![(0.0_f32, 0.0_f32, 0.62_f32, 0.60_f32)];
        for k in 0..6 {
            let ang = std::f32::consts::TAU * k as f32 / 6.0;
            let d = 0.62 + 0.08 * ((k * 5) % 4) as f32;
            let r = 0.36 + 0.07 * ((k * 3) % 4) as f32;
            circles.push((ang.cos() * d, ang.sin() * d * 0.86, r, r));
        }
        for y in 0..s {
            for x in 0..s {
                let u = (x as f32 + 0.5) / s as f32 * 2.2 - 1.1;
                let v = (y as f32 + 0.5) / s as f32 * 2.2 - 1.1;
                let mut best = 0.0_f32;
                for &(cx, cy, rx, ry) in &circles {
                    let dx = (u - cx) / rx;
                    let dy = (v - cy) / ry;
                    best = best.max((1.0 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0));
                }
                let a = best * best * (3.0 - 2.0 * best);
                let b = (a * 255.0) as u8;
                let o = (y * s + x) * 4;
                rgba[o..o + 4].copy_from_slice(&[b, b, b, b]);
            }
        }
        rgba
    }

    #[test]
    #[ignore = "requires a GPU; run with --ignored"]
    fn gpu_sprite_matches_cpu_reference() {
        let gpu = Gpu::new("").expect("D3D12 adapter");
        let shader = PuffShader::new(&gpu.device, &gpu.queue);
        let got = shader.build().expect("dispatch");
        let want = reference();
        let mut worst = 0i32;
        for (a, b) in got.iter().zip(&want) {
            worst = worst.max((i32::from(*a) - i32::from(*b)).abs());
        }
        assert!(worst <= 1, "worst={worst}");
    }
}
