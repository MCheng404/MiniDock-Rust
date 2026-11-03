//! 栏体外的环境烟雾：把 `minidock_core::smoke` 的模拟接到 egui 上。
//!
//! 这一层做三件事：
//! 1. 驱动：按真实帧间隔推进模拟（`dt` 取单调时钟的差值，不用固定 16 ms）；
//! 2. 上传：把预乘 RGBA 位图写进一张 egui 纹理；
//! 3. 绘制：图像加上升的小烟团，画在栏体**之前**。
//!
//! 烟画在栏体下面：画在上面会把图标染紫。栏体四周溢出的那圈已经够了。
//!
//! 纹理尺寸按**物理像素**算，绘制时除以 `pixels_per_point`：分层窗口用
//! 物理像素，egui 用逻辑点，混用会在 150% 缩放下缩一半。

use egui::{Color32, ColorImage, TextureHandle, TextureOptions};
use minidock_core::smoke::{Fluid, Frame, Noise, Rgb, Tuning, CELL, RES};

/// 上升的小烟团：从栏体冒出的暗色小团，外圈带一层彩色辉光。
///
/// 配置项叫 `clouds`，这里按形状叫 `Puff`。
#[derive(Clone, Copy, Debug)]
struct Puff {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    life: f32,
    max: f32,
}

/// 一个够用的伪随机（xorshift32）。
///
/// 不引 `rand`：这里只要"看起来随机"，且必须**可复现**（同一帧重绘两次
/// 得到同一个粒子，否则无人值守截图每次都不一样）。
#[derive(Debug)]
struct Rng(u32);

impl Rng {
    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// `[0, 1)`。
    fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / 16_777_216.0
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }
}

/// 一帧里与"画在哪儿"有关的输入。
///
/// 打包成一个结构体而不是五个参数：`paint` 本来已经要收 `ui` / `ppp` /
/// `tune`，再加四个坐标就超过 clippy 的 7 个参数线，而且调用点会变成一串
/// 无名的元组，读的人分不清哪个是栏体、哪个是位移。
#[derive(Clone, Copy, Debug)]
pub struct View {
    /// 栏体在窗口里的物理矩形 `(x, y, w, h)`，不含隐藏位移。
    pub bar: (f32, f32, f32, f32),
    /// 隐藏动画位移（物理像素）。
    pub offset: (f32, f32),
    /// 光标在窗口物理坐标里的位置；`None` = 不在 Dock 上。
    pub cursor: Option<(f32, f32)>,
    /// 单调时钟的秒数。
    pub now_s: f64,
}

/// 烟雾的运行时状态。一个 Dock 窗口一份。
pub struct SmokeFx {
    /// 噪声表：40 ms 生成，第一次真正出烟时才做（关掉烟雾的用户不必付这个钱）。
    noise: Option<Noise>,
    fluid: Fluid,
    /// 合成后的纹理。
    tex: Option<TextureHandle>,
    /// 上一层用的时间（秒，单调）。`None` = 第一帧，不推进模拟。
    last_s: Option<f64>,
    /// 上一帧光标位置（**窗口物理坐标**），用来算推力方向。
    last_mouse: Option<(f32, f32)>,
    /// 烟团粒子的贴图（一张 64×64，重复使用）。
    puff_tex: Option<TextureHandle>,
    puffs: Vec<Puff>,
    /// 粒子生成累加器：满 1 就冒一个。
    spawn_acc: f32,
    rng: Rng,
    /// 上一帧是否在画。用来判断"刚从小隐藏里回来"，需要丢弃旧的时间基准。
    was_on: bool,
    /// 诊断计帧（`MINIDOCK_SMOKE_TRACE=1`）。
    trace_n: u32,
}

/// 诊断开关。与 `MINIDOCK_GEOM` / `MINIDOCK_TRACE` 同一套约定。
fn trace_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("MINIDOCK_SMOKE_TRACE").is_ok_and(|v| v.trim() != "0"))
}

impl std::fmt::Debug for SmokeFx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmokeFx")
            .field("grid", &self.fluid.grid())
            .field("puffs", &self.puffs.len())
            .field("noise", &self.noise.is_some())
            .finish()
    }
}

impl Default for SmokeFx {
    fn default() -> Self {
        Self::new()
    }
}

impl SmokeFx {
    #[must_use]
    pub fn new() -> Self {
        Self {
            noise: None,
            fluid: Fluid::new(),
            tex: None,
            last_s: None,
            last_mouse: None,
            puff_tex: None,
            puffs: Vec::new(),
            spawn_acc: 0.0,
            rng: Rng(0x9E37_79B9),
            was_on: false,
            trace_n: 0,
        }
    }

    /// 是否还需要继续出帧（主循环据此决定等待超时）。
    ///
    /// 与 `Tuning::enabled` 分开：这个回答的是"上一帧画了东西吗"，
    /// 关掉烟雾之后不必再叫醒。
    #[must_use]
    pub fn active(&self) -> bool {
        self.was_on
    }

    /// 画一帧。`view` 里是这一帧的坐标与时刻。
    pub fn paint(&mut self, ui: &egui::Ui, ppp: f32, tune: &Tuning, view: View) {
        let View {
            bar,
            offset,
            cursor,
            now_s,
        } = view;
        if !tune.enabled {
            self.was_on = false;
            self.last_s = None;
            return;
        }
        self.was_on = true;
        let noise = self.noise.get_or_insert_with(Noise::build);
        // ① 帧间隔。第一帧 / 暂停后回来：dt = 0（只光栅化，不推进）。
        let dt = match self.last_s {
            Some(prev) => ((now_s - prev) as f32).clamp(0.0, minidock_core::smoke::MAX_DT),
            None => 0.0,
        };
        self.last_s = Some(now_s);
        // 纹理漂移用的时间。取模是为了让 `f32` 在跑几天之后仍有小数精度。
        let t = ((now_s * f64::from(tune.speed)) % 3600.0) as f32;

        // ② 模拟区域：栏体四边各外扩一个 margin。用理想矩形，**不夹到
        //    窗口内** —— 超出的部分画的时候自然被离屏目标裁掉，而夹紧会
        //    让网格尺寸随隐藏动画抖动（每帧重建 = 每帧清空状态）。
        let (mx, my) = (tune.margin_x(), tune.margin_y());
        let (bx, by, bw, bh) = bar;
        let gx0 = bx - mx;
        let gy0 = by - my;
        let w = ((bw + 2.0 * mx) / CELL).ceil().max(4.0) as usize;
        let h = ((bh + 2.0 * my) / CELL).ceil().max(4.0) as usize;
        self.fluid.ensure(w, h, 0.0, (mx, my), tune.thickness);

        // ③ 鼠标：位置换算到网格单元，速度按"上一帧到这一帧的位移"算。
        //
        // 位移归一化到 1/60 秒。不归一化时帧率越低鼠标拨得越狠，看起来
        // 像在甩烟。
        let mouse = cursor.map_or((-999.0, -999.0), |(x, y)| {
            ((x - gx0) / CELL, (y - gy0) / CELL)
        });
        let norm = (dt * 60.0).min(2.0);
        let vel = match (cursor, self.last_mouse) {
            (Some((x, y)), Some((px, py))) => ((x - px) / CELL * norm, (y - py) / CELL * norm),
            _ => (0.0, 0.0),
        };
        self.last_mouse = cursor;

        self.fluid.step(
            noise,
            tune,
            Frame {
                dt: dt * tune.speed,
                time: t,
                mouse,
                mouse_vel: vel,
            },
        );

        // ④ 光栅化 + 上色。第二色沿栏体从左到右过渡，需要知道栏体在
        //    位图里的横向位置。
        let (base, second) = tune.colors((now_s % 86400.0) as f32);
        let (pw, ph) = self.fluid.raster(noise, base, t);
        if pw == 0 || ph == 0 {
            return;
        }
        self.fluid.post(
            base,
            second,
            ((bx - gx0) / RES as f32, bw / RES as f32),
            tune.opacity,
        );

        // 诊断：网格 / 位图 / 最大 alpha / 栏体矩形 / 光标。
        // 只在 `MINIDOCK_SMOKE_TRACE=1` 时出声，且约每 30 帧一次 ——
        // 这个功能"看不见"的原因只可能是三层之一（没启用 / 全透明 /
        // 画在窗口外），这三项覆盖了前两项，矩形覆盖第三项。
        if trace_on() {
            self.trace_n = self.trace_n.wrapping_add(1);
            if self.trace_n % 30 == 1 {
                let max_a = self
                    .fluid
                    .pixels()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|p| p[3])
                    .max()
                    .unwrap_or(0);
                eprintln!(
                    "[smoke] grid={w}x{h} bmp={pw}x{ph} maxA={max_a} bar=({bx},{by},{bw},{bh}) off={offset:?} op={:.2} base=[{:.0},{:.0},{:.0}] cursor={cursor:?}",
                    tune.opacity, base[0], base[1], base[2]
                );
            }
        }

        let image = ColorImage::from_rgba_premultiplied([pw, ph], self.fluid.pixels());
        let tex = match &mut self.tex {
            Some(t) => {
                t.set(image, TextureOptions::LINEAR);
                t.clone()
            }
            None => ui
                .ctx()
                .load_texture("minidock-smoke", image, TextureOptions::LINEAR),
        };
        self.tex = Some(tex.clone());

        let k = 1.0 / ppp;
        let rect = egui::Rect::from_min_size(
            egui::pos2((gx0 + offset.0) * k, (gy0 + offset.1) * k),
            egui::vec2((pw * RES) as f32 * k, (ph * RES) as f32 * k),
        );
        // 不透明度 <= 100% 走 tint alpha；>100% 已经在 `post` 里提过 alpha。
        let alpha = (tune.opacity.min(1.0) * 255.0).round() as u8;
        ui.painter().image(
            tex.id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::from_rgba_unmultiplied(255, 255, 255, alpha),
        );

        // ⑤ 上升的小烟团。
        if tune.clouds {
            self.paint_puffs(ui, ppp, tune, view, base, dt);
        }
    }

    /// 小烟团：从栏体上缘冒出来，缓慢上浮、左右摆、边升边变大。
    ///
    /// 用一张预先做好的软边贴图叠两次（辉光 + 实体）：比画十几个圆便宜，
    /// 也不会在圆的相切处出现白边。
    fn paint_puffs(
        &mut self,
        ui: &egui::Ui,
        ppp: f32,
        tune: &Tuning,
        view: View,
        base: Rgb,
        dt: f32,
    ) {
        let (bx, by, bw, _bh) = view.bar;
        let offset = view.offset;
        // 生成速率：栏体越宽冒得越多。
        self.spawn_acc += dt * (1.2 + bw / 900.0);
        while self.spawn_acc >= 1.0 {
            self.spawn_acc -= 1.0;
            let x = bx + self.rng.range(0.0, bw.max(1.0));
            let y = by + self.rng.range(0.0, 8.0);
            let vx = self.rng.range(-7.0, 7.0);
            let vy = -self.rng.range(10.0, 24.0);
            let max = self.rng.range(2.6, 4.0);
            self.puffs.push(Puff {
                x,
                y,
                vx,
                vy,
                life: 0.0,
                max,
            });
        }
        // 推进与回收。`y` 出画（升到区域上缘之外）或寿命到就删。
        let top = by - tune.margin_y();
        self.puffs.retain_mut(|p| {
            p.life += dt;
            p.x += (p.vx + 6.0 * (p.life * 3.0 + p.x * 0.02).sin()) * dt;
            p.y += p.vy * dt;
            p.life < p.max && p.y > top
        });

        if self.puffs.is_empty() {
            return;
        }
        let tex = self.puff_texture(ui.ctx());
        let k = 1.0 / ppp;
        let glow = [
            (base[0] * 1.55 + 40.0).min(255.0) / 255.0,
            (base[1] * 1.55 + 40.0).min(255.0) / 255.0,
            (base[2] * 1.55 + 40.0).min(255.0) / 255.0,
        ];
        for p in &self.puffs {
            let u = (p.life / p.max).clamp(0.0, 1.0);
            // 正弦包络：出生与消散两头淡，中间实。再乘一个"刚离开栏体"
            // 的淡入，否则粒子会在栏体边缘凭空出现。
            let a =
                (std::f32::consts::PI * u).sin().powf(0.8) * ((p.y - top) / 18.0).clamp(0.0, 1.0);
            if a < 0.02 {
                continue;
            }
            let r = 3.2 + p.max * 1.5 * (0.6 + 0.5 * u);
            let c = egui::pos2((p.x + offset.0) * k, (p.y + offset.1) * k);
            let halo = egui::vec2(r * 3.2, r * 3.2) * k;
            let core = egui::vec2(r * 2.0, r * 1.8) * k;
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(
                tex.id(),
                egui::Rect::from_center_size(c, halo),
                uv,
                Color32::from_rgba_unmultiplied(
                    (glow[0] * 255.0) as u8,
                    (glow[1] * 255.0) as u8,
                    (glow[2] * 255.0) as u8,
                    (38.0 * a) as u8,
                ),
            );
            ui.painter().image(
                tex.id(),
                egui::Rect::from_center_size(c, core),
                uv,
                Color32::from_rgba_unmultiplied(8, 5, 12, (200.0 * a) as u8),
            );
        }
    }

    /// 烟团贴图：若干圆取并集形成的软边团块。
    ///
    /// 只做一张：形状的重复感靠每次绘制的非等比缩放与透明度差异打散。
    fn puff_texture(&mut self, ctx: &egui::Context) -> TextureHandle {
        if let Some(t) = &self.puff_tex {
            return t.clone();
        }
        const S: usize = 64;
        let mut rgba = vec![0u8; S * S * 4];
        // 中心一团 + 一圈小团。
        let mut circles = vec![(0.0_f32, 0.0_f32, 0.62_f32, 0.60_f32)];
        let n = 6;
        for k in 0..n {
            let ang = std::f32::consts::TAU * k as f32 / n as f32;
            let d = 0.62 + 0.08 * ((k * 5) % 4) as f32;
            let r = 0.36 + 0.07 * ((k * 3) % 4) as f32;
            circles.push((ang.cos() * d, ang.sin() * d * 0.86, r, r));
        }
        for y in 0..S {
            for x in 0..S {
                // 贴图坐标 → 形状坐标（形状大致落在 ±1.1）。
                let u = (x as f32 + 0.5) / S as f32 * 2.2 - 1.1;
                let v = (y as f32 + 0.5) / S as f32 * 2.2 - 1.1;
                let mut best = 0.0_f32;
                for &(cx, cy, rx, ry) in &circles {
                    let dx = (u - cx) / rx;
                    let dy = (v - cy) / ry;
                    let d = (dx * dx + dy * dy).sqrt();
                    best = best.max((1.0 - d).clamp(0.0, 1.0));
                }
                // 软边：线性距离场再 smoothstep 一次，边缘不会出现一圈硬边。
                let a = best * best * (3.0 - 2.0 * best);
                let b = (a * 255.0) as u8;
                let o = (y * S + x) * 4;
                rgba[o] = b;
                rgba[o + 1] = b;
                rgba[o + 2] = b;
                rgba[o + 3] = b;
            }
        }
        let tex = ctx.load_texture(
            "minidock-smoke-puff",
            ColorImage::from_rgba_premultiplied([S, S], &rgba),
            TextureOptions::LINEAR,
        );
        self.puff_tex = Some(tex.clone());
        tex
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 端到端跑几帧：模拟 → 光栅化 → 上传成 egui 纹理。
    ///
    /// 这条测试存在的理由：烟雾"看不见"有三层可能（没启用 / 全透明 /
    /// 没进纹理），而截图只能看到最后一层的结果 —— 也就是"什么都没有"。
    /// 这里直接抓住 `textures_delta`，把中间那层钉死。
    #[test]
    fn smoke_reaches_egui_as_non_empty_texture() {
        let ctx = egui::Context::default();
        let mut fx = SmokeFx::new();
        let tune = Tuning {
            enabled: true,
            ..Tuning::default()
        };
        // 栏体：200×60 放在 400×200 的画布里，四周留得出光晕。
        let bar = (100.0, 90.0, 200.0, 60.0);

        let mut last: Option<Vec<u8>> = None;
        for i in 0..40 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 200.0),
                )),
                ..Default::default()
            };
            let view = View {
                bar,
                offset: (0.0, 0.0),
                cursor: None,
                now_s: f64::from(i) * 0.033,
            };
            let mut out = ctx.run_ui(raw, |ui| {
                fx.paint(ui, 1.0, &tune, view);
            });
            for (id, deltas) in &out.textures_delta.set {
                for d in deltas {
                    let [w, h] = d.image.size();
                    // 只认烟雾那张：它是 2:1 的长条；烟团贴图是 64×64、
                    // 字形图集是正方形，都被这条筛掉。
                    if w > 60 && h > 30 && w != h {
                        let egui::epaint::ImageData::Color(img) = &d.image;
                        last = Some(img.pixels.iter().map(|c| c.a()).collect());
                        let _ = id;
                    }
                }
            }
            out.textures_delta.clear();
        }
        let alpha = last.expect("烟雾位图从未进入 textures_delta");
        let max = alpha.iter().copied().max().unwrap_or(0);
        assert!(max > 20, "上传的位图几乎全透明，max={max}");
        // 发射区是一条**轮廓窄带**，所以亮像素只占一成左右：要求
        // 8% 以上就够证明"烟真的画出来了"，又不至于被流量涨落打翻。
        let lit = alpha.iter().filter(|a| **a > 8).count();
        assert!(
            lit * 12 > alpha.len(),
            "只有 {lit}/{} 个像素有内容",
            alpha.len()
        );
    }
}
