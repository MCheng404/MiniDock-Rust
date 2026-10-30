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

/// 渲染目标 + 呈现器的组合。一个窗口一份。
pub struct FrameRenderer {
    ctx: egui::Context,
    renderer: Renderer,
    offscreen: Offscreen,
    presenter: LayeredPresenter,
    pixels_per_point: f32,
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
        let offscreen = Offscreen::new(&gpu.device, width, height, Gpu::COLOR_FORMAT);
        let presenter = LayeredPresenter::new(width as i32, height as i32)?;

        let ctx = egui::Context::default();
        if !minidock_settings::fonts::install_cjk(&ctx) {
            eprintln!("警告: 未找到中文字体，界面会因缺字形而崩溃。");
        }

        let renderer = Renderer::new(
            &gpu.device,
            Gpu::COLOR_FORMAT,
            RendererOptions {
                msaa_samples: 1,
                depth_stencil_format: None,
                dithering: false,
                ..Default::default()
            },
        );

        Ok(Self {
            ctx,
            renderer,
            offscreen,
            presenter,
            pixels_per_point,
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
        self.offscreen = Offscreen::new(&gpu.device, width, height, Gpu::COLOR_FORMAT);
        self.presenter = LayeredPresenter::new(width as i32, height as i32)?;
        Ok(())
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

    /// 画一帧并呈现到 `hwnd`。`pos` 是窗口左上角的屏幕坐标。
    ///
    /// `build` 里只负责往 `ui` 上画；副作用（改配置、发命令）在闭包外处理。
    pub fn render(
        &mut self,
        gpu: &Gpu,
        raw: egui::RawInput,
        hwnd: HWND,
        pos: (i32, i32),
        build: impl FnMut(&mut egui::Ui),
    ) -> Result<FrameOutcome, GpuError> {
        let full = self.ctx.run_ui(raw, build);

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
                    label: Some("minidock-frame"),
                });
        let buffers =
            self.renderer
                .update_buffers(&gpu.device, &gpu.queue, &mut encoder, &jobs, &screen);

        {
            let mut pass = encoder
                .begin_render_pass(&egui_wgpu::wgpu::RenderPassDescriptor {
                    label: Some("minidock"),
                    color_attachments: &[Some(egui_wgpu::wgpu::RenderPassColorAttachment {
                        view: &self.offscreen.view,
                        resolve_target: None,
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

        let pixels = self
            .offscreen
            .read_pixels(&gpu.device)
            .map_err(GpuError::Wgpu)?;
        self.presenter.upload_rgba(&pixels);
        self.presenter.present(hwnd, pos.0, pos.1)?;

        for id in &textures_delta.free {
            self.renderer.free_texture(id);
        }
        textures_delta.clear();

        Ok(FrameOutcome {
            cursor,
            repaint_delay: delay,
        })
    }
}
