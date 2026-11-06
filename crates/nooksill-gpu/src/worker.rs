//! 后台线程：把会阻塞的 Win32 / IO 调用挪出 UI 线程。
//!
//! Dock 的 UI 线程同时负责消息泵和出帧，**任何一次同步系统调用都直接变成
//! 掉帧**。这几条路径以前全在 UI 线程上：
//!
//! | 路径 | 阻塞点 | 频率 |
//! |---|---|---|
//! | 取图标 | `SHGetFileInfoW` + `IImageList::GetIcon` + `GetDIBits` | 启动 |
//! | 运行状态 | `EnumWindows` + 每个窗口 `OpenProcess` | 1.2 s |
//! | 遮挡检测 | `EnumWindows` + 每个窗口 8 次查询 | 100 ms（智能隐藏） |
//! | 写 ini | 读文件 + UTF-16 全量回写 | 拖拽每一步 / 改设置 |
//! | 启动应用 | `ShellExecuteW` | 点击 |
//!
//! 现在每类各有一个专用线程，结果经 `std::sync::mpsc` 回传；UI 线程只做
//! `try_recv`，**永远不等**。
//!
//! # 为什么是专用线程而不是线程池
//!
//! 四类工作的性质不同：取图标必须**串行**（shell 的 COM 对象不能并发访问），
//! 扫描是**周期**的，写盘要**保序**，启动是**一次性**的。塞进一个池子只会
//! 让"保存配置"排在"取 20 个图标"后面。
//!
//! # 为什么用 std 的 mpsc 而不是 crossbeam
//!
//! 队列深度是个位数（启动时几十个图标请求，之后基本是空的），`mpsc` 的
//! 无锁快路径完全够用，少一个依赖。
//!
//! # 线程里绝对不做的事
//!
//! - 不碰 `egui::Context`（纹理注册与图集上传必须由 UI 线程做）；
//! - 不创建窗口、不改窗口状态；
//! - 不持有 `Dock` 的引用（只处理自己那份数据的副本）。
//!
//! `wgpu` compute 是例外：`Device` / `Queue` 是 `Send + Sync`，图标圆角
//! 遮罩在工作线程提交 compute 并回读，不经过 egui。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use nooksill_core::icon::{apply_round_mask, fit_icon, FitMode, IconFit};
use nooksill_core::ini::Ini;
use nooksill_core::item::{Item, ItemKind};
use nooksill_core::layout::Rect;
use nooksill_core::running;
use nooksill_core::store;
use nooksill_platform::app_window;
use nooksill_platform::icon::{self, IconBitmap};
use nooksill_platform::launch;
use nooksill_platform::occlusion;
use nooksill_platform::win::ensure_com;
use rayon::prelude::*;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::gpu::Gpu;
use crate::shaders::icon::IconShader;

/// "后台有结果可取"的自定义消息。
///
/// 主循环看到它只**打断等待**，不强制出帧 —— 否则每次扫描结果都会多渲染
/// 一帧，"空闲 0 帧"的产品承诺就没了。
pub const WM_APP_WAKE: u32 = WM_APP + 1;

/// UI 线程的唤醒句柄，可跨线程复制。
///
/// 没有它，主循环就只能靠轮询去发现"后台有结果了"；有了它，
/// `MsgWaitForMultipleObjectsEx` 会在结果就绪时立刻返回，空闲时仍然
/// 一直睡着。
#[derive(Clone, Copy, Debug)]
pub struct Waker(isize);

impl Waker {
    #[must_use]
    pub fn new(hwnd: HWND) -> Self {
        Self(hwnd.0 as isize)
    }

    pub(crate) fn wake(self) {
        // 失败只说明窗口已销毁（进程正在退出），不必处理。
        let hwnd = HWND(self.0 as *mut core::ffi::c_void);
        // SAFETY: `PostMessageW` 对已销毁的句柄返回错误而不是崩溃；
        // 这里不检查返回值是刻意的（见上）。
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_APP_WAKE, WPARAM(0), LPARAM(0));
        }
    }
}

// ============================ 图标 ============================

/// 一次取图标的请求。字段全是自足的：工作线程拿不到 `Dock`。
#[derive(Clone, Debug)]
pub struct IconRequest {
    pub index: usize,
    pub kind: ItemKind,
    pub path: String,
    /// 自定义图标文件（`.ico`）。空 = 从 `path` 取。
    pub icon_src: String,
    /// 图标填充方式（0 = 等比留白，见 `FitBitmap`）。
    pub fit: i32,
}

/// 处理完、可以直接上传纹理的图标。
///
/// 缩放与圆角遮罩已经在工作线程做完（可并行），UI 线程只剩
/// `load_texture` —— 那一步必须在有 `egui::Context` 的线程上做。
#[derive(Clone, Debug)]
pub struct ReadyIcon {
    pub index: usize,
    /// 请求时的条目路径。UI 线程据此核对"这个下标还是不是同一个条目" ——
    /// 排队期间用户可能增删条目，下标会指向别人。
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub fit: IconFit,
}

/// 取图标的工作线程。
#[derive(Debug)]
pub struct IconLoader {
    tx: Option<Sender<IconRequest>>,
    rx: Receiver<ReadyIcon>,
    /// 只用来给线程起名以便调试；退出时**不 join**（见 `Drop`）。
    _thread: JoinHandle<()>,
}

impl IconLoader {
    /// 起一个取图标线程。线程创建失败时返回错误，调用方可以退回同步加载。
    pub fn spawn(waker: Waker, gpu: Option<&Gpu>) -> std::io::Result<Self> {
        let shader: Option<Arc<IconShader>> =
            gpu.map(|g| Arc::new(IconShader::new(&g.device, &g.queue)));
        let (req_tx, req_rx) = mpsc::channel::<IconRequest>();
        let (res_tx, res_rx) = mpsc::channel::<ReadyIcon>();
        let thread = thread::Builder::new()
            .name("nooksill-icons".to_owned())
            .spawn(move || icon_thread(req_rx, res_tx, waker, shader))?;
        Ok(Self {
            tx: Some(req_tx),
            rx: res_rx,
            _thread: thread,
        })
    }

    /// 排一个取图标任务。队满或线程已退出时静默丢弃 —— 图标取不到只是
    /// 退回首字占位，不该影响别的功能。
    pub fn submit(&self, req: IconRequest) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(req);
        }
    }

    /// 取走本轮已完成的图标（不阻塞）。
    pub fn drain(&self) -> Vec<ReadyIcon> {
        self.rx.try_iter().collect()
    }
}

impl Drop for IconLoader {
    fn drop(&mut self) {
        // 丢掉发送端 → 工作线程的 `recv` 返回 Err → 处理完手上这一张就退出。
        // **不 join**：shell 调用可能卡在慢速路径（网络盘、被杀毒软件拦），
        // 退出时等它会把整个进程拖住。线程只碰自己的数据，detach 是安全的。
        self.tx = None;
    }
}

fn icon_thread(
    rx: Receiver<IconRequest>,
    tx: Sender<ReadyIcon>,
    waker: Waker,
    shader: Option<Arc<IconShader>>,
) {
    // 这个线程要碰 shell 的 COM 对象（`SHGetImageList` 返回的 `IImageList`），
    // 必须**在本线程**初始化 COM。`ensure_com` 是每线程一次，任意线程可调。
    ensure_com();

    loop {
        // 攒一批再处理：启动时会一次性排进 N 个，逐条做等于把并行度扔掉。
        let Ok(first) = rx.recv() else { return };
        let mut batch = vec![first];
        while batch.len() < BATCH {
            match rx.try_recv() {
                Ok(next) => batch.push(next),
                Err(_) => break,
            }
        }

        // 第一步**串行**：shell 调用（`GetIcon` 返回的是共享图像列表里的
        // 对象，并发访问没有保证）。
        let fetched: Vec<(IconRequest, Option<IconBitmap>)> = batch
            .into_iter()
            .map(|req| {
                let bmp = fetch_icon(&req);
                (req, bmp)
            })
            .collect();

        // 第二步**并行**：缩放判定与圆角遮罩是纯 CPU，每张 256×256 要走
        // 6 万多个像素，20 张就是 130 万次循环 —— 这是本项目里唯一值得
        // 上多核的地方。
        let ready: Vec<ReadyIcon> = fetched
            .into_par_iter()
            .filter_map(|(req, bmp)| bmp.map(|b| process(&req, b, shader.as_deref())))
            .collect();

        let produced = !ready.is_empty();
        for icon in ready {
            if tx.send(icon).is_err() {
                return; // UI 线程没了，收工
            }
        }
        if produced {
            waker.wake();
        }
    }
}

/// 一批最多攒多少个。8 足够填满常见核心数，又不会让第一批等太久。
const BATCH: usize = 8;

/// 走 shell 取图标。
fn fetch_icon(req: &IconRequest) -> Option<IconBitmap> {
    // 自定义图标优先：`.ico` 交给 shell 的图像列表，拿到的是文件本身那张图。
    // 取不到再退回条目自己的图标 —— 用户给错了文件不该让图标整个消失。
    let custom = (!req.icon_src.is_empty())
        .then(|| icon::shell_icon(&req.icon_src))
        .flatten();
    if custom.is_some() {
        return custom;
    }
    match req.kind {
        // 分割线不画图标；网址要联网抓 favicon，还没做。
        ItemKind::Separator | ItemKind::Url => None,
        ItemKind::Trash => icon::shell_icon("shell:RecycleBinFolder"),
        _ => icon::shell_icon(&req.path).or_else(|| icon::resource_icon(&req.path, 0)),
    }
}

/// 纯 CPU 的那一半：判定填充方式、按需做圆角遮罩。
fn process(req: &IconRequest, bmp: IconBitmap, shader: Option<&IconShader>) -> ReadyIcon {
    let fit = fit_icon(
        &bmp.rgba,
        bmp.width,
        bmp.height,
        FitMode::from_code(req.fit),
    );
    let mut rgba = bmp.rgba;
    if let Some(r) = fit.rounded {
        let bbox = (fit.bbox.x0, fit.bbox.y0, fit.bbox.x1, fit.bbox.y1);
        let masked = shader.and_then(|s| s.round_mask(&rgba, bmp.width, bmp.height, bbox, r));
        match masked {
            Some(out) => rgba = out,
            None => apply_round_mask(&mut rgba, bmp.width, fit.bbox, r),
        }
    }
    ReadyIcon {
        index: req.index,
        path: req.path.clone(),
        width: bmp.width,
        height: bmp.height,
        rgba,
        fit,
    }
}

// ============================ 扫描 ============================

/// 扫描线程的输入。只在参数变化时发一次，避免每轮灌队列。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScanInput {
    /// 需要检测遮挡的屏幕矩形；`None` = 不检测（非智能隐藏模式）。
    pub cover: Option<Rect>,
    /// 是否需要扫描"哪些应用在运行"。
    ///
    /// 指示点与"正在运行"气泡都关掉时它是 false —— 那时每 1.2 秒枚举一遍
    /// 全部顶层窗口纯属白烧：忙一点的桌面上是 50 个窗口 × 六七次查询，
    /// 外加每个进程一次 `OpenProcess`。开着 Dock 什么也不显示，CPU 却
    /// 掉不下来，用户只会觉得"这软件很重"。
    pub want_running: bool,
}

/// 扫描线程的输出。
#[derive(Clone, Debug, Default)]
pub struct ScanOutput {
    /// 运行中的 exe 小写路径集合。
    pub running: Option<HashSet<String>>,
    /// 该矩形是否被普通窗口盖住。
    pub covered: Option<bool>,
}

/// 运行状态扫描周期。
///
/// 更短没有意义：用户点开一个程序的感知延迟远大于这个量级，而每轮要
/// `EnumWindows` + 为每个窗口 `OpenProcess`。
const RUNNING_EVERY: Duration = Duration::from_millis(1200);
/// 遮挡检测周期：自动隐藏本身按 100 ms 轮询光标，遮挡判定跟它同频。
const COVER_EVERY: Duration = Duration::from_millis(100);

/// 周期扫描线程：运行状态 + 遮挡检测。
///
/// 两者都是"枚举所有顶层窗口"，放在同一个线程里可以避免两条线程同时
/// `EnumWindows` 抢同一个系统锁。
#[derive(Debug)]
pub struct Scanner {
    tx: Option<Sender<ScanInput>>,
    rx: Receiver<ScanOutput>,
    _thread: JoinHandle<()>,
}

impl Scanner {
    /// `exclude` 是 Dock 自己的窗口句柄（`HWND` 不是 `Send`，这里存成
    /// `isize`，线程内再还原）。
    pub fn spawn(exclude: isize, waker: Waker) -> std::io::Result<Self> {
        let (in_tx, in_rx) = mpsc::channel::<ScanInput>();
        let (out_tx, out_rx) = mpsc::channel::<ScanOutput>();
        let thread = thread::Builder::new()
            .name("nooksill-scan".to_owned())
            .spawn(move || scan_thread(in_rx, out_tx, exclude, waker))?;
        Ok(Self {
            tx: Some(in_tx),
            rx: out_rx,
            _thread: thread,
        })
    }

    /// 更新扫描参数。
    pub fn update(&self, input: ScanInput) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(input);
        }
    }

    /// 取走本轮已完成的扫描结果（不阻塞）。
    pub fn drain(&self) -> Vec<ScanOutput> {
        self.rx.try_iter().collect()
    }
}

impl Drop for Scanner {
    fn drop(&mut self) {
        // 同 `IconLoader`：丢掉发送端即可，线程会在下一次超时后退出。
        self.tx = None;
    }
}

fn scan_thread(rx: Receiver<ScanInput>, tx: Sender<ScanOutput>, exclude: isize, waker: Waker) {
    let mut input = ScanInput::default();
    // 首轮立刻扫一次：指示点不该等到 1.2 秒后才亮。
    let mut next_running = Instant::now();
    let mut next_cover = Instant::now();

    loop {
        // 先看看到点没有，再决定睡多久。
        //
        // 顺序很重要：如果先等输入再检查截止时刻，持续到来的参数更新会让
        // 循环一直 `continue`，扫描永远轮不上（实测表现为"悬停动画期间
        // 指示点不再更新"）。
        let now = Instant::now();
        let mut out = ScanOutput::default();
        if now >= next_running {
            next_running = now + RUNNING_EVERY;
            if !input.want_running {
                // 不需要就跳过，但**保留节拍**：重新打开时最多等 1.2 秒
                // 就有第一份结果，不必额外记一个"刚被打开"的状态。
            } else {
                // `running_apps()` 返回 `HashMap<String, HWND>`，`HWND` 不是
                // `Send`，所以必须在这里就转成集合再回传。
                out.running = Some(app_window::running_apps().into_keys().collect());
            }
        }
        if now >= next_cover {
            next_cover = now + COVER_EVERY;
            if let Some(rect) = input.cover {
                // 还原成 HWND 只为了"别把自己的窗口当成遮挡物"。
                let me = HWND(exclude as *mut core::ffi::c_void);
                out.covered = Some(occlusion::any_window_covers(rect, &[me]));
            }
        }
        if out.running.is_some() || out.covered.is_some() {
            if tx.send(out).is_err() {
                return;
            }
            waker.wake();
        }

        // 睡到最近的截止时刻。期间来的参数更新直接覆盖 ——
        // "最新值有效"，不需要排队。
        let wait = next_running
            .min(next_cover)
            .saturating_duration_since(Instant::now());
        match rx.recv_timeout(wait) {
            Ok(next) => input = next,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

// ============================ 写盘 ============================

/// 一次写盘请求。`None` 表示"这次不动那一段"。
#[derive(Clone, Debug)]
pub struct WriteJob {
    pub items: Option<Vec<Item>>,
    pub config: Option<Box<nooksill_core::config::Config>>,
    /// 计时器 / 待办（`[Tool]` 段）。带一个时间基准 ——
    /// `Remain` 要按"提交这一刻"的剩余量算，不能在写盘线程里重新取时间
    /// （那时可能已经过去几十毫秒，秒表会有可见跳变）。
    pub tool: Option<(Box<nooksill_core::tool::ToolState>, i64)>,
}

/// 写 ini 的工作线程。串行执行，天然保序。
#[derive(Debug)]
pub struct DiskWriter {
    tx: Option<Sender<WriteJob>>,
    rx: Receiver<Result<(), String>>,
    _thread: JoinHandle<()>,
}

impl DiskWriter {
    pub fn spawn(ini: PathBuf, waker: Waker) -> std::io::Result<Self> {
        let (job_tx, job_rx) = mpsc::channel::<WriteJob>();
        let (res_tx, res_rx) = mpsc::channel::<Result<(), String>>();
        let thread = thread::Builder::new()
            .name("nooksill-disk".to_owned())
            .spawn(move || disk_thread(ini, job_rx, res_tx, waker))?;
        Ok(Self {
            tx: Some(job_tx),
            rx: res_rx,
            _thread: thread,
        })
    }

    pub fn submit(&self, job: WriteJob) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(job);
        }
    }

    /// 取走已完成的写盘结果（不阻塞）。
    pub fn drain(&self) -> Vec<Result<(), String>> {
        self.rx.try_iter().collect()
    }
}

impl Drop for DiskWriter {
    fn drop(&mut self) {
        self.tx = None;
    }
}

fn disk_thread(ini: PathBuf, rx: Receiver<WriteJob>, tx: Sender<Result<(), String>>, waker: Waker) {
    while let Ok(job) = rx.recv() {
        if tx.send(write(&ini, &job)).is_err() {
            return;
        }
        waker.wake();
    }
}

/// 读一次 → 改两段 → 原子写回。
///
/// **必须重新读**：设置窗口与条目编辑是两条路径，谁后用内存副本覆盖谁就
/// 会把对方的改动抹掉。
fn write(ini: &Path, job: &WriteJob) -> Result<(), String> {
    let mut model = Ini::load(ini).map_err(|e| format!("读取 {} 失败：{e}", ini.display()))?;
    if let Some(items) = &job.items {
        store::save_items(items, &mut model);
    }
    if let Some(cfg) = &job.config {
        cfg.write_ini(&mut model);
    }
    if let Some((tool, now_ms)) = &job.tool {
        tool.save(&mut model, *now_ms);
    }
    model
        .save(ini)
        .map_err(|e| format!("写入 {} 失败：{e}", ini.display()))
}

// ============================ 启动应用 ============================

/// 一次"打开条目"的请求。
#[derive(Clone, Debug)]
pub struct OpenJob {
    pub item: Box<Item>,
    /// Shift+点击 / 中键：不管有没有在跑，都新起一个。
    pub force: bool,
    /// `ClickToggle=1`：已在前台则最小化。
    pub toggle: bool,
    /// `Browser` 键：网页应用用哪个浏览器（0 自动 / 1 Edge / 2 Chrome /
    /// 3 Firefox / 4 系统默认）。非网址条目忽略。
    pub browser: i32,
}

/// 启动应用的工作线程。
///
/// `ShellExecuteW` 冷启动时几十毫秒起步（要等 shell 找关联、拉起进程），
/// 这是点击卡顿的唯一来源。
#[derive(Debug)]
pub struct Opener {
    tx: Option<Sender<OpenJob>>,
    _thread: JoinHandle<()>,
}

impl Opener {
    pub fn spawn() -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel::<OpenJob>();
        let thread = thread::Builder::new()
            .name("nooksill-open".to_owned())
            .spawn(move || open_thread(rx))?;
        Ok(Self {
            tx: Some(tx),
            _thread: thread,
        })
    }

    pub fn submit(&self, job: OpenJob) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(job);
        }
    }
}

impl Drop for Opener {
    fn drop(&mut self) {
        self.tx = None;
    }
}

fn open_thread(rx: Receiver<OpenJob>) {
    while let Ok(job) = rx.recv() {
        let item = &job.item;
        let key = running::running_key(item);
        // 与左键点击同一条路径：已运行则激活，否则启动。`force` 跳过激活，
        // 否则"想开第二个实例"永远开不出来。
        //
        // 注意这里**分两步**而不是 `is_some_and(activate)`：`activate` 现在
        // 会区分"切过去了 / 找到了但系统拒绝置前"。后者如果按"没切成"处理
        // 去启动新实例，用户会看到两个窗口 —— 那是比"没反应"更糟的结果。
        if !job.force {
            if let Some(exe) = &key {
                let r = app_window::activate(exe, job.toggle);
                if trace() {
                    eprintln!("[launch] 激活 {} → {r:?}", item.name);
                }
                if r.is_running() {
                    continue;
                }
            }
        }
        if let Err(e) = launch::launch(item, job.browser) {
            eprintln!("启动「{}」失败：{e}", item.name);
            continue;
        }
        if trace() {
            eprintln!("[launch] 启动 {}（force={}）", item.name, job.force);
        }
        // 启动后把窗口带到前台。`ShellExecuteW` 是异步的：进程刚起来时窗口
        // 还不存在，而且新进程自己置前常常被前台锁拦住（Dock 窗口是
        // `WS_EX_NOACTIVATE`，点击不让它拿到前台权）。
        //
        // 用独立线程等而不是在本线程里 sleep：这是**串行**队列，用户连点
        // 三个图标时，一次 3 秒的等待会把后两个拖住。`force` 时不等 ——
        // "再开一个实例"本来就不该抢焦点。
        if !job.force {
            if let Some(exe) = key {
                let name = item.name.clone();
                std::thread::spawn(move || follow_launch(&name, &exe));
            }
        }
    }
}

/// 启动后等窗口出现，然后置前。最多等 [`FOLLOW_MS`]。
///
/// 只在新启动的情况下调用：应用自己会置前的情况占多数，这里兜的是
/// "窗口起来了但被前台锁挡在后面"那部分。
fn follow_launch(name: &str, exe: &str) {
    let start = Instant::now();
    while start.elapsed() < FOLLOW_MS {
        std::thread::sleep(std::time::Duration::from_millis(120));
        if app_window::find_window_of(exe).is_none() {
            continue;
        }
        let r = app_window::activate(exe, false);
        if trace() {
            eprintln!("[launch] {name} 的窗口已出现 → {r:?}");
        }
        return;
    }
    if trace() {
        eprintln!("[launch] {name} 在 {:?} 内没等到窗口", FOLLOW_MS);
    }
}

/// 启动后等窗口的上限。
///
/// 2.4 秒：冷启动的应用（Electron / Office）常见 1–2 秒；再长就是在替
/// 一个"根本不会开窗口"的程序白等（后台服务、命令行工具）。
const FOLLOW_MS: Duration = Duration::from_millis(2400);

/// `NOOKSILL_LAUNCH_TRACE=1`：打印点击落点（激活还是启动、有没有切成）。
fn trace() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("NOOKSILL_LAUNCH_TRACE").is_ok_and(|v| v.trim() != "0"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nooksill_core::config::Config;

    /// 写 `[Items]` 时不能碰 `[Settings]`，反之亦然 —— 两条路径共用一个
    /// 文件，谁用内存副本整体覆盖谁就会把对方的改动抹掉。
    #[test]
    fn write_merges_instead_of_overwriting() {
        let dir = std::env::temp_dir().join("nooksill_worker_write");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("NookSill.ini");

        let mut ini = Ini::new();
        ini.set("Settings", "Opacity", "55");
        ini.set("Items", "Count", "1");
        ini.set("Items", "I0", r"0|0|旧|C:\old.exe||0|");
        ini.set("Future", "Keep", "me");
        ini.save(&path).expect("准备文件");

        // 只写设置：条目与未知段必须原样留着。
        let cfg = Config::defaults(96);
        write(
            &path,
            &WriteJob {
                items: None,
                config: Some(Box::new(cfg)),
                tool: None,
            },
        )
        .expect("写设置");
        let after = Ini::load(&path).expect("读回");
        assert_eq!(after.get("Items", "I0"), Some(r"0|0|旧|C:\old.exe||0|"));
        assert_eq!(after.get("Future", "Keep"), Some("me"));
        assert_eq!(
            after.get("Settings", "Opacity"),
            Some("55"),
            "设置段应当被本次写入覆盖"
        );

        // 只写条目：设置段与未知段必须原样留着。
        write(
            &path,
            &WriteJob {
                items: Some(vec![Item::new(ItemKind::File, "新", r"C:\new.exe")]),
                config: None,
                tool: None,
            },
        )
        .expect("写条目");
        let after = Ini::load(&path).expect("读回");
        assert_eq!(after.get("Future", "Keep"), Some("me"));
        assert_eq!(
            after.get("Items", "I0").map(|s| s.contains("new.exe")),
            Some(true)
        );
        assert!(
            after.get("Settings", "Opacity").is_some(),
            "设置段不该被条目写入抹掉"
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    /// 在**工作线程**上跑一遍 shell 取图标。
    ///
    /// 这是本轮改动里风险最高的一环：`SHGetImageList` 返回的 `IImageList`
    /// 是 shell 的 COM 对象，必须在本线程初始化 COM 才能用。单线程时
    /// 这个问题看不出来，一挪到工作线程就变成"图标偶尔取不到"。
    ///
    /// `#[ignore]`：它要真的调 shell，在没有交互式会话的环境（某些 CI）
    /// 上会失败。手动跑：`cargo test -p nooksill-gpu -- --ignored`。
    #[test]
    #[ignore = "要调 shell，手动运行"]
    fn icon_loader_extracts_on_a_worker_thread() {
        let loader = IconLoader::spawn(Waker::new(HWND::default()), None).expect("起线程");
        loader.submit(IconRequest {
            index: 0,
            kind: ItemKind::File,
            icon_src: String::new(),
            path: r"C:\Windows\System32\notepad.exe".to_owned(),
            fit: 0,
        });

        let mut got = Vec::new();
        for _ in 0..500 {
            got = loader.drain();
            if !got.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(got.len(), 1, "应当收到一张图标");
        assert_eq!(got[0].index, 0);
        assert!(
            got[0].width > 0 && got[0].height > 0,
            "记事本一定有图标，取不到说明工作线程的 COM 没初始化好"
        );
    }

    /// 扫描线程要在后台真的枚举出东西来。
    #[test]
    #[ignore = "依赖桌面会话，手动运行"]
    fn scanner_reports_running_apps() {
        let scanner = Scanner::spawn(0, Waker::new(HWND::default())).expect("起线程");
        let mut saw_running = false;
        for _ in 0..200 {
            if scanner.drain().iter().any(|o| o.running.is_some()) {
                saw_running = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(saw_running, "扫描线程应当在 2 秒内给出一次运行状态");
    }

    /// 端到端跑一遍写盘线程：起线程 → 投任务 → 收结果。
    ///
    /// 这条测试覆盖的是**接线**（通道、线程启动、结果回传），而不是写文件
    /// 本身 —— 后者由 `write_merges_instead_of_overwriting` 覆盖。
    /// `Waker` 用空句柄：`PostMessageW` 会失败，正好验证"唤醒失败不影响结果"。
    #[test]
    fn disk_writer_round_trips_a_job() {
        let dir = std::env::temp_dir().join("nooksill_worker_roundtrip");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("NookSill.ini");

        let writer = DiskWriter::spawn(path.clone(), Waker::new(HWND::default())).expect("起线程");
        writer.submit(WriteJob {
            items: Some(vec![Item::new(ItemKind::File, "a", r"C:.exe")]),
            config: None,
            tool: None,
        });

        // 最多等 2 秒：正常是毫秒级，留足余量给慢机器。
        let mut got = Vec::new();
        for _ in 0..200 {
            got = writer.drain();
            if !got.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(got.len(), 1, "应当收到一条写盘结果");
        assert!(got[0].is_ok(), "写盘应当成功: {got:?}");
        assert!(path.exists(), "文件应当已生成");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    /// 图标处理要保留下标与路径：UI 线程靠这两项核对"装给谁"。
    #[test]
    fn process_keeps_index_and_path() {
        let req = IconRequest {
            index: 3,
            kind: ItemKind::File,
            path: r"C:\x\a.exe".to_owned(),
            icon_src: String::new(),
            fit: 0,
        };
        let bmp = IconBitmap {
            width: 2,
            height: 2,
            rgba: vec![255u8; 16],
        };
        let ready = process(&req, bmp, None);
        assert_eq!(ready.index, 3);
        assert_eq!(ready.path, r"C:\x\a.exe");
        assert_eq!((ready.width, ready.height), (2, 2));
        assert_eq!(ready.rgba.len(), 16, "圆角遮罩不该改变缓冲区长度");
    }
}
