//! 激活探针：对同一个 exe 连续调 `activate(exe, toggle=true)`，把每一步的
//! 判定与结果打出来。
//!
//! 为什么需要它：`activate` 的行为取决于**当前前台是谁**，而这个状态在
//! 自动化点击里根本不可控（鼠标位置、悬浮窗、虚拟桌面都会影响）。把它
//! 做成一个独立进程，argv 给个正在跑的 exe，两次调用就能看出
//! "显示 → 最小化" 是否成对。
//!
//! 用法：
//! ```text
//! cargo run -q -p nooksill-platform --example activate_probe -- notepad.exe
//! ```

use std::time::Duration;

use nooksill_platform::app_window;

fn main() {
    let Some(name) = std::env::args().nth(1) else {
        eprintln!("用法: activate_probe <exe 路径或名字>");
        std::process::exit(2);
    };

    // 允许传短名（notepad.exe）：在所有窗口里按文件名找。
    let exe = if name.contains('\\') {
        name.clone()
    } else {
        let found = app_window::running_apps()
            .into_keys()
            .find(|k| k.ends_with(&format!("\\{}", name.to_lowercase())));
        match found {
            Some(p) => p,
            None => {
                eprintln!("没找到正在运行的 {name}");
                std::process::exit(1);
            }
        }
    };

    println!("目标: {exe}");
    for round in 1..=3 {
        println!(
            "第 {round} 次: 在前台={} 窗口={:?}",
            app_window::in_front_of(&exe),
            app_window::find_window_of(&exe).map(|h| h.0 as isize)
        );
        let r = app_window::activate(&exe, true);
        println!("           → {r:?}");
        std::thread::sleep(Duration::from_millis(600));
    }
}
