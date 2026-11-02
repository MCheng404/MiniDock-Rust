//! 打印某个月的日历网格与农历信息，用来人工核对渲染与算法。
//!
//! 用法：`cargo run -q -p minidock-core --example calendar -- 2026 11`

use minidock_core::calendar::{self, CalendarState};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let year: i32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(2026);
    let month: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(11);

    let mut st = CalendarState::new((year, month, 1));
    st.select(year, month, 1);

    println!("===== {year} 年 {month} 月 =====");
    println!("一\t二\t三\t四\t五\t六\t日");
    for row in st.cells().chunks(7) {
        let line: Vec<String> = row
            .iter()
            .map(|c| {
                let mark = if c.in_month { "" } else { "·" };
                format!(
                    "{mark}{}/{}({})",
                    c.day,
                    c.sub,
                    if c.highlight { "*" } else { "" }
                )
            })
            .collect();
        println!("{}", line.join("\t"));
    }

    println!("\n===== 若干固定点 =====");
    for (y, m, d) in [
        (2026, 11, 2),
        (2026, 2, 17),
        (2027, 2, 6),
        (2026, 9, 25),
        (2026, 10, 1),
    ] {
        let l = calendar::solar_to_lunar(y, m, d);
        let sub = calendar::sub_label(y, m, d);
        println!(
            "{y}-{m:02}-{d:02}  星期{}  农历 {}  副标签 {}  干支 {}",
            calendar::weekday_name(calendar::weekday_of(y, m, d)),
            l.map_or_else(|| "—".to_owned(), calendar::lunar_text),
            sub.0,
            calendar::ganzhi_text(y, m, d)
        );
    }
    if let Some((y, n, days)) = calendar::next_term(year, month, 1) {
        let (tm, td) = calendar::term_date(y, n);
        println!(
            "\n下一个节气：{} {} 月 {} 日（还有 {} 天）",
            calendar::term_name(n),
            tm,
            td,
            days
        );
    }
}
