//! 一帧的完整提交路径：egui → 离屏纹理 → 回读 → 分层窗口。
//!
//! 这条链路原先在 `overlay.rs`（设置窗口）与 `bin/dock.rs`（Dock 窗口）
//! 各写了一遍，逐行相同。抽出来之后**只有一个地方**需要跟着 wgpu/egui
//! 的 API 变动走，也只有一个地方需要保证顺序：
//!
//! 1. `run_ui` 产出 `FullOutput`；
//! 2. `textures_delta.set` 逐项上传（字形图集、图标纹理）；
//! 3. `tessellate` → `update_buffers` 写顶点/索引；
//! 4. `begin_render_pass` + `LoadOp::Clear(TRANSPARENT)` —— **透明清屏是
//!    窗口能透出桌面的关键**，写成 `Load` 或任何不透明色都会退化成实心块；
//! 5. `copy_texture_to_buffer` → 同步回读 → 预乘 BGRA 写进 DIB；
//! 6. `UpdateLayeredWindow` 交给 DWM；
//! 7. `textures_delta.free` 回收，然后 `clear()` —— 不 clear 会在 Drop 时
//!    触发断言 panic（M0 实测）。
//!
//! 关于颜色格式：离屏目标**必须**是 sRGB（[`Gpu::COLOR_FORMAT`]）。
//! 用 `Rgba8Unorm` 时 egui 会自己做一次 gamma 编码，回读出来的面板
//! 从 24 灰变成 81 灰。

use std::time::Duration;

use egui_wgpu::{Renderer, RendererOptions, ScreenDescriptor};
use windows::Win32::Foundation::HWND;

use crate::gpu::{Gpu, GpuError};
use crate::offscreen::Offscreen;
use crate::present::LayeredPresenter;
pub use crate::present::Reflection;

/// 一帧渲染完成后，调用方真正需要知道的两件事。
///
/// 不返回 `egui::FullOutput`：shapes 与纹理增量在 `render` 内部就消费掉了，
/// 交出去只会诱使调用方再处理一遍。
#[derive(Clone, Copy, Debug)]
pub struct FrameOutcome {
    /// egui 想要的光标形状（`WM_SETCURSOR` 时用）。
    pub cursor: egui::CursorIcon,
    /// 下一次该在什么时候重绘。`Duration::MAX` = 没有待办，可以阻塞等消息。
    pub repaint_delay: Duration,
}

/// Frost may come from the GPU compute target or the CPU fallback image.
enum FrostLayer {
    Gpu { id: egui::TextureId },
    Cpu(egui::TextureHandle),
}

impl FrostLayer {
    fn id(&self) -> egui::TextureId {
        match self {
            Self::Gpu { id } => *id,
            Self::Cpu(handle) => handle.id(),
        }
    }
}

/// 渲染目标 + 呈现器的组合。一个窗口一份。
pub struct FrameRenderer {
    ctx: egui::Context,
    renderer: Renderer,
    offscreen: Offscreen,
    presenter: LayeredPresenter,
    pixels_per_point: f32,
    device: egui_wgpu::wgpu::Device,
    queue: egui_wgpu::wgpu::Queue,
    /// Shared compute runtime; also registers native textures with egui.
    shaders: crate::shaders::ShaderRuntime,
    /// Lazily created: only windows that enable frost pay for the pipeline.
    blur: Option<crate::shaders::frost::GpuBlur>,
    /// Smoke compute output, registered after `run_ui` produces it.
    smoke: Option<crate::shaders::smoke::SharedLayer>,
    /// 倒影参数；`None` = 这一帧不画倒影（设置窗、菜单都为 None）。
    reflection: Option<Reflection>,
    /// 毛玻璃背景层，画在 UI 最底层（见 `crate::frost`）。
    frost: Option<FrostLayer>,
    /// 面板可见期间的实时抓取线程。`None` = 只在开窗时抓过一次。
    live: Option<crate::frost::Live>,
    /// 上一个 [`crate::frost::Spec`]，实时帧来了才知道模糊半径与圆角。
    frost_spec: Option<crate::frost::Spec>,
}

impl std::fmt::Debug for FrameRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameRenderer")
            .field("size", &(self.offscreen.width(), self.offscreen.height()))
            .field("pixels_per_point", &self.pixels_per_point)
            .finish_non_exhaustive()
    }
}

impl FrameRenderer {
    /// 建离屏目标、分层窗口 DIB 与 egui 渲染器。
    ///
    /// `width`/`height` 是**物理像素**（`CreateWindowExW` 与离屏纹理的口径）。
    /// 字体必须在第一帧之前装好：egui 0.37 缺字形是 `panic` 而不是豆腐块。
    pub fn new(
        gpu: &Gpu,
        width: u32,
        height: u32,
        pixels_per_point: f32,
    ) -> Result<Self, GpuError> {
        // Large windows use 2x: at high pixel density the extra samples cost
        // tens of MB without visible gain. Config can force 1/2/4.
        let flags = gpu
            .adapter
            .get_texture_format_features(Gpu::COLOR_FORMAT)
            .flags;
        let adapter_specific = gpu
            .device
            .features()
            .contains(egui_wgpu::wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES);
        let area = u64::from(width) * u64::from(height);
        let wanted = crate::perf::auto_msaa_samples(area, crate::gpu::msaa_preference());
        let samples = crate::perf::resolve_samples(wanted, flags, adapter_specific);
        let offscreen = Offscreen::new(&gpu.device, width, height, Gpu::COLOR_FORMAT, samples);
        let presenter = LayeredPresenter::new(width as i32, height as i32)?;

        let ctx = egui::Context::default();
        if !nooksill_settings::fonts::install_cjk(&ctx) {
            eprintln!("警告: 未找到中文字体，界面会因缺字形而崩溃。");
        }

        let renderer = Renderer::new(
            &gpu.device,
            Gpu::COLOR_FORMAT,
            RendererOptions {
                msaa_samples: samples,
                depth_stencil_format: None,
                // 抖动：柔和大面积低透明度的投影渐变时，8bit 里只有几十个
                // 可用档位，不开抖动会出肉眼可见的色带。
                dithering: true,
                ..Default::default()
            },
        );

        Ok(Self {
            ctx,
            renderer,
            offscreen,
            presenter,
            pixels_per_point,
            device: gpu.device.clone(),
            queue: gpu.queue.clone(),
            shaders: crate::shaders::ShaderRuntime::new(&gpu.device, &gpu.queue),
            blur: None,
            smoke: None,
            reflection: None,
            frost: None,
            live: None,
            frost_spec: None,
        })
    }

    /// 改变离屏目标与 DIB 的尺寸，**保留** egui 上下文与渲染管线。
    ///
    /// 为什么需要它：右键菜单的高度随项数变化，每次打开都要换尺寸。
    /// 而 `FrameRenderer::new` 里最贵的两步是 egui 上下文（装 CJK 字体，
    /// 解析字体文件）与 wgpu 管线编译（几十毫秒）—— 它们与尺寸无关。
    /// 每次重建的话，菜单弹出会明显卡一下。
    ///
    /// 尺寸不变时直接返回，避免无谓地重建纹理。
    pub fn resize(&mut self, gpu: &Gpu, width: u32, height: u32) -> Result<(), GpuError> {
        if self.offscreen.width() == width && self.offscreen.height() == height {
            return Ok(());
        }
        self.offscreen = Offscreen::new(
            &gpu.device,
            width,
            height,
            Gpu::COLOR_FORMAT,
            self.offscreen.samples(),
        );
        self.presenter = LayeredPresenter::new(width as i32, height as i32)?;
        Ok(())
    }

    /// Connect a smoke compute output to this renderer. Called once at startup.
    pub fn set_smoke_layer(&mut self, layer: crate::shaders::smoke::SharedLayer) {
        self.smoke = Some(layer);
    }

    /// Blur captured bytes on the GPU and register the output with egui.
    /// Returns false so the caller can fall back to CPU blur.
    pub fn set_frost_gpu(
        &mut self,
        bits: &[u8],
        w: u32,
        h: u32,
        spec: crate::frost::Spec,
        ppp: f32,
    ) -> bool {
        if self.blur.is_none() {
            self.blur = crate::shaders::frost::GpuBlur::new(&self.device, &self.queue);
        }
        let Some(blur) = self.blur.as_mut() else {
            return false;
        };
        let scale = f32::from(crate::frost::CAPTURE_PCT) / 100.0;
        if blur
            .blur(bits, w, h, spec.radius, spec.corner_pt * ppp * scale)
            .is_none()
        {
            return false;
        }
        let Some(view) = blur.view() else {
            return false;
        };
        let filter = egui_wgpu::wgpu::FilterMode::Linear;
        let id = match &self.frost {
            Some(FrostLayer::Gpu { id }) => {
                self.renderer.update_egui_texture_from_wgpu_texture(
                    &self.device,
                    view,
                    filter,
                    *id,
                );
                *id
            }
            _ => self
                .renderer
                .register_native_texture(&self.device, view, filter),
        };
        self.frost = Some(FrostLayer::Gpu { id });
        true
    }

    /// CPU fallback: reuse the texture handle when the size is unchanged.
    pub fn set_frost(&mut self, image: egui::ColorImage) {
        if let Some(FrostLayer::Cpu(tex)) = &mut self.frost {
            if tex.size() == image.size {
                tex.set(image, egui::TextureOptions::LINEAR);
                return;
            }
        }
        let tex = self
            .ctx
            .load_texture("nooksill-frost", image, egui::TextureOptions::LINEAR);
        self.frost = Some(FrostLayer::Cpu(tex));
    }

    /// 撤掉背景层（毛玻璃关掉、或捕获失败时调用）。
    pub fn clear_frost(&mut self) {
        self.frost = None;
    }

    /// 开始实时抓取背景。
    pub fn start_live_frost(&mut self, hwnd: HWND, spec: crate::frost::Spec, _ppp: f32) {
        self.frost_spec = Some(spec);
        self.live = crate::frost::Live::start(hwnd);
    }

    /// 停掉实时抓取。背景层留在原地，面板照常显示上一帧。
    pub fn stop_live_frost(&mut self) {
        self.live = None;
        self.frost_spec = None;
    }

    /// 当前毛玻璃参数。
    #[must_use]
    pub const fn frost_spec(&self) -> Option<crate::frost::Spec> {
        self.frost_spec
    }

    /// Consume the newest captured frame and blur it on the GPU.
    fn poll_frost(&mut self) -> bool {
        let Some(frame) = self.live.as_ref().and_then(crate::frost::Live::latest) else {
            return false;
        };
        let Some(spec) = self.frost_spec else {
            return false;
        };
        self.set_frost_gpu(&frame.bits, frame.w, frame.h, spec, self.pixels_per_point)
    }

    /// 当前有没有背景层。面板据此决定底色用不透明还是半透明 tint。
    #[must_use]
    pub const fn frost_on(&self) -> bool {
        self.frost.is_some()
    }

    /// egui 用的 `Context`（需要 `request_repaint` 之类时用）。
    #[must_use]
    pub const fn ctx(&self) -> &egui::Context {
        &self.ctx
    }

    #[must_use]
    pub const fn pixels_per_point(&self) -> f32 {
        self.pixels_per_point
    }

    #[must_use]
    pub const fn size(&self) -> (u32, u32) {
        (self.offscreen.width(), self.offscreen.height())
    }

    /// MSAA 采样数。截图工具会在输出里带出来，方便确认抗锯齿档位。
    #[must_use]
    pub const fn samples(&self) -> u32 {
        self.offscreen.samples()
    }

    /// 设置倒影参数，下一帧生效。`None` = 不画。
    ///
    /// 放在呈现层而不是 egui 里：倒影是"同一帧的镜像像素"，只有拿到回读
    /// 缓冲才可能画得准；用 egui 再画一遍图标是另一套代码，必然漂移。
    pub const fn set_reflection(&mut self, spec: Option<Reflection>) {
        self.reflection = spec;
    }

    /// Draw a frame and present it. `pos` is the window's screen origin.
    pub fn render(
        &mut self,
        gpu: &Gpu,
        raw: egui::RawInput,
        hwnd: HWND,
        pos: (i32, i32),
        build: impl FnMut(&mut egui::Ui),
    ) -> Result<FrameOutcome, GpuError> {
        let outcome = self.draw_frame(gpu, raw, build);
        {
            let presenter = &mut self.presenter;
            // Write the mapped pixels straight into the DIB: no full-frame
            // Vec allocation and no extra copy.
            self.offscreen
                .with_mapped_pixels(&gpu.device, |data, stride| {
                    presenter.upload_rgba_strided(data, stride as usize);
                    Ok(())
                })
                .map_err(GpuError::Wgpu)?;
        }
        if let Some(spec) = self.reflection {
            self.presenter.mirror_below(spec);
        }
        self.presenter.present(hwnd, pos.0, pos.1)?;
        Ok(outcome)
    }

    /// Draw a frame and return premultiplied RGBA pixels without presenting.
    /// Used by offscreen screenshots and pixel regression.
    pub fn render_rgba(
        &mut self,
        gpu: &Gpu,
        raw: egui::RawInput,
        build: impl FnMut(&mut egui::Ui),
    ) -> Result<(Vec<u8>, FrameOutcome), GpuError> {
        let outcome = self.draw_frame(gpu, raw, build);
        let pixels = self
            .offscreen
            .read_pixels(&gpu.device)
            .map_err(GpuError::Wgpu)?;
        Ok((pixels, outcome))
    }

    /// Submit one frame. Both output paths share this draw code.
    fn draw_frame(
        &mut self,
        gpu: &Gpu,
        raw: egui::RawInput,
        mut build: impl FnMut(&mut egui::Ui),
    ) -> FrameOutcome {
        // A live frost frame must land before `run_ui` claims texture deltas.
        self.poll_frost();
        let frost = self.frost.as_ref().map(FrostLayer::id);
        let full = self.ctx.run_ui(raw, |ui| {
            if let Some(id) = frost {
                let rect = ui.max_rect();
                ui.painter().image(
                    id,
                    rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
            build(ui);
        });

        // `SmokeFx::paint` produced a view inside `run_ui`; register it now,
        // before the render pass samples the texture. First frame has no id yet
        // and is skipped; from then on the same texture object is refreshed.
        if let Some(layer) = &self.smoke {
            if let Ok(mut l) = layer.lock() {
                if let Some(view) = &l.view {
                    let id = self.shaders.register(&mut self.renderer, view, l.id);
                    l.id = Some(id);
                }
            }
        }

        let delay = full
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map_or(Duration::MAX, |v| v.repaint_delay);
        let cursor = full.platform_output.cursor_icon;

        let egui::FullOutput {
            shapes,
            pixels_per_point,
            mut textures_delta,
            ..
        } = full;

        let jobs = self.ctx.tessellate(shapes, pixels_per_point);
        for (id, deltas) in &textures_delta.set {
            for delta in deltas {
                self.renderer
                    .update_texture(&gpu.device, &gpu.queue, *id, delta);
            }
        }

        let screen = ScreenDescriptor {
            size_in_pixels: [self.offscreen.width(), self.offscreen.height()],
            pixels_per_point,
        };
        let mut encoder =
            gpu.device
                .create_command_encoder(&egui_wgpu::wgpu::CommandEncoderDescriptor {
                    label: Some("nooksill-frame"),
                });
        let buffers =
            self.renderer
                .update_buffers(&gpu.device, &gpu.queue, &mut encoder, &jobs, &screen);

        {
            // MSAA 开启时颜色画进多采样纹理、硬件 resolve 到离屏纹理；
            // 关闭时 color_target() 把两个引用都指向同一块纹理。
            let (color, resolve) = self.offscreen.color_target();
            let mut pass = encoder
                .begin_render_pass(&egui_wgpu::wgpu::RenderPassDescriptor {
                    label: Some("nooksill"),
                    color_attachments: &[Some(egui_wgpu::wgpu::RenderPassColorAttachment {
                        view: color,
                        resolve_target: resolve,
                        depth_slice: None,
                        ops: egui_wgpu::wgpu::Operations {
                            // 透明清屏：分层窗口里 alpha=0 即"透出桌面"。
                            load: egui_wgpu::wgpu::LoadOp::Clear(
                                egui_wgpu::wgpu::Color::TRANSPARENT,
                            ),
                            store: egui_wgpu::wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            self.renderer.render(&mut pass, &jobs, &screen);
        }
        self.offscreen.copy_to_buffer(&mut encoder);
        gpu.queue
            .submit(buffers.into_iter().chain([encoder.finish()]));

        // Official egui-wgpu order: free after submit, then clear the delta.
        for id in &textures_delta.free {
            self.renderer.free_texture(id);
        }
        textures_delta.clear();

        FrameOutcome {
            cursor,
            repaint_delay: delay,
        }
    }
}
