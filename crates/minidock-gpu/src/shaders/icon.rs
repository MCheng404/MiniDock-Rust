//! GPU rounded-corner mask for icons. CPU `apply_round_mask` is the reference.
//! Icons are processed once each, so the output is read back into `ReadyIcon`.

use egui_wgpu::wgpu;

use super::ShaderRuntime;

const WGSL: &str = include_str!("icon.wgsl");

pub struct IconShader {
    rt: ShaderRuntime,
    pipeline: wgpu::ComputePipeline,
}

impl std::fmt::Debug for IconShader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IconShader").finish_non_exhaustive()
    }
}

impl IconShader {
    #[must_use]
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let mut rt = ShaderRuntime::new(device, queue);
        let pipeline = rt.pipeline("icon", WGSL, "main").clone();
        Self { rt, pipeline }
    }

    /// Apply the mask and return the resulting RGBA bytes.
    #[must_use]
    pub fn round_mask(
        &self,
        rgba: &[u8],
        w: u32,
        h: u32,
        bbox: (u32, u32, u32, u32),
        radius: f32,
    ) -> Option<Vec<u8>> {
        if w == 0 || h == 0 || radius <= 0.0 || rgba.len() < (w * h * 4) as usize {
            return None;
        }
        let p: [f32; 7] = [
            w as f32,
            h as f32,
            bbox.0 as f32,
            bbox.1 as f32,
            bbox.2 as f32,
            bbox.3 as f32,
            radius,
        ];
        let params = self.rt.upload("icon-mask-params", f32_bytes(&p));
        let pix = self.rt.upload("icon-pix", &rgba[..(w * h * 4) as usize]);
        let (texture, view) = self.rt.output(w, h);
        let group = self
            .rt
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("icon-mask"),
                layout: &self.pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: pix.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                ],
            });
        self.rt.dispatch(&self.pipeline, &group, w * h);
        self.rt.readback(&texture, w, h)
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
    use minidock_core::icon::{apply_round_mask, BBox};

    #[test]
    #[ignore = "requires a GPU; run with --ignored"]
    fn gpu_mask_matches_cpu_reference() {
        let gpu = Gpu::new("").expect("D3D12 adapter");
        let shader = IconShader::new(&gpu.device, &gpu.queue);
        let (w, h) = (48u32, 32u32);
        let mut src = vec![0u8; (w * h * 4) as usize];
        for (i, px) in src.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            px.copy_from_slice(&[(i % 256) as u8, 90, 200, 255]);
        }
        let bbox = BBox {
            x0: 4,
            y0: 3,
            x1: 40,
            y1: 27,
        };
        for radius in [3.0f32, 9.0] {
            let mut cpu = src.clone();
            apply_round_mask(&mut cpu, w, bbox, radius);
            let got = shader
                .round_mask(&src, w, h, (bbox.x0, bbox.y0, bbox.x1, bbox.y1), radius)
                .expect("dispatch");
            assert_eq!(got, cpu, "radius={radius}");
        }
    }
}
