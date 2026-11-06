//! 起动台的诊断探针：打印标签组与每个应用落在哪一组。
//!
//! 用法：
//!
//! ```bash
//! cargo run -q -p nooksill-core --example lp_dump -- <launchpad.dat>
//! ```
//!
//! 排查"分类不对"时先跑它：能看到标签组读出来是什么、每个应用被归到第几组，
//! 省得靠界面一个个数（190 个应用根本数不过来）。

use nooksill_core::launchpad::Launchpad;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("用法：lp_dump <launchpad.dat>");
    let text = std::fs::read_to_string(&path).expect("读文件失败");
    let lp = Launchpad::parse(&text);
    println!("标签组：{:?}", lp.tags());
    println!("应用：{} 个", lp.apps().len());
    let mut counts = vec![0usize; lp.tags().len() + 1];
    for i in 0..lp.apps().len() {
        counts[lp.category_of(i)] += 1;
    }
    for (i, c) in counts.iter().enumerate().skip(1) {
        println!("  第 {i} 组 {:?}: {c} 个", lp.tags().get(i - 1));
    }
}
