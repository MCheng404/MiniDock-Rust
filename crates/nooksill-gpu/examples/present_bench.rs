//! Cost of the layered-window present path, measured on a live window.
//!
//! Three per-frame costs, none of which depends on scene complexity:
//! 1. `upload_rgba_strided` — RGBA rows to premultiplied BGRA inside the DIB;
//! 2. `present` — full-surface `UpdateLayeredWindow`;
//! 3. `present_dirty` — `UpdateLayeredWindowIndirect` with a small `prcDirty`.
//!
//! ```text
//! cargo run --release -p nooksill-gpu --example present_bench -- [w] [h] [iters]
//! ```
//!
//! Defaults: 1320x1080 (dock bar), 300 iterations, first 30 dropped as warm-up.
//!
//! The window is layered, top-level and near-transparent (alpha 1/255), so it
//! is invisible on screen while DWM still treats it as a real surface. Run it
//! in `--release`: debug timings are 3-5x higher and only good for ratios.
//!
//! Reading the numbers: if every phase lands near the refresh interval (16 ms
//! at 60 Hz), `UpdateLayeredWindow` is refreshing-bound and no CPU-side change
//! will help. If phases are well under it, the cost is the bitmap copy itself.

use std::time::{Duration, Instant};

use nooksill_gpu::present::LayeredPresenter;
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, RegisterClassW, ShowWindow, SW_SHOWNOACTIVATE, WNDCLASSW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
};

const WARMUP: usize = 30;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let width = arg(&mut args, 1320);
    let height = arg(&mut args, 1080);
    let iters = arg(&mut args, 300usize);
    let stride = width as usize * 4;

    let hwnd = make_window(width, height)?;
    let mut presenter = LayeredPresenter::new(width, height)?;
    let src = vec![1u8; stride * height as usize];

    println!("size {width}x{height}  iters {iters}  warm-up {WARMUP}\n");
    println!(
        "{:<24} {:>8} {:>8} {:>8} {:>8} {:>8}",
        "phase", "min", "p50", "p95", "max", "mean"
    );

    let upload = sample(iters, || presenter.upload_rgba_strided(&src, stride));
    upload.row("upload_rgba_strided");

    let mut failures = 0usize;
    let full = sample(iters, || {
        if presenter.present(hwnd, 0, 0).is_err() {
            failures += 1;
        }
    });
    full.row("present (full)");

    let dirty = RECT {
        left: 0,
        top: 0,
        right: width / 8,
        bottom: height / 8,
    };
    let part = sample(iters, || {
        if presenter.present_dirty(hwnd, 0, 0, dirty).is_err() {
            failures += 1;
        }
    });
    part.row("present_dirty (1/64)");

    println!(
        "\npresent ceiling: {:.0} fps (full) / {:.0} fps (dirty)",
        1000.0 / full.mean,
        1000.0 / part.mean
    );
    if failures > 0 {
        println!("failures: {failures} — timings above are not trustworthy");
    }
    Ok(())
}

fn arg<T: std::str::FromStr>(it: &mut impl Iterator<Item = String>, dflt: T) -> T {
    it.next().and_then(|s| s.parse().ok()).unwrap_or(dflt)
}

struct Stats {
    min: f64,
    p50: f64,
    p95: f64,
    max: f64,
    mean: f64,
}

/// Run `f` `iters + WARMUP` times and keep per-call wall time.
fn sample(iters: usize, mut f: impl FnMut()) -> Stats {
    let mut times = Vec::with_capacity(iters);
    for i in 0..iters + WARMUP {
        let t0 = Instant::now();
        f();
        let dt = t0.elapsed();
        if i >= WARMUP {
            times.push(dt);
        }
    }
    Stats::new(times)
}

impl Stats {
    fn new(mut times: Vec<Duration>) -> Self {
        times.sort_unstable();
        let ms = |d: &Duration| d.as_secs_f64() * 1e3;
        let n = times.len();
        let at = |q: f64| ms(&times[((n as f64 - 1.0) * q).round() as usize]);
        Self {
            min: ms(&times[0]),
            p50: at(0.5),
            p95: at(0.95),
            max: ms(&times[n - 1]),
            mean: times.iter().map(ms).sum::<f64>() / n as f64,
        }
    }

    fn row(&self, label: &str) {
        println!(
            "{label:<24} {:>8.3} {:>8.3} {:>8.3} {:>8.3} {:>8.3}",
            self.min, self.p50, self.p95, self.max, self.mean
        );
    }
}

fn make_window(width: i32, height: i32) -> Result<HWND, Box<dyn std::error::Error>> {
    let instance = unsafe { GetModuleHandleW(None)? };
    let class = WNDCLASSW {
        hInstance: instance.into(),
        lpfnWndProc: Some(wndproc),
        lpszClassName: w!("PresentBenchWindow"),
        ..Default::default()
    };
    unsafe { RegisterClassW(&class) };
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
            w!("PresentBenchWindow"),
            w!("present_bench"),
            WS_POPUP,
            0,
            0,
            width,
            height,
            None,
            None,
            Some(instance.into()),
            None,
        )?
    };
    let _ = unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
    Ok(hwnd)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
