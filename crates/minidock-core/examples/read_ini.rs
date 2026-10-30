//! 读一份 `MiniDock.ini` 并打印解析结果，用于排查"配置写了却没生效"。
//!
//! # 为什么需要这个
//!
//! 排查一起「`Edge=1` 写了、Dock 却仍停在底部」的问题时，直接看 ini 文件内容
//! 是对的，但看不到**编码是否被识别**、**键是否进了 `[Settings]` 段**、
//! **条目解析出了几项**。这三件事任何一件出错，现象都是"配置不生效"。
//!
//! 实测结论：那次真正的成因是 ini 被另一个实例整份覆盖回默认值，
//! 与解析无关。但如果没有这个工具，只能反复猜"是不是解析 bug"。
//!
//! # 用法
//!
//! ```text
//! cargo run -q -p minidock-core --example read_ini -- path/to/MiniDock.ini
//! ```
//!
//! `raw X=` 打的是**原样字符串**，与解析后的枚举值对比即可判断是"没读到"
//! 还是"读了但没认出来"。

use minidock_core::config::Config;
use minidock_core::ini::Ini;
use minidock_core::store;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("用法: cargo run -p minidock-core --example read_ini -- <ini 路径>");
    let ini = Ini::load(std::path::Path::new(&path)).expect("读取失败");

    // 固定按 144 dpi 解析：这里只看"配置项读没读到"，与 DPI 缩放无关。
    let cfg = Config::from_ini(&ini, 144);

    println!(
        "Edge={:?} Mode={:?} Reserve={}",
        cfg.edge, cfg.mode, cfg.reserve
    );
    println!(
        "IconSize={} ZoomSize={} Clock={}",
        cfg.icon_size, cfg.zoom_size, cfg.clock
    );
    println!(
        "[Settings] 共 {} 键 / [Items] 共 {} 键",
        ini.len_of("Settings"),
        ini.len_of("Items")
    );

    // 原样值：解析结果为默认值时分不清"没读到"和"认不出"，这里补上。
    for key in ["Edge", "Mode", "Reserve", "IconSize", "Count"] {
        let sec = if key == "Count" { "Items" } else { "Settings" };
        println!("raw {key}={:?}", ini.get(sec, key));
    }

    let items = store::load_items(&ini);
    println!("解析出 {} 个条目", items.len());
    for (i, it) in items.iter().enumerate() {
        println!("  [{i}] {:?} name={} path={}", it.kind, it.name, it.path);
    }
}
