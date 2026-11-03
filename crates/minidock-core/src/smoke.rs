//! 环境烟雾：栏体外侧上浮、卷曲、随鼠标让开的烟。
//!
//! 三块内容：
//!
//! - [`Noise`]：可平铺的烟纹，域扭曲分形噪声；
//! - [`Fluid::step`]：二维流体，浮力 + 涡量约束 + 压力投影；
//! - [`Fluid::raster`]：网格 → 预乘 RGBA 位图。
//!
//! 烟由密度场和速度场共同决定：密度被速度带走，速度由浮力和噪声驱动，
//! 涡量约束让小漩涡不散，光标把烟推开而不擦掉。预渲染动画做不出这种
//! 连续性。
//!
//! 三个约束：
//!
//! 1. 一条栏体一份模拟。
//! 2. 时间由调用方传入（`t` / `dt`），库内不取时钟，单测才能复现同一帧。
//! 3. 只对**底部停靠**生效：浮力朝上、上缘衰减慢，都是按底部调的。顶端
//!    停靠时烟往屏幕外冒；左右停靠时栏体是竖的，圆形距离场不成立。
//!    判断在 [`Tuning::from_config`] 里，不满足时直接给 `enabled = false`。

use crate::config::{Config, Key};

/// 噪声表边长。`192 = 4*48 = 3*64 = 6*32`，基频 4/3/6 的每个倍频周期都能
/// 整除它，平铺时没有接缝。
pub const NOISE_N: usize = 192;

/// 模拟网格的单元边长（物理像素）。
///
/// 5 px 在速度与精度之间取平：栏体高 48 px 时纵向 24 格，距离场算出来的
/// 圆角不会退化成方块。
pub const CELL: f32 = 5.0;

/// 光栅化分辨率：一个 [`CELL`] 切成几个像素。
///
/// 2 表示网格放大 2.5 倍上屏，中间靠双线性插值补齐：模拟在粗网格上跑，
/// 画面是连续的。
pub const RES: usize = 2;

/// 单帧最大步长（秒）。超过就截断：窗口被拖动 / 断点调试回来时，
/// 一个 3 秒的 `dt` 会让显式积分直接炸掉。
pub const MAX_DT: f32 = 0.05;

/// 颜色（0..=255，sRGB 分量）。
pub type Rgb = [f32; 3];

/// 八种预设色。`SmokeColor` 的 0..=7 直接索引这个数组。
///
/// **数组顺序即配置语义**：索引写进 ini 之后，调整顺序会改变已有配置的
/// 含义。
pub const PALETTE: [Rgb; 8] = [
    [88.0, 52.0, 160.0],
    [168.0, 84.0, 22.0],
    [160.0, 30.0, 36.0],
    [200.0, 60.0, 120.0],
    [34.0, 96.0, 150.0],
    [70.0, 140.0, 185.0],
    [34.0, 128.0, 78.0],
    [150.0, 108.0, 10.0],
];

/// 强调色槽位（`SmokeColor = 8`）。
pub const SLOT_ACCENT: i32 = 8;
/// 彩虹槽位（`SmokeColor = 9`）。
pub const SLOT_RAINBOW: i32 = 9;

/// 不透明度档位：17% / 30% / 50% / 70% / 100% / 125% / 150%。
///
/// 超过 100% 的两档靠 [`Fluid::post`] 的 alpha 增益实现 —— 预乘位图的
/// 分量不能超过 alpha，单纯乘颜色只会变灰。
const OPACITY: [f32; 7] = [0.17, 0.30, 0.50, 0.70, 1.0, 1.25, 1.5];
const DENSITY: [f32; 3] = [0.6, 1.0, 1.5];
const SPEED: [f32; 3] = [0.6, 1.0, 1.6];
const THICKNESS: [f32; 4] = [0.7, 1.0, 1.4, 1.9];
const MOUSE: [f32; 3] = [0.0, 1.0, 2.0];
const REACH: [f32; 3] = [0.7, 1.0, 1.4];

/// 从配置解出来的模拟参数。全部是"直接能用的量"，不再保留 ini 下标。
///
/// 解析放在 core 而不是 gpu：设置界面要显示"当前是哪一档"，预览要跟
/// 运行时完全一致，两边都得读同一套换算。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuning {
    pub enabled: bool,
    /// 全局不透明度（>1 时提亮，见 [`Fluid::post`]）。
    pub opacity: f32,
    /// 烟雾浓度。
    pub density: f32,
    /// 时间缩放。
    pub speed: f32,
    /// 光晕厚度，同时决定外边距（`fl_mx` / `fl_my`）。
    pub thickness: f32,
    /// 鼠标推力的强度，0 = 关。
    pub mouse: f32,
    /// 鼠标作用半径系数。
    pub reach: f32,
    /// 浅色高光。
    pub glow: bool,
    /// 主色槽位（0..=7 预设，8 强调色，9 彩虹）。
    pub color: i32,
    /// 第二色槽位（沿栏体从左到右过渡），`None` = 不开。
    pub color2: Option<i32>,
    /// 强调色的具体值：由平台层从 DWM 取，取不到就给主色。
    pub accent: Rgb,
    /// 是否生成上升的小烟团。
    pub clouds: bool,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            enabled: false,
            opacity: OPACITY[3],
            density: DENSITY[1],
            speed: SPEED[1],
            thickness: THICKNESS[1],
            mouse: MOUSE[1],
            reach: REACH[1],
            glow: false,
            color: 0,
            color2: None,
            accent: PALETTE[0],
            clouds: true,
        }
    }
}

impl Tuning {
    /// 从配置解析。`accent` 由调用方向平台层讨要（失败给主色即可）。
    ///
    /// 关闭、或停靠边不是底部时返回 `enabled = false` —— 见模块注释第 4 条。
    #[must_use]
    pub fn from_config(cfg: &Config, accent: Rgb) -> Self {
        let idx = |key: Key, table_len: usize| -> usize {
            (cfg.get_i32(key).max(0) as usize).min(table_len - 1)
        };
        let on = cfg.get_i32(Key::SMOKE) != 0 && cfg.edge == crate::config::Edge::Bottom;
        let c2 = cfg.get_i32(Key::SMOKE_COLOR2);
        Self {
            enabled: on,
            opacity: OPACITY[idx(Key::SMOKE_OPACITY, OPACITY.len())],
            density: DENSITY[idx(Key::SMOKE_DENSITY, DENSITY.len())],
            speed: SPEED[idx(Key::SMOKE_SPEED, SPEED.len())],
            thickness: THICKNESS[idx(Key::SMOKE_THICK, THICKNESS.len())],
            mouse: MOUSE[idx(Key::SMOKE_MOUSE, MOUSE.len())],
            reach: REACH[idx(Key::SMOKE_REACH, REACH.len())],
            glow: cfg.get_i32(Key::SMOKE_TONE) != 0,
            color: cfg.get_i32(Key::SMOKE_COLOR),
            color2: (c2 >= 0).then_some(c2),
            accent,
            clouds: cfg.get_i32(Key::SMOKE_CLOUDS) != 0,
        }
    }

    /// 左右外边距（px）。
    #[must_use]
    pub fn margin_x(&self) -> f32 {
        30.0 * self.thickness
    }

    /// 上下外边距（px）。
    #[must_use]
    pub fn margin_y(&self) -> f32 {
        34.0 * self.thickness
    }

    /// 上缘每步衰减系数。厚度越大衰减越慢（烟飘得远），速度越快衰减越快。
    ///
    /// 用幂而不是线性：线性在 thickness=1.9 时上缘衰减慢三成，烟会糊出
    /// 屏幕。
    #[must_use]
    pub fn decay_top(&self) -> f32 {
        0.982_f32.powf(self.speed / self.thickness)
    }

    /// 侧边 / 下方每步衰减系数。
    #[must_use]
    pub fn decay_side(&self) -> f32 {
        0.94_f32.powf(self.speed / self.thickness)
    }

    /// 某个槽位此刻的颜色（已应用 tone）。`wall` 是**未缩放**的秒数，
    /// 彩虹按它走 90 秒一圈 —— 用缩放后的时间会让"速度"顺带改变色相速度，
    /// 那是个很奇怪的第二效果。
    #[must_use]
    pub fn color_at(&self, slot: i32, wall: f32) -> Rgb {
        let mut c = match slot {
            0..=7 => PALETTE[slot as usize],
            SLOT_ACCENT => self.accent,
            _ => rainbow(wall),
        };
        if self.glow {
            // 高光档提亮同一色相，不换成白色。
            for v in &mut c {
                *v = (*v * 1.35 + 55.0).min(255.0);
            }
        }
        c
    }

    /// 主色与第二色。
    #[must_use]
    pub fn colors(&self, wall: f32) -> (Rgb, Option<Rgb>) {
        (
            self.color_at(self.color, wall),
            self.color2.map(|s| self.color_at(s, wall)),
        )
    }
}

/// 慢速彩虹：90 秒一圈，饱和度 0.7、明度 170，逐帧求值。
#[must_use]
pub fn rainbow(t: f32) -> Rgb {
    const V: f32 = 170.0;
    const SAT: f32 = 0.7;
    let h = (t / 90.0).rem_euclid(1.0) * 6.0;
    let k = h as usize;
    let f = h - k as f32;
    let pq = V * (1.0 - SAT);
    let qq = V * (1.0 - SAT * f);
    let tt = V * (1.0 - SAT * (1.0 - f));
    match k % 6 {
        0 => [V, tt, pq],
        1 => [qq, V, pq],
        2 => [pq, V, tt],
        3 => [pq, qq, V],
        4 => [tt, pq, V],
        _ => [V, pq, qq],
    }
}

// ============================ 烟纹（噪声） ============================

/// 三次平滑插值的网格噪声。
fn hash(x: i32, y: i32, seed: i32) -> f32 {
    let mut h = (x as u32)
        .wrapping_mul(374_761_393)
        .wrapping_add((y as u32).wrapping_mul(668_265_263))
        .wrapping_add((seed as u32).wrapping_mul(2_246_822_519));
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    ((h ^ (h >> 16)) & 0x00FF_FFFF) as f32 / 16_777_215.0
}

fn vnoise(x: f32, y: f32, period: i32, seed: i32) -> f32 {
    let xi = x.floor();
    let yi = y.floor();
    let xf = x - xi;
    let yf = y - yi;
    // `3t² - 2t³`：一阶导在格点处不连续，放大后能看到极轻微的棱。
    // 配合 5 个倍频和双线性放大看不出来，比 Perlin 的五次曲线省一半乘法。
    let u = xf * xf * (3.0 - 2.0 * xf);
    let v = yf * yf * (3.0 - 2.0 * yf);
    let (xi, yi) = (xi as i32, yi as i32);
    let wrap = |q: i32| ((q % period) + period) % period;
    let (x0, y0) = (wrap(xi), wrap(yi));
    let (x1, y1) = (wrap(xi + 1), wrap(yi + 1));
    let a = hash(x0, y0, seed);
    let b = hash(x1, y0, seed);
    let c = hash(x0, y1, seed);
    let d = hash(x1, y1, seed);
    a + (b - a) * u + (c - a) * v + (a - b - c + d) * u * v
}

/// 五个倍频的分形噪声。
fn fbm(x: f32, y: f32, period: i32, seed: i32) -> f32 {
    let mut total = 0.0;
    let mut amp = 0.5;
    for octave in 0..5 {
        let fi = 1 << octave;
        total += amp * vnoise(x * fi as f32, y * fi as f32, period * fi, seed + octave);
        amp *= 0.5;
    }
    total
}

/// 三层可平铺的烟纹（各 `n × n` 的 alpha）。
///
/// 生成方式是**域扭曲**：先取一层 FBM 当作每点的偏移量，再拿偏移后的
/// 坐标去采样第三层。直线状的噪声纹因此卷成烟絮，这是"像烟"和"像云"
/// 的分界线，也是最不该省的一步。
#[derive(Clone)]
pub struct Noise {
    n: usize,
    alpha: [Vec<f32>; 3],
    mean: [f32; 3],
}

impl std::fmt::Debug for Noise {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Noise")
            .field("n", &self.n)
            .field("mean", &self.mean)
            .finish()
    }
}

impl Noise {
    /// 按默认边长生成。[`NOISE_N`] 一档约 40 ms（release），只在启动时做一次。
    #[must_use]
    pub fn build() -> Self {
        Self::build_sized(NOISE_N)
    }

    /// 指定边长。**只给测试用**：小尺寸能把单测从几百毫秒压到几毫秒，
    /// 而平铺性质与边长无关（周期性由噪声空间决定，见模块注释）。
    #[must_use]
    pub fn build_sized(n: usize) -> Self {
        const BASIS: [i32; 3] = [4, 3, 6];
        const SEED: [i32; 3] = [11, 53, 97];
        let n = n.max(8);
        let mut alpha: [Vec<f32>; 3] = std::array::from_fn(|_| Vec::new());
        let mut mean = [0.0_f32; 3];
        for (k, buf) in alpha.iter_mut().enumerate() {
            buf.resize(n * n, 0.0);
            let bs = BASIS[k];
            let sd = SEED[k];
            let mut sum = 0.0_f64;
            for j in 0..n {
                for i in 0..n {
                    let x = i as f32 / n as f32 * bs as f32;
                    let y = j as f32 / n as f32 * bs as f32;
                    let qx = fbm(x, y, bs, sd);
                    let qy = fbm(x + 5.2, y + 1.3, bs, sd + 7);
                    let w = fbm(x + 3.2 * qx, y + 3.2 * qy, bs, sd + 13);
                    let mut al = ((w - 0.32) / 0.5).clamp(0.0, 1.0);
                    // smoothstep：把线性阈值弄成软边，否则烟絮边缘是一条
                    // 等值线，放大后能看出"色阶台阶"。
                    al = al * al * (3.0 - 2.0 * al);
                    buf[j * n + i] = al;
                    sum += f64::from(al);
                }
            }
            mean[k] = (sum / (n * n) as f64) as f32;
        }
        Self { n, alpha, mean }
    }

    /// 纹理均值。湍流里**必须**减掉它：不减时噪声期望不为 0，每帧都给
    /// 速度加同一个偏置，烟会整体朝一个方向漂。
    #[must_use]
    pub fn mean(&self, k: usize) -> f32 {
        self.mean[k]
    }

    #[must_use]
    pub fn size(&self) -> usize {
        self.n
    }

    /// 采样第 `k` 层。`x` / `y` 是**网格单元坐标**，`t` 是已按速度缩放的
    /// 秒数：内部再乘 0.55/0.7 与 1.2/2.0 让纹理缓慢斜向漂移。
    ///
    /// 湍流与密度目标都从这里取。
    #[must_use]
    pub fn source(&self, k: usize, x: f32, y: f32, t: f32) -> f32 {
        self.tap(k, x * 0.55 + t * 1.2, y * 0.70 + t * 2.0)
    }

    /// 直接双线性采样（`u` / `v` 就是纹理坐标，环绕平铺）。
    ///
    /// 与 [`Noise::source`] 的区别只在"要不要那层漂移变换"：光栅化阶段
    /// 的纹理坐标已经被位移场偏移过，再叠一次漂移就是双重位移。
    #[must_use]
    pub fn tap(&self, k: usize, u: f32, v: f32) -> f32 {
        let n = self.n;
        let nf = n as f32;
        let mut fx = u - (u / nf).floor() * nf;
        let mut fy = v - (v / nf).floor() * nf;
        // 浮点误差可能把 fx 推到 n 上（正好整除时），取模兜一下。
        if fx >= nf {
            fx -= nf;
        }
        if fy >= nf {
            fy -= nf;
        }
        let x0 = fx as usize;
        let y0 = fy as usize;
        let x1 = (x0 + 1) % n;
        let y1 = (y0 + 1) % n;
        let ax = fx - x0 as f32;
        let ay = fy - y0 as f32;
        let tex = &self.alpha[k];
        let at = |xx: usize, yy: usize| tex[yy * n + xx];
        (at(x0, y0) * (1.0 - ax) + at(x1, y0) * ax) * (1.0 - ay)
            + (at(x0, y1) * (1.0 - ax) + at(x1, y1) * ax) * ay
    }
}

// ============================ 流体 ============================

/// 一帧的输入。分成两个结构体（本结构与 [`Tuning`]）是因为生命周期不同：
/// 前者每帧都变，后者只在改设置时变。
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    /// 帧间隔（秒），调用方负责夹到 [`MAX_DT`] 以内。
    pub dt: f32,
    /// 已按速度缩放的秒数（纹理漂移、噪声湍流、密度目标都用它）。
    pub time: f32,
    /// 光标在**网格单元**坐标里的位置。
    pub mouse: (f32, f32),
    /// 光标一帧的位移（单元格）。
    pub mouse_vel: (f32, f32),
}

/// 单条栏体的流场 + 染料 + 光栅缓冲。
///
/// 所有场都是同一尺寸的平铺数组，下标 `i = x + y*w`；带 `0` 后缀的是
/// 上一帧的缓冲。
///
/// 只实现 `Debug` 的简版：逐字段打印 13 个 `Vec<f32>` 没有可读性。
pub struct Fluid {
    w: usize,
    h: usize,
    seed: f32,
    /// 左右 / 上下外边距（px），决定"胶囊"在网格里的位置。
    margin: (f32, f32),
    u: Vec<f32>,
    v: Vec<f32>,
    u0: Vec<f32>,
    v0: Vec<f32>,
    d: Vec<f32>,
    d0: Vec<f32>,
    /// 涡量与压力共用同一块内存，涡量的结果在解压力前就被清零。
    tmp: Vec<f32>,
    div: Vec<f32>,
    /// 位移场：纹理坐标被流场"拖着走"，烟丝因此跟着卷。
    ox: Vec<f32>,
    oy: Vec<f32>,
    ox0: Vec<f32>,
    oy0: Vec<f32>,
    /// 每格的最终 alpha（胶囊遮罩 + 密度）。
    al: Vec<f32>,
    /// 轮廓窄带权重（0..=1）：栏体边缘为 1，往内 3 格、往外 `reach` 格淡到 0。
    ///
    /// 只在 `ensure()` 里算一次 —— 它只跟尺寸、边距、厚度有关，与这一帧的
    /// 流场无关。源（[`Fluid::step`]）与光栅化（[`Fluid::raster`]）共用它，
    /// 于是"从哪儿冒烟"和"哪儿看得见"永远是同一条带子。
    edge: Vec<f32>,
    /// 生成 `edge` 时用的厚度（变了要重算）。
    thick: f32,
    /// 输出位图（预乘 RGBA）。
    px: Vec<u8>,
    pw: usize,
    ph: usize,
}

impl std::fmt::Debug for Fluid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fluid")
            .field("grid", &(self.w, self.h))
            .field("bitmap", &(self.pw, self.ph))
            .field("seed", &self.seed)
            .field("margin", &self.margin)
            .finish()
    }
}

impl Default for Fluid {
    fn default() -> Self {
        Self::new()
    }
}

impl Fluid {
    #[must_use]
    pub fn new() -> Self {
        Self {
            w: 0,
            h: 0,
            seed: 0.0,
            margin: (0.0, 0.0),
            u: Vec::new(),
            v: Vec::new(),
            u0: Vec::new(),
            v0: Vec::new(),
            d: Vec::new(),
            d0: Vec::new(),
            tmp: Vec::new(),
            div: Vec::new(),
            ox: Vec::new(),
            oy: Vec::new(),
            ox0: Vec::new(),
            oy0: Vec::new(),
            al: Vec::new(),
            edge: Vec::new(),
            thick: 0.0,
            px: Vec::new(),
            pw: 0,
            ph: 0,
        }
    }

    /// 保证网格与边距匹配，不匹配就整体重建（所有场清空）。
    ///
    /// 返回是否重建。**重建即丢失动画状态**，所以只在尺寸/边距真的变了
    /// 时调用；调用方（`SmokeFx`）每帧拿当前栏体尺寸来比一次即可。
    pub fn ensure(
        &mut self,
        w: usize,
        h: usize,
        seed: f32,
        margin: (f32, f32),
        thickness: f32,
    ) -> bool {
        if self.w == w
            && self.h == h
            && self.margin == margin
            && self.thick == thickness
            && !self.u.is_empty()
        {
            return false;
        }
        self.w = w.max(4);
        self.h = h.max(4);
        self.seed = seed;
        self.margin = margin;
        self.thick = thickness;
        let n = self.w * self.h;
        for f in [
            &mut self.u,
            &mut self.v,
            &mut self.u0,
            &mut self.v0,
            &mut self.d,
            &mut self.d0,
            &mut self.tmp,
            &mut self.div,
            &mut self.ox,
            &mut self.oy,
            &mut self.ox0,
            &mut self.oy0,
            &mut self.al,
        ] {
            f.clear();
            f.resize(n, 0.0);
        }
        self.edge = build_edge(self.w, self.h, margin, thickness);
        self.px.clear();
        self.pw = 0;
        self.ph = 0;
        true
    }

    #[must_use]
    pub fn grid(&self) -> (usize, usize) {
        (self.w, self.h)
    }

    /// 推进一帧。
    ///
    /// 顺序不能动：浮力 → 湍流 → 鼠标 → 阻尼 → 涡量约束 → 自输运 →
    /// 压力投影 → 染料输运 → 位移场 → 源/耗散。压力投影**必须**在
    /// 自输运之后：投影的目的是让速度场无散，先投影再输运等于白投影。
    pub fn step(&mut self, noise: &Noise, tune: &Tuning, f: Frame) {
        let (w, h) = (self.w, self.h);
        if w < 4 || h < 4 {
            return;
        }
        let dt = f.dt.clamp(0.0, MAX_DT);
        if dt <= 0.0 {
            return;
        }
        let (mx, my) = f.mouse;
        let mvx = f.mouse_vel.0.clamp(-8.0, 8.0);
        let mvy = f.mouse_vel.1.clamp(-8.0, 8.0);
        let mstr = tune.mouse;
        let rr = tune.reach * tune.reach;
        let t = f.time;
        let mean1 = noise.mean(1);
        let mean2 = noise.mean(2);

        // ① 体积力：浮力朝上（y 减小），噪声给不规则湍流，光标推开烟。
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let i = x + y * w;
                self.v[i] -= dt * 7.0 * self.d[i];
                let n1 = noise.source(
                    1,
                    x as f32 * 1.6 + 17.0 + self.seed,
                    y as f32 * 1.6,
                    t * 0.35,
                ) - mean1;
                let n2 = noise.source(
                    2,
                    x as f32 * 1.4 + self.seed,
                    y as f32 * 1.4 + 53.0,
                    t * 0.3,
                ) - mean2;
                self.u[i] += dt * 26.0 * n1;
                self.v[i] += dt * 16.0 * n2;
                let ddx = x as f32 - mx;
                let ddy = y as f32 - my;
                let r2 = ddx * ddx + ddy * ddy;
                if mstr > 0.0 && r2 < 130.0 * mstr * rr {
                    let g = (-r2 / (26.0 * mstr * rr)).exp() * mstr;
                    self.u[i] += g * (mvx * 5.0 + ddx * 0.55);
                    self.v[i] += g * (mvy * 5.0 + ddy * 0.55);
                    // 中心区顺带稀释染料：光标划过是"让开"，不是"擦除"。
                    self.d[i] *= 1.0 - 0.16 * (-r2 / (22.0 * rr)).exp() * mstr;
                }
                self.u[i] *= 0.985;
                self.v[i] *= 0.985;
            }
        }

        // ② 涡量约束：数值耗散会把小漩涡抹平，这里把涡量"找回来"。
        //    没有这一步，烟会在两秒内变成一团糊。
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let i = x + y * w;
                self.tmp[i] =
                    0.5 * ((self.v[i + 1] - self.v[i - 1]) - (self.u[i + w] - self.u[i - w]));
            }
        }
        for y in 2..h - 2 {
            for x in 2..w - 2 {
                let i = x + y * w;
                let gx = 0.5 * (self.tmp[i + 1].abs() - self.tmp[i - 1].abs());
                let gy = 0.5 * (self.tmp[i + w].abs() - self.tmp[i - w].abs());
                let len = (gx * gx + gy * gy).sqrt() + 1e-5;
                self.u[i] += dt * 2.2 * (gy / len) * self.tmp[i];
                self.v[i] -= dt * 2.2 * (gx / len) * self.tmp[i];
            }
        }

        // ③ 自输运。先把当前速度抄进 `u0`/`v0`，再以它们为回溯场写回
        //    `u`/`v`：两份此刻逐元素相等，等价于原地回溯，但避开了
        //    `&self.u` 与 `&mut self.u` 的借用冲突。
        self.u0.copy_from_slice(&self.u);
        self.v0.copy_from_slice(&self.v);
        advect(&self.u0, &self.v0, &self.u0, &mut self.u, w, h, dt * 8.0);
        advect(&self.u0, &self.v0, &self.v0, &mut self.v, w, h, dt * 8.0);

        // ④ 压力投影：解泊松方程让速度场无散。10 次 Jacobi 迭代，次数
        //    再少会看到烟从边上漏出去。
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let i = x + y * w;
                self.div[i] =
                    -0.5 * (self.u[i + 1] - self.u[i - 1] + self.v[i + w] - self.v[i - w]);
                self.tmp[i] = 0.0;
            }
        }
        for _ in 0..10 {
            for y in 1..h - 1 {
                for x in 1..w - 1 {
                    let i = x + y * w;
                    self.tmp[i] = (self.div[i]
                        + self.tmp[i - 1]
                        + self.tmp[i + 1]
                        + self.tmp[i - w]
                        + self.tmp[i + w])
                        * 0.25;
                }
            }
        }
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let i = x + y * w;
                self.u[i] -= 0.5 * (self.tmp[i + 1] - self.tmp[i - 1]);
                self.v[i] -= 0.5 * (self.tmp[i + w] - self.tmp[i - w]);
            }
        }

        // ⑤ 染料跟着流场走。
        self.d0.copy_from_slice(&self.d);
        advect(&self.u, &self.v, &self.d0, &mut self.d, w, h, dt * 8.0);

        // ⑥ 位移场：也自输运，再按速度累积。纹理坐标被它偏移，
        //    于是"烟丝被卷起来"这件事发生在纹理层而不只是密度层。
        self.ox0.copy_from_slice(&self.ox);
        self.oy0.copy_from_slice(&self.oy);
        advect(&self.u, &self.v, &self.ox0, &mut self.ox, w, h, dt * 8.0);
        advect(&self.u, &self.v, &self.oy0, &mut self.oy, w, h, dt * 8.0);
        for i in 0..self.ox.len() {
            self.ox[i] = ((self.ox[i] + self.u[i] * dt * 8.0) * 0.994).clamp(-10.0, 10.0);
            self.oy[i] = ((self.oy[i] + self.v[i] * dt * 8.0) * 0.994).clamp(-10.0, 10.0);
        }

        // ⑦ 源与耗散：**贴着轮廓**补充（由噪声调制的目标密度），外部按方向
        //    衰减 —— 上方衰减慢（烟往上飘得远），侧下方快。
        //
        //    发射区只覆盖轮廓附近的窄带。栏体高约 100 px（72 的图标加内边距），
        //    若整块内部都发射，烟会被浮力顶到上沿、困在边界里，只看得见
        //    沿顶边的一条细线，漏到外面的部分还是断的。内部留空不影响观感
        //    （被栏体挡住），外面那圈才需要完整。
        let ix0 = self.margin.0 / CELL;
        let iy0 = self.margin.1 / CELL;
        let ix1 = w as f32 - self.margin.0 / CELL;
        let iy1 = h as f32 - self.margin.1 / CELL;
        let dk_top = tune.decay_top();
        let dk_side = tune.decay_side();
        for y in 0..h {
            for x in 0..w {
                let i = x + y * w;
                let (xf, yf) = (x as f32, y as f32);
                if xf >= ix0 && xf < ix1 && yf >= iy0 && yf < iy1 {
                    // 轮廓窄带的权重（同一张掩码，绘制阶段还要再用一次）。
                    let band = self.edge[i];
                    let target = band
                        * (0.12 + 0.85 * noise.source(0, xf * 1.6 + self.seed * 2.0, yf * 1.6, t))
                        * tune.density;
                    // 目标为 0 的地方（栏体深处）也走同一条收敛式：染料会
                    // 慢慢散掉，不会在内部积成一块永不消退的淤积。
                    //
                    // 0.10 的收敛速度。窄带发射时，流场一拍就能把带里的染料
                    // 扫走（|v| ≈ 7 格/帧），比率再低会一直半饱和，画出来
                    // 是一圈灰紫的淡痕。
                    self.d[i] += (target - self.d[i]) * 0.10;
                } else if yf < iy0 {
                    self.d[i] *= dk_top;
                } else {
                    self.d[i] *= dk_side;
                }
            }
        }
    }

    /// 把网格画成预乘 RGBA 位图。
    ///
    /// 返回 `(宽, 高)`（像素）。位图覆盖的范围是
    /// `网格 * CELL`，即栏体外扩 `margin`。
    ///
    /// 三步：算每格 alpha（密度 × 胶囊距离场 × 上下遮罩）→ 双线性放大到
    /// 像素 → 三层纹理取"最大不透明度"合成并调色。
    pub fn raster(&mut self, noise: &Noise, base: Rgb, t: f32) -> (usize, usize) {
        let (w, h) = (self.w, self.h);
        if w < 4 || h < 4 {
            return (0, 0);
        }
        // ① 每格 alpha：密度 × 轮廓窄带（掩码在 `ensure()` 里算好，源与
        //    绘制共用同一条带子，两处不会各调各的。
        for i in 0..self.al.len() {
            let a = self.d[i].min(1.0);
            self.al[i] = (a * 2.2).min(1.0) * self.edge[i];
        }

        // ② 光栅化
        let pw = (w as f32 * CELL / RES as f32).ceil() as usize;
        let ph = (h as f32 * CELL / RES as f32).ceil() as usize;
        if self.pw != pw || self.ph != ph {
            self.pw = pw;
            self.ph = ph;
            self.px.clear();
            self.px.resize(pw * ph * 4, 0);
        }
        // 亮部与暗部：亮部偏白（同一色相加亮度），暗部压到半亮。
        // 只用"密度"调亮度会让烟看起来像单色玻璃，加上这一层才有体积感。
        let light = [
            (base[0] * 1.55 + 30.0).min(255.0),
            (base[1] * 1.5 + 18.0).min(255.0),
            (base[2] * 1.55 + 30.0).min(255.0),
        ];
        let dark = [base[0] * 0.5, base[1] * 0.5, base[2] * 0.5];
        // 三层的缩放 / 漂移速度 / 不透明度。改动任何一个都会让烟絮的层次
        // 变平。
        const LAYERS: [(usize, f32, f32, f32, f32); 3] = [
            (0, 1.55, 3.0, -6.0, 0.95),
            (1, 1.08, -4.0, -4.0, 0.75),
            (2, 2.16, 2.0, -8.0, 0.60),
        ];
        let (al, ox, oy) = (&self.al, &self.ox, &self.oy);
        let px = &mut self.px;
        for py in 0..ph {
            for pxx in 0..pw {
                let out = (py * pw + pxx) * 4;
                let cx = (pxx as f32 + 0.5) * RES as f32 / CELL - 0.5;
                let cy = (py as f32 + 0.5) * RES as f32 / CELL - 0.5;
                let a0 = bilinear(al, w, h, cx, cy);
                if a0 < 0.004 {
                    px[out..out + 4].fill(0);
                    continue;
                }
                let dx = bilinear(ox, w, h, cx, cy) * CELL;
                let dy = bilinear(oy, w, h, cx, cy) * CELL;
                let wx = pxx as f32 * RES as f32 - dx;
                let wy = py as f32 * RES as f32 - dy;
                let mut rest = 1.0;
                let mut top = 0.0;
                for &(k, scale, vx, vy, op) in &LAYERS {
                    let pr = noise.tap(k, wx / scale - vx * t, wy / scale - vy * t);
                    rest *= 1.0 - op * pr;
                    if k == 0 {
                        top = pr;
                    }
                }
                let dens = 1.0 - rest;
                let a = (dens * a0 * 0.95).min(0.85);
                let m = (dens * 1.1 + top * 0.2).min(1.0);
                let r = (dark[0] + (light[0] - dark[0]) * m) * a;
                let g = (dark[1] + (light[1] - dark[1]) * m) * a;
                let b = (dark[2] + (light[2] - dark[2]) * m) * a;
                // 预乘：分量不超过 alpha，所以先乘 a 再取整。
                px[out] = (r + 0.5) as u8;
                px[out + 1] = (g + 0.5) as u8;
                px[out + 2] = (b + 0.5) as u8;
                px[out + 3] = (a * 255.0 + 0.5) as u8;
            }
        }
        (pw, ph)
    }

    /// 第二位颜色（沿栏体从左到右渐变）与 >100% 的不透明度增益。
    ///
    /// 在位图上做后处理：两个效果都只看最终 alpha，合成阶段做比采样阶段做
    /// 省一半分支。
    ///
    /// `bar` 是栏体在**位图坐标系**里的 `(左, 宽)`；`gain` < 1 无效
    /// （低于 100% 的不透明度交给绘制时的 tint alpha）。
    pub fn post(&mut self, base: Rgb, second: Option<Rgb>, bar: (f32, f32), gain: f32) {
        let (pw, ph) = (self.pw, self.ph);
        if pw == 0 || ph == 0 {
            return;
        }
        let two = second.is_some();
        if !two && gain <= 1.0 {
            return;
        }
        // 亮度基准：换色时保持每个像素的明度，否则第二色比主色暗的地方
        // 会突然"陷下去"一块。
        let lum = |c: Rgb| (0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]).max(1.0);
        let lum1 = lum(base);
        let c2 = second.unwrap_or([0.0; 3]);
        let gain = gain.max(1.0);
        for py in 0..ph {
            for pxx in 0..pw {
                let out = (py * pw + pxx) * 4;
                let a = self.px[out + 3];
                if a == 0 {
                    continue;
                }
                let mut r = f32::from(self.px[out]);
                let mut g = f32::from(self.px[out + 1]);
                let mut b = f32::from(self.px[out + 2]);
                if two {
                    // 该像素在栏体上的相对位置；栏体外按 0 / 1 夹住。
                    let k = ((pxx as f32 - bar.0) / bar.1.max(1.0)).clamp(0.0, 1.0);
                    if k > 0.004 {
                        let inv = 255.0 / f32::from(a);
                        let (ur, ug, ub) = (r * inv, g * inv, b * inv);
                        let sl = (0.3 * ur + 0.59 * ug + 0.11 * ub) / lum1;
                        let af = f32::from(a) / 255.0;
                        r = (ur + ((c2[0] * sl).min(255.0) - ur) * k) * af;
                        g = (ug + ((c2[1] * sl).min(255.0) - ug) * k) * af;
                        b = (ub + ((c2[2] * sl).min(255.0) - ub) * k) * af;
                    }
                }
                let mut na = f32::from(a);
                if gain > 1.0 {
                    na = (na * gain).min(255.0);
                    let sc = na / f32::from(a);
                    r *= sc;
                    g *= sc;
                    b *= sc;
                }
                self.px[out] = r.min(na) as u8;
                self.px[out + 1] = g.min(na) as u8;
                self.px[out + 2] = b.min(na) as u8;
                self.px[out + 3] = na as u8;
            }
        }
    }

    /// 诊断用的场统计：`(染料最大值, 染料均值, 速度最大值, alpha 最大值)`。
    ///
    /// 排查"烟看不见"时唯一有用的三件事 —— 染料没建起来（源/输运问题）、
    /// 速度爆了（数值不稳）、还是遮罩把它掐掉了。
    #[must_use]
    pub fn stats(&self) -> (f32, f32, f32, f32) {
        let max = |f: &Vec<f32>| f.iter().copied().fold(0.0_f32, f32::max);
        let mean = if self.d.is_empty() {
            0.0
        } else {
            self.d.iter().sum::<f32>() / self.d.len() as f32
        };
        let speed = self
            .u
            .iter()
            .zip(self.v.iter())
            .fold(0.0_f32, |m, (u, v)| m.max((u * u + v * v).sqrt()));
        (max(&self.d), mean, speed, max(&self.al))
    }

    /// 预乘 RGBA 位图（`pw * ph * 4` 字节）。
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.px
    }
}

/// 轮廓窄带掩码：栏体边缘附近为 1，往内 3 格、往外 `reach` 格淡到 0。
///
/// 形状是跑道：两端半圆、中间直线。
/// 用"到中轴线的距离减半径"求带符号距离，比矩形距离场更接近 D2D 的
/// `DrawRoundedRectangle(rad + d)` 那一圈圈描边。
fn build_edge(w: usize, h: usize, margin: (f32, f32), thickness: f32) -> Vec<f32> {
    let iy0 = margin.1 / CELL;
    let iy1 = h as f32 - margin.1 / CELL;
    let ih = (iy1 - iy0).max(1.0);
    let ix0 = margin.0 / CELL;
    let ix1 = w as f32 - margin.0 / CELL;
    let rr = ih * 0.5;
    let cyc = iy0 + rr;
    // 内侧衰减宽度（格）。3 格 = 15 px，正好是栏体圆角的一半左右：
    // 再宽会把烟推到图标区，再窄会在栏体边缘留下一道生硬的分界。
    const INNER: f32 = 3.0;
    let mut out = vec![0.0_f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let (xf, yf) = (x as f32, y as f32);
            let qx = (ix0 + rr - xf).max(xf - (ix1 - rr)).max(0.0);
            let qy = (yf - cyc).abs();
            let sd = (qx * qx + qy * qy).sqrt() - rr;
            // 上方 6 格、下方 3 格 —— 光往上散，这是"烟"不是"雾"。
            let reach = if yf < cyc { 6.0 } else { 3.0 } * thickness;
            let t = if sd <= 0.0 {
                (1.0 + sd / INNER).clamp(0.0, 1.0)
            } else {
                (1.0 - sd / reach).clamp(0.0, 1.0)
            };
            out[y * w + x] = t * t * (3.0 - 2.0 * t);
        }
    }
    out
}

/// 双线性采样。`cx` / `cy` 会被夹到 `[0, w-1.001]`，调用方不用自己防越界。
fn bilinear(f: &[f32], w: usize, h: usize, cx: f32, cy: f32) -> f32 {
    let cx = cx.clamp(0.0, w as f32 - 1.001);
    let cy = cy.clamp(0.0, h as f32 - 1.001);
    let x0 = cx as usize;
    let y0 = cy as usize;
    let j = x0 + y0 * w;
    let fx = cx - x0 as f32;
    let fy = cy - y0 as f32;
    (f[j] * (1.0 - fx) + f[j + 1] * fx) * (1.0 - fy)
        + (f[j + w] * (1.0 - fx) + f[j + w + 1] * fx) * fy
}

/// 半拉格朗日输运：沿 `-dt * (u,v)` 回溯，双线性取值。
///
/// `src` 与 `dst` 不能是同一块内存；`vel` 是**回溯用的**速度场（与 `src`
/// 相同或更早一帧都可以，见 [`Fluid::step`] 第 ③ 步的注释）。
fn advect(vel_u: &[f32], vel_v: &[f32], src: &[f32], dst: &mut [f32], w: usize, h: usize, dt: f32) {
    for y in 0..h {
        for x in 0..w {
            let i = x + y * w;
            let mut px = x as f32 - dt * vel_u[i];
            let mut py = y as f32 - dt * vel_v[i];
            px = px.clamp(0.0, w as f32 - 1.001);
            py = py.clamp(0.0, h as f32 - 1.001);
            let x0 = px as usize;
            let y0 = py as usize;
            let j = x0 + y0 * w;
            let fx = px - x0 as f32;
            let fy = py - y0 as f32;
            dst[i] = (src[j] * (1.0 - fx) + src[j + 1] * fx) * (1.0 - fy)
                + (src[j + w] * (1.0 - fx) + src[j + w + 1] * fx) * fy;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Edge};

    #[test]
    fn palette_matches_upstream_order() {
        // 索引会写进 ini，顺序就是契约。
        assert_eq!(PALETTE[0], [88.0, 52.0, 160.0], "0 = violet");
        assert_eq!(PALETTE[7], [150.0, 108.0, 10.0], "7 = gold");
    }

    #[test]
    fn noise_alpha_is_normalized() {
        let n = Noise::build_sized(32);
        for k in 0..3 {
            for &a in &n.alpha[k] {
                assert!((0.0..=1.0).contains(&a), "alpha 越界: {a}");
            }
            let m = n.mean(k);
            assert!(m > 0.05 && m < 0.95, "第 {k} 层均值异常: {m}");
        }
    }

    #[test]
    fn noise_source_wraps_on_the_tile() {
        let n = Noise::build_sized(48);
        // 每个单元横跨 0.55 个纹理像素，所以一个平铺周期 = n / 0.55 格。
        let period = n.size() as f32 / 0.55;
        for k in 0..3 {
            let a = n.source(k, 3.0, 7.0, 0.0);
            let b = n.source(k, 3.0 + period, 7.0, 0.0);
            assert!((a - b).abs() < 1e-3, "第 {k} 层平铺有接缝: {a} vs {b}");
        }
    }

    /// 与真实栏体同量级的网格：48 px 高的栏体 + 默认边距（34 px 上下）
    /// 正好 24 格高。**不能图快用 12 格**：那样 `iy1 < iy0`（胶囊上下
    /// 颠倒），密度循环会把每一格都当"外部"，跑完一片透明 —— 这条
    /// 不是假想的坑，第一版测试就是这么写错的。
    fn warmed(noise: &Noise, frames: usize) -> Fluid {
        let tune = Tuning {
            enabled: true,
            ..Tuning::default()
        };
        let mut fl = Fluid::new();
        fl.ensure(
            52,
            24,
            0.0,
            (tune.margin_x(), tune.margin_y()),
            tune.thickness,
        );
        for i in 0..frames {
            fl.step(
                noise,
                &tune,
                Frame {
                    dt: 1.0 / 60.0,
                    time: i as f32 / 60.0,
                    // 光标放到网格外：这条测的是"没被推过的稳态"。
                    mouse: (-99.0, -99.0),
                    mouse_vel: (0.0, 0.0),
                },
            );
        }
        fl
    }

    #[test]
    fn fluid_stays_bounded() {
        let noise = Noise::build_sized(32);
        let tune = Tuning {
            enabled: true,
            ..Tuning::default()
        };
        let mut fl = Fluid::new();
        fl.ensure(
            52,
            24,
            0.0,
            (tune.margin_x(), tune.margin_y()),
            tune.thickness,
        );
        for i in 0..120 {
            fl.step(
                &noise,
                &tune,
                Frame {
                    dt: 1.0 / 60.0,
                    time: i as f32 / 60.0,
                    // 光标压在胶囊中间，顺带覆盖鼠标推力分支。
                    mouse: (26.0, 12.0),
                    mouse_vel: (0.5, -0.5),
                },
            );
        }
        for (name, f) in [("u", &fl.u), ("v", &fl.v), ("d", &fl.d)] {
            for &v in f.iter() {
                assert!(v.is_finite(), "{name} 出现 NaN/Inf");
                assert!(v.abs() < 400.0, "{name} 发散: {v}");
            }
        }
        assert!(fl.d.iter().all(|v| *v >= 0.0), "染料不该为负");
        // 浮力是朝上的（y 减小），所以染料上缘一定比胶囊内部浓：
        // 这条能抓住"把浮力符号写反"这种一眼看不出来的错。
        let sum = |range: std::ops::Range<usize>| fl.d[range].iter().sum::<f32>();
        let above = sum(0..52 * 6);
        let inside = sum(52 * 8..52 * 16);
        assert!(above > 0.0, "胶囊上方应当有飘上去的烟");
        assert!(inside > 0.0, "胶囊内部应当有源");
    }

    #[test]
    fn raster_is_premultiplied() {
        let noise = Noise::build_sized(24);
        let mut fl = warmed(&noise, 40);
        let (pw, ph) = fl.raster(&noise, PALETTE[0], 0.0);
        assert!(pw > 0 && ph > 0);
        assert_eq!(fl.pixels().len(), pw * ph * 4);
        let mut lit = 0;
        for px in fl.pixels().as_chunks::<4>().0 {
            let a = px[3];
            assert!(px[0] <= a && px[1] <= a && px[2] <= a, "未预乘: {px:?}");
            if a > 8 {
                lit += 1;
            }
        }
        // 烟的发射区是**一圈**窄带（见 `build_edge`），不是整块，所以
        // 亮像素只占一成多。低于 1/8 说明窄带没建起来或密度没收敛。
        assert!(
            lit > pw * ph / 8,
            "只有 {lit}/{} 个像素不透明，遮罩或密度算错了",
            pw * ph
        );
    }

    #[test]
    fn post_keeps_premultiplied_and_brightens() {
        let noise = Noise::build_sized(24);
        let mut fl = warmed(&noise, 40);
        let (pw, _) = fl.raster(&noise, PALETTE[0], 0.0);
        let max_a = fl
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| p[3])
            .max()
            .unwrap_or(0);
        fl.post(PALETTE[0], Some(PALETTE[2]), (0.0, pw as f32), 1.5);
        for px in fl.pixels().as_chunks::<4>().0 {
            let a = px[3];
            assert!(px[0] <= a && px[1] <= a && px[2] <= a, "后处理破坏预乘");
        }
        let after = fl
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| p[3])
            .max()
            .unwrap_or(0);
        assert!(after > max_a, "125% 以上的档位必须提亮 alpha");
    }

    #[test]
    fn tuning_reads_config_tables() {
        let mut cfg = Config::defaults(96);
        cfg.apply_i32(Key::SMOKE, 1);
        cfg.edge = Edge::Bottom;
        let t = Tuning::from_config(&cfg, PALETTE[3]);
        assert!(t.enabled);
        assert!((t.thickness - 1.0).abs() < 1e-6);
        assert!(t.clouds);

        // 顶端停靠：浮力方向与上缘衰减都不成立，直接不启用。
        cfg.edge = Edge::Top;
        assert!(!Tuning::from_config(&cfg, PALETTE[3]).enabled);
        // 关掉开关同理。
        cfg.edge = Edge::Bottom;
        cfg.apply_i32(Key::SMOKE, 0);
        assert!(!Tuning::from_config(&cfg, PALETTE[3]).enabled);
    }

    #[test]
    fn tone_lightens_instead_of_whitening() {
        let mut cfg = Config::defaults(96);
        cfg.apply_i32(Key::SMOKE, 1);
        let base = Tuning::from_config(&cfg, PALETTE[0]).color_at(0, 0.0);
        cfg.apply_i32(Key::SMOKE_TONE, 1);
        let glow = Tuning::from_config(&cfg, PALETTE[0]).color_at(0, 0.0);
        assert!(glow[2] > base[2] && glow[0] > base[0]);
        // 色相关系不变：紫色仍然是"蓝多于绿"。
        assert!(glow[2] > glow[1]);
    }

    #[test]
    fn rainbow_runs_a_full_circle_in_ninety_seconds() {
        let a = rainbow(0.0);
        let b = rainbow(90.0);
        for i in 0..3 {
            assert!((a[i] - b[i]).abs() < 1e-3, "90 秒应当回到同一点");
        }
        assert!(rainbow(22.5) != a, "四分之一圈必须换色");
    }
}
