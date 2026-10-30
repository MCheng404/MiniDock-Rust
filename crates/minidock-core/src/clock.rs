//! 时钟卡片要显示的文字。
//!
//! 逐条对应原版 `ClockLines()`（`minidock.c` 2433 行）。这里只产出文字，
//! 排版与绘制在 UI 层 —— 字符串拼接是纯逻辑，放到 core 才能秒级验证。
//!
//! ## 两套日期写法（容易搞混）
//!
//! | 停靠方向 | `DateFmt` | 输出 |
//! |---|---|---|
//! | 横（上/下） | 0 | `2026年10月30日` +（带周几时）` 周五` |
//! | 横 | 1 | `2026/10/30 周五` |
//! | 横 | 2 | `2026-10-30 周五` |
//! | 竖（左/右） | 0 | `10月30日`，周几**单独一行** |
//! | 竖 | 1 | `10/30`，周几单独一行 |
//!
//! 横排不显示年份是不够的：横排卡片宽、放得下；竖排卡片只有 21 px 一行，
//! 塞年份会挤成一团。原版就是按这个理由分成两套的。

use crate::config::{Config, Key};

/// 一份时间快照。字段对应 `SYSTEMTIME`，但去掉了 Win32 依赖。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ClockParts {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    /// 0 = 周日。与 `SYSTEMTIME::wDayOfWeek` 一致。
    pub weekday: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

/// 卡片里的一行。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClockLine {
    pub text: String,
    /// 是不是日期组。绘制时在同一组的行之间不画分割线。
    pub is_date: bool,
}

const WEEKDAY: [&str; 7] = ["日", "一", "二", "三", "四", "五", "六"];

fn weekday_name(w: u8) -> &'static str {
    WEEKDAY.get(w as usize).copied().unwrap_or("日")
}

/// 生成卡片要显示的各行，顺序即绘制顺序。
#[must_use]
pub fn clock_lines(cfg: &Config, t: &ClockParts) -> Vec<ClockLine> {
    // 语义（上游选项数组是权威）：0 = 都显示，1 = 仅时间，2 = 仅日期。
    let has_time = cfg.get_i32(Key::CLOCK_SHOW) != 2;
    let has_date = cfg.get_i32(Key::CLOCK_SHOW) != 1;
    let vertical = !cfg.edge.is_horizontal();

    let time = if cfg.get_i32(Key::CLOCK_SEC) != 0 {
        format!("{:02}:{:02}:{:02}", t.hour, t.minute, t.second)
    } else {
        format!("{:02}:{:02}", t.hour, t.minute)
    };

    let mut dates: Vec<String> = Vec::new();
    if has_date {
        let fmt = cfg.get_i32(Key::DATE_FMT);
        let week = cfg.get_i32(Key::CLOCK_WEEK) != 0;
        if vertical {
            dates.push(match fmt {
                1 => format!("{:02}/{:02}", t.month, t.day),
                2 => format!("{:02}-{:02}", t.month, t.day),
                _ => format!("{}月{}日", t.month, t.day),
            });
            if week {
                dates.push(format!("周{}", weekday_name(t.weekday)));
            }
        } else {
            let mut d = match fmt {
                1 => format!("{:04}/{:02}/{:02}", t.year, t.month, t.day),
                2 => format!("{:04}-{:02}-{:02}", t.year, t.month, t.day),
                _ => format!("{}年{}月{}日", t.year, t.month, t.day),
            };
            if week {
                d.push_str(" 周");
                d.push_str(weekday_name(t.weekday));
            }
            dates.push(d);
        }
    }

    let mut out = Vec::with_capacity(dates.len() + 1);
    // `ClockOrder`：1 = 日期在上、时间在下。竖排时原版也照样支持。
    if has_time && has_date && cfg.get_i32(Key::CLOCK_ORDER) != 0 {
        out.extend(dates.into_iter().map(|text| ClockLine {
            text,
            is_date: true,
        }));
        out.push(ClockLine {
            text: time,
            is_date: false,
        });
    } else {
        if has_time {
            out.push(ClockLine {
                text: time,
                is_date: false,
            });
        }
        out.extend(dates.into_iter().map(|text| ClockLine {
            text,
            is_date: true,
        }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Edge};

    fn at(year: u16, month: u8, day: u8, weekday: u8) -> ClockParts {
        ClockParts {
            year,
            month,
            day,
            weekday,
            hour: 9,
            minute: 5,
            second: 7,
        }
    }

    fn cfg() -> Config {
        let mut c = Config::defaults(96);
        c.clock = true;
        c.edge = Edge::Bottom;
        c
    }

    /// 横排默认：时间一行、带年份的日期一行。
    /// 这条是回归测试——我们曾经错用竖排的短日期，界面上少了年份。
    #[test]
    fn horizontal_default_includes_year() {
        let lines = clock_lines(&cfg(), &at(2026, 10, 30, 5));
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "09:05");
        assert!(!lines[0].is_date);
        assert_eq!(lines[1].text, "2026年10月30日 周五");
        assert!(lines[1].is_date);
    }

    /// 竖排：短日期 + 周几单独一行（卡片窄，塞不下年份）。
    #[test]
    fn vertical_puts_weekday_on_its_own_line() {
        let mut c = cfg();
        c.edge = Edge::Left;
        let lines = clock_lines(&c, &at(2026, 10, 30, 5));
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].text, "09:05");
        assert_eq!(lines[1].text, "10月30日");
        assert_eq!(lines[2].text, "周五");
        // 日期与周几同组，绘制时它们之间不该有分割线。
        assert!(lines[1].is_date && lines[2].is_date);
    }

    #[test]
    fn date_formats_follow_config() {
        let mut c = cfg();
        // 先关掉周几，否则日期行会带 " 周一"，比对的就不是格式本身了。
        c.apply_i32(Key::CLOCK_WEEK, 0);
        c.apply_i32(Key::DATE_FMT, 1);
        assert_eq!(clock_lines(&c, &at(2026, 10, 5, 1))[1].text, "2026/10/05");
        c.apply_i32(Key::DATE_FMT, 2);
        assert_eq!(clock_lines(&c, &at(2026, 10, 5, 1))[1].text, "2026-10-05");
    }

    #[test]
    fn seconds_are_optional() {
        let mut c = cfg();
        assert_eq!(clock_lines(&c, &at(2026, 1, 1, 0))[0].text, "09:05");
        c.apply_i32(Key::CLOCK_SEC, 1);
        assert_eq!(clock_lines(&c, &at(2026, 1, 1, 0))[0].text, "09:05:07");
    }

    /// 只显示时间 / 只显示日期时只剩一行，且不是空串。
    #[test]
    fn single_part_modes_produce_one_line() {
        let mut c = cfg();
        c.apply_i32(Key::CLOCK_SHOW, 1); // 仅时间
        let l = clock_lines(&c, &at(2026, 10, 30, 5));
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].text, "09:05");

        c.apply_i32(Key::CLOCK_SHOW, 2); // 仅日期
        let l = clock_lines(&c, &at(2026, 10, 30, 5));
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].text, "2026年10月30日 周五");
    }

    /// 周几可以关掉，关掉后日期行不再带 " 周五"。
    #[test]
    fn weekday_can_be_disabled() {
        let mut c = cfg();
        c.apply_i32(Key::CLOCK_WEEK, 0);
        assert_eq!(
            clock_lines(&c, &at(2026, 10, 30, 5))[1].text,
            "2026年10月30日"
        );
    }

    /// `ClockOrder=1` 把日期放上面。
    #[test]
    fn order_swaps_date_and_time() {
        let mut c = cfg();
        c.apply_i32(Key::CLOCK_ORDER, 1);
        let l = clock_lines(&c, &at(2026, 10, 30, 5));
        assert!(l[0].is_date, "第一行应当是日期");
        assert_eq!(l[1].text, "09:05");
    }

    /// 周几越界不能 panic（系统返回值理论上不会，但别赌）。
    #[test]
    fn out_of_range_weekday_is_safe() {
        let l = clock_lines(&cfg(), &at(2026, 10, 30, 200));
        assert!(l[1].text.ends_with("周日"));
    }
}
