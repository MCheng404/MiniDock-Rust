//! 农历 / 节气 / 干支历。对应上游 v1.6.0 的 `g_lunarInfo` / `SolarToLunar` /
//! `TermDate` / `CalSub` / `GanzhiText`（minidock.c:5613-5745）。
//!
//! 这一块**全部是纯算法**，一个 Win32 调用都没有。于是可以用穷举的方式验证
//! 而不是"跑起来看着像对的"：1901..=2099 共七万多天，逐日做
//! 公历 → 农历 → 公历 的往返，任何一天的月长或闰月算错都会当场暴露。
//!
//! 表在 [`calendar_tables`] 里，由脚本从上游源码提取生成 —— 201 个十六进制
//! 数手抄会错位，而且错了不报错，只会让某几年的农历静默算错。

mod calendar_tables;

use calendar_tables::{
    GAN, LUNAR_DAY, LUNAR_INFO, LUNAR_MONTH, TERM_INFO, TERM_NAME, WEEKDAY, ZHI, ZODIAC,
};

// ============================================================================
// 公历 ↔ 天数（Howard Hinnant 的 civil-from-days，上游用的也是这套）
// ============================================================================

/// 公历日期转"1970-01-01 起的天数"。
///
/// 用的 Hinnant 算法：先按 400 年一个纪元折算，纪元内再按 153 天的
/// 五个月周期折算。好处是**没有分支判断闰年**，1901..2099 全程单调。
#[must_use]
pub fn days_from_civil(year: i32, month: u32, day: u32) -> i32 {
    debug_assert!((1..=12).contains(&month), "月份越界: {month}");
    debug_assert!((1..=31).contains(&day), "日越界: {day}");
    let y = year - i32::from(month <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32; // [0, 399]
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe as i32 - 719_468
}

/// 天数转公历日期。与 [`days_from_civil`] 互为逆运算。
#[must_use]
pub fn civil_from_days(days: i32) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i32 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (y + i32::from(m <= 2), m, d)
}

/// 星期几，0 = 周日。上游写法是 `((days % 7) + 11) % 7`。
#[must_use]
pub fn weekday_of(year: i32, month: u32, day: u32) -> u32 {
    (((days_from_civil(year, month, day) % 7) + 11) % 7) as u32
}

/// 星期名（"日" / "一" / …）。日历表头与信息面板共用。
#[must_use]
pub fn weekday_name(w: u32) -> &'static str {
    WEEKDAY[(w % 7) as usize]
}

// ============================================================================
// 农历
// ============================================================================

/// 农历日期。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Lunar {
    /// 农历年（不是公历年 —— 春节前相差一年）。
    pub year: i32,
    /// 月，1..=12。
    pub month: u32,
    /// 日，1..=30。
    pub day: u32,
    /// 是否闰月。
    pub leap: bool,
}

/// 农历表覆盖的年份范围（表下标 `year - 1900`，共 201 项）。
const LUNAR_MIN: i32 = 1900;
const LUNAR_MAX: i32 = 2100;

/// 支持换算的公历年份范围。上游是 1901..=2099
/// （1900 年初的农历数据不全，2100 年之后表里没有）。
pub const MIN_YEAR: i32 = 1901;
pub const MAX_YEAR: i32 = 2099;

/// 某农历年的闰月月份；0 表示无闰月。
fn leap_month(year: i32) -> u32 {
    LUNAR_INFO[(year - LUNAR_MIN) as usize] & 0xf
}

/// 某农历年闰月的天数；无闰月时为 0。
fn leap_days(year: i32) -> u32 {
    if leap_month(year) == 0 {
        0
    } else if LUNAR_INFO[(year - LUNAR_MIN) as usize] & 0x1_0000 != 0 {
        30
    } else {
        29
    }
}

/// 某农历年第 `month` 个**普通月**（1..=12）的天数。
fn month_days(year: i32, month: u32) -> u32 {
    if LUNAR_INFO[(year - LUNAR_MIN) as usize] & (0x1_0000 >> month) != 0 {
        30
    } else {
        29
    }
}

/// 某农历年的总天数（354..=385）。
fn year_days(year: i32) -> i32 {
    // 348 = 12 × 29：先当每个月都是小月，再按位补大月。
    //
    // 位是 `0x8000 >> (m - 1)`，m = 1..=12，也就是 0x8000 一路右移到 0x10。
    // （上游写成 `for (i = 0x8000; i > 0x8; i >>= 1)`，条件 `i > 0x8` 让循环
    // 停在 0x10，正好 12 次。第一版照抄成 `(0x8000..=0x8)` 是个空区间，
    // 一年只有 348 天，整个农历往后错位了两年多。）
    let mut sum = 348;
    let info = LUNAR_INFO[(year - LUNAR_MIN) as usize];
    for shift in 0..12 {
        if info & (0x8000 >> shift) != 0 {
            sum += 1;
        }
    }
    sum + leap_days(year) as i32
}

/// 公历转农历。超出 1901..=2099 返回 `None`（上游也是这个范围）。
#[must_use]
pub fn solar_to_lunar(year: i32, month: u32, day: u32) -> Option<Lunar> {
    if !(MIN_YEAR..=MAX_YEAR).contains(&year) {
        return None;
    }
    // 1900-01-31 是农历 1900 年正月初一，作为基准。
    let mut offset = days_from_civil(year, month, day) - days_from_civil(1900, 1, 31);

    // ---- 先定位农历年 ----
    let mut ly = LUNAR_MIN;
    loop {
        let d = year_days(ly);
        if offset < d {
            break;
        }
        offset -= d;
        ly += 1;
        // 表的右边界：走到 2100 还没完说明输入越界（已被上面的范围检查挡住，
        // 这里只是防止 debug_assert 在 release 下被优化掉后越界读表）。
        if ly > LUNAR_MAX {
            return None;
        }
    }

    // ---- 再定位月（含闰月）----
    let leap = leap_month(ly);
    let mut m = 1_u32;
    let mut is_leap = false;
    loop {
        let d = if is_leap {
            leap_days(ly)
        } else {
            month_days(ly, m)
        };
        if offset < d as i32 {
            break;
        }
        offset -= d as i32;
        // 顺序是「… 四月、闰四月、五月 …」：普通月走完若正好是闰月，下一步
        // 停在同一个月份号的闰月上，再下一步才进下个月。
        if !is_leap && leap > 0 && m == leap {
            is_leap = true;
        } else {
            is_leap = false;
            m += 1;
            if m > 12 {
                return None;
            }
        }
    }

    Some(Lunar {
        year: ly,
        month: m,
        day: offset as u32 + 1,
        leap: is_leap,
    })
}

/// 农历转公历。与 [`solar_to_lunar`] 互为逆运算（用于往返验证）。
#[must_use]
pub fn lunar_to_solar(l: Lunar) -> Option<(i32, u32, u32)> {
    if l.year < LUNAR_MIN || l.year > MAX_YEAR || !(1..=12).contains(&l.month) {
        return None;
    }
    if l.day < 1 || l.day > 30 {
        return None;
    }
    // 校验这一天真的存在：闰月得真有闰，日不能超过当月长度。
    let leap = leap_month(l.year);
    if l.leap && leap != l.month {
        return None;
    }
    let len = if l.leap {
        leap_days(l.year)
    } else {
        month_days(l.year, l.month)
    };
    if l.day > len {
        return None;
    }

    let mut days = days_from_civil(1900, 1, 31);
    for y in LUNAR_MIN..l.year {
        days += year_days(y);
    }
    for m in 1..l.month {
        days += month_days(l.year, m) as i32;
        if leap > 0 && m == leap {
            days += leap_days(l.year) as i32;
        }
    }
    if l.leap {
        days += month_days(l.year, l.month) as i32;
    }
    days += l.day as i32 - 1;
    let (y, m, d) = civil_from_days(days);
    Some((y, m, d))
}

/// 农历月名（"正" / "二" / … / "腊"）。
#[must_use]
pub fn lunar_month_name(month: u32) -> &'static str {
    LUNAR_MONTH[(month.clamp(1, 12) - 1) as usize]
}

/// 农历日名（"初一" / … / "三十"）。
#[must_use]
pub fn lunar_day_name(day: u32) -> &'static str {
    LUNAR_DAY[(day.clamp(1, 30) - 1) as usize]
}

/// 完整的农历描述，如「甲辰年（龙） 正月 初一」。
#[must_use]
pub fn lunar_text(l: Lunar) -> String {
    let yi = ((l.year - 4) % 10 + 10) % 10;
    let zi = ((l.year - 4) % 12 + 12) % 12;
    format!(
        "{}{}年（{}） {}{}月{}",
        GAN[yi as usize],
        ZHI[zi as usize],
        ZODIAC[zi as usize],
        if l.leap { "闰" } else { "" },
        lunar_month_name(l.month),
        lunar_day_name(l.day)
    )
}

// ============================================================================
// 节气
// ============================================================================

/// 第 `n` 个节气（0 = 小寒，23 = 冬至）在 `year` 年的日期。
///
/// 上游用的是**平气**公式（把回归年当常数 365.2422 天），不是现代的定气。
/// 于是与本世纪的真实节气最多差一天。照抄是因为要跟上游的日历**对得上**；
/// 真要精确到分钟得换成 VSOP87，那不是一个 Dock 该背的重量。
#[must_use]
pub fn term_date(year: i32, n: usize) -> (u32, u32) {
    let n = n % 24;
    let ms = 31_556_925_974.7_f64 * f64::from(year - 1900) + TERM_INFO[n] as f64 * 60_000.0;
    let days =
        f64::from(days_from_civil(1900, 1, 6)) + (2.0 * 60.0 + 5.0) / 1440.0 + ms / 86_400_000.0;
    let (_, m, d) = civil_from_days(days.floor() as i32);
    (m, d)
}

/// 节气名。
#[must_use]
pub fn term_name(n: usize) -> &'static str {
    TERM_NAME[n % 24]
}

/// 从 `(y, m, d)` 往后数，最近的节气。跳过当天。
#[must_use]
pub fn next_term(year: i32, month: u32, day: u32) -> Option<(i32, usize, i32)> {
    let today = days_from_civil(year, month, day);
    for y in year..=year + 1 {
        for n in 0..24 {
            let (tm, td) = term_date(y, n);
            let t = days_from_civil(y, tm, td);
            if t > today {
                return Some((y, n, t - today));
            }
        }
    }
    None
}

// ============================================================================
// 单元格副标签
// ============================================================================

/// 公历节日（月, 日, 名）。
const SOLAR_FESTIVALS: [(u32, u32, &str); 6] = [
    (1, 1, "元旦"),
    (2, 14, "情人节"),
    (5, 1, "劳动节"),
    (6, 1, "儿童节"),
    (10, 1, "国庆节"),
    (12, 25, "圣诞节"),
];

/// 日历格子下方的副标签与"是否高亮"。
///
/// 优先级照上游 `CalSub`：公历节日 > 农历节日 > 节气 > 初一显示月名 > 农历日名。
/// 高亮表示"这是个值得用红色标出来的日子"（节日与节气）。
#[must_use]
pub fn sub_label(year: i32, month: u32, day: u32) -> (String, bool) {
    if let Some((_, _, name)) = SOLAR_FESTIVALS
        .iter()
        .find(|(m, d, _)| *m == month && *d == day)
    {
        return ((*name).to_owned(), true);
    }

    let lunar = solar_to_lunar(year, month, day);
    if let Some(l) = lunar {
        if !l.leap {
            let festival = match (l.month, l.day) {
                (1, 1) => Some("春节"),
                (1, 15) => Some("元宵"),
                (5, 5) => Some("端午"),
                (7, 7) => Some("七夕"),
                (8, 15) => Some("中秋"),
                (9, 9) => Some("重阳"),
                (12, d) if d == month_days(l.year, 12) => Some("除夕"),
                _ => None,
            };
            if let Some(name) = festival {
                return (name.to_owned(), true);
            }
        }
    }

    // 每个公历月有两个节气：第 (m-1)*2 个与第 (m-1)*2+1 个。
    for n in [(month as usize - 1) * 2, (month as usize - 1) * 2 + 1] {
        let (tm, td) = term_date(year, n);
        if tm == month && td == day {
            return (term_name(n).to_owned(), true);
        }
    }

    if let Some(l) = lunar {
        if l.day == 1 {
            return (
                format!(
                    "{}{}月",
                    if l.leap { "闰" } else { "" },
                    lunar_month_name(l.month)
                ),
                false,
            );
        }
        return (lunar_day_name(l.day).to_owned(), false);
    }

    (String::new(), false)
}

/// 干支历。年以**立春**换、月以**节**换（不是以正月初一换）。
///
/// 这是"四柱"里的年月日三柱，时柱要出生时辰，日历上不显示。
#[must_use]
pub fn ganzhi_text(year: i32, month: u32, day: u32) -> String {
    // 干支年：立春之前还算上一年。
    let (lm, ld) = term_date(year, 2); // 索引 2 = 立春
    let gy = if month > lm || (month == lm && day >= ld) {
        year
    } else {
        year - 1
    };
    let ys = ((gy - 4) % 10 + 10) % 10;
    let yz = ((gy - 4) % 12 + 12) % 12;

    // 干支月：以"节"（每月的第一个节气）为界。
    let (_tm, td) = term_date(year, (month as usize - 1) * 2);
    let eff_month = if day >= td {
        month as i32
    } else {
        month as i32 - 1
    };
    let branch = (eff_month % 12 + 12) % 12;
    // 五虎遁：年干决定正月的天干。正月建寅，所以从「寅」往前数 offset 位。
    let offset = (branch - 2 + 12) % 12;
    let ms = ((ys % 5) * 2 + 2 + offset) % 10;

    // 干支日：60 天一循环，1970-01-01 是辛巳日（偏移 17 是上游的对齐值）。
    let di = ((days_from_civil(year, month, day) + 17) % 60 + 60) % 60;

    format!(
        "{}{}年  {}{}月  {}{}日",
        GAN[ys as usize],
        ZHI[yz as usize],
        GAN[ms as usize],
        ZHI[branch as usize],
        GAN[(di % 10) as usize],
        ZHI[(di % 12) as usize]
    )
}

// ============================================================================
// 日历的 UI 状态
// ============================================================================

/// 一格里要显示的东西。算好之后 UI 只管画，不再碰算法。
#[derive(Clone, Debug, PartialEq)]
pub struct DayCell {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    /// 是否属于当前显示的月份（前后补齐的日子为 false）。
    pub in_month: bool,
    /// 副标签（节日 / 节气 / 农历日）。
    pub sub: String,
    /// 副标签是否高亮（节日与节气）。
    pub highlight: bool,
}

/// 日历的选中状态与显示的年月。
///
/// 与 UI 无关：翻页、选中、生成网格都在这里，于是"点击格子跳到下个月"
/// 这类行为能用单测覆盖，不用起窗口。
#[derive(Clone, Debug)]
pub struct CalendarState {
    /// 当前显示的年。
    pub year: i32,
    /// 当前显示的月。
    pub month: u32,
    /// 选中的日期。
    pub selected: (i32, u32, u32),
    cells: [DayCell; 42],
}

impl CalendarState {
    /// 以某一天为"今天"建立状态（选中今天、显示当月）。
    #[must_use]
    pub fn new(today: (i32, u32, u32)) -> Self {
        let mut s = Self {
            year: today.0.clamp(MIN_YEAR, MAX_YEAR - 1),
            month: today.1,
            selected: today,
            cells: std::array::from_fn(|_| DayCell {
                year: today.0,
                month: today.1,
                day: 1,
                in_month: true,
                sub: String::new(),
                highlight: false,
            }),
        };
        s.refresh();
        s
    }

    /// 翻页。`dy` / `dm` 为年 / 月的增量，月份会进位；年份夹在 1901..=2098
    /// （上游同样夹这个范围 —— 再往后农历表就没有数据了）。
    pub fn step(&mut self, dy: i32, dm: i32) {
        let mut y = self.year + dy;
        let mut m = self.month as i32 + dm;
        while m > 12 {
            m -= 12;
            y += 1;
        }
        while m < 1 {
            m += 12;
            y -= 1;
        }
        let y = y.clamp(MIN_YEAR, MAX_YEAR - 1);
        if y != self.year {
            // 夹住年份时月份也要跟着夹，否则会出现 2098 年 13 月。
            m = m.clamp(1, 12);
        }
        self.year = y;
        self.month = m as u32;
        self.refresh();
    }

    /// 选中某一天。若它不在当前显示的月份，顺带翻到那个月。
    pub fn select(&mut self, year: i32, month: u32, day: u32) {
        self.selected = (year, month, day);
        if year != self.year || month != self.month {
            self.year = year.clamp(MIN_YEAR, MAX_YEAR - 1);
            self.month = month;
            self.refresh();
        }
    }

    /// 回到今天。
    pub fn go_today(&mut self, today: (i32, u32, u32)) {
        self.year = today.0.clamp(MIN_YEAR, MAX_YEAR - 1);
        self.month = today.1;
        self.selected = today;
        self.refresh();
    }

    /// 42 个格子（6 行 × 7 列，周一起始）。
    #[must_use]
    pub fn cells(&self) -> &[DayCell; 42] {
        &self.cells
    }

    /// 重算格子。翻页 / 改选中时调用。
    fn refresh(&mut self) {
        // 周一起始：把"周日 = 0"的偏移转成"周一 = 0"。
        let first = weekday_of(self.year, self.month, 1);
        let offset = ((first + 6) % 7) as i32;
        let base = days_from_civil(self.year, self.month, 1) - offset;
        for k in 0..42 {
            let (y, m, d) = civil_from_days(base + k as i32);
            let (sub, highlight) = sub_label(y, m, d);
            self.cells[k] = DayCell {
                year: y,
                month: m,
                day: d,
                in_month: m == self.month,
                sub,
                highlight,
            };
        }
    }
}

// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 公历 ↔ 天数往返。这一条挂了，下面全部没意义。
    #[test]
    fn civil_roundtrip_over_the_supported_range() {
        for y in MIN_YEAR..=MAX_YEAR {
            for m in 1..=12 {
                // 每月 31 天都试一遍：多出来的日子会被下一月接住，
                // 正好顺带验证跨月进位。
                for d in 1..=31 {
                    let z = days_from_civil(y, m, d);
                    let (y2, m2, d2) = civil_from_days(z);
                    let expect = days_from_civil(y2, m2, d2);
                    assert_eq!(z, expect, "{y}-{m}-{d} 往返不一致");
                }
            }
        }
    }

    /// 已知的星期锚点（2026-11-02 是周一）。
    #[test]
    fn weekday_anchors() {
        assert_eq!(weekday_of(1970, 1, 1), 4, "1970-01-01 是周四");
        assert_eq!(weekday_of(2000, 1, 1), 6, "2000-01-01 是周六");
        assert_eq!(weekday_of(2026, 11, 2), 1, "2026-11-02 是周一");
        assert_eq!(weekday_name(0), "日");
        assert_eq!(weekday_name(1), "一");
    }

    /// 春节锚点。农历年首是全年最容易错的一天：闰月算错、月长算错
    /// 都会把春节挪前一两天，而这里对的是公开可查的日期。
    #[test]
    fn spring_festival_anchors() {
        let anchors = [
            ((2020, 1, 25), 2020),
            ((2021, 2, 12), 2021),
            ((2022, 2, 1), 2022),
            ((2023, 1, 22), 2023),
            ((2024, 2, 10), 2024),
            ((2025, 1, 29), 2025),
            ((2026, 2, 17), 2026),
        ];
        for ((y, m, d), ly) in anchors {
            let l = solar_to_lunar(y, m, d).expect("范围内");
            assert_eq!(
                (l.year, l.month, l.day, l.leap),
                (ly, 1, 1, false),
                "{y}-{m}-{d} 应当是农历 {ly} 年正月初一"
            );
            // 前一天应当是上一年的腊月，且不是初一。
            let (py, pm, pd) = civil_from_days(days_from_civil(y, m, d) - 1);
            let prev = solar_to_lunar(py, pm, pd).expect("范围内");
            assert_eq!(prev.month, 12, "{py}-{pm}-{pd} 应当是腊月");
            assert_ne!(prev.day, 1, "春节前一天不该是初一");
        }
    }

    /// 闰月锚点：这三个年份的闰月是公开可查的。
    #[test]
    fn leap_month_anchors() {
        // 2020 闰四月，闰四月初一 = 2020-05-23
        let l = solar_to_lunar(2020, 5, 23).expect("范围内");
        assert_eq!((l.month, l.day, l.leap), (4, 1, true));
        // 2023 闰二月，闰二月初一 = 2023-03-22
        let l = solar_to_lunar(2023, 3, 22).expect("范围内");
        assert_eq!((l.month, l.day, l.leap), (2, 1, true));
        // 2017 闰六月，闰六月初一 = 2017-07-23
        let l = solar_to_lunar(2017, 7, 23).expect("范围内");
        assert_eq!((l.month, l.day, l.leap), (6, 1, true));
    }

    /// 逐日往返：1901-01-01 到 2099-12-31 每一天，
    /// 公历 → 农历 → 公历 必须回到原处。
    ///
    /// 这条比"抽查几个日期"强得多：月长、闰月、跨年进位任何一处算错，
    /// 都会在某个具体日子上崩掉。七万多天，测试仍然只要几十毫秒。
    #[test]
    fn solar_lunar_roundtrip_every_day() {
        let start = days_from_civil(MIN_YEAR, 1, 1);
        let end = days_from_civil(MAX_YEAR, 12, 31);
        let mut checked = 0_u32;
        for z in start..=end {
            let (y, m, d) = civil_from_days(z);
            let l = solar_to_lunar(y, m, d).unwrap_or_else(|| panic!("{y}-{m}-{d} 换算失败"));
            assert!(
                (1..=12).contains(&l.month) && (1..=30).contains(&l.day),
                "{y}-{m}-{d} 得到非法农历 {:?}",
                l
            );
            let back = lunar_to_solar(l).unwrap_or_else(|| panic!("{y}-{m}-{d} 反算失败: {l:?}"));
            assert_eq!((y, m, d), back, "{y}-{m}-{d} 往返不一致（农历 {l:?}）");
            checked += 1;
        }
        assert_eq!(checked, 72_684, "1901..=2099 共 72684 天");
    }

    /// 农历日的连续性：后一天的农历日要么是 +1，要么是下月初一。
    #[test]
    fn lunar_days_are_contiguous() {
        let start = days_from_civil(MIN_YEAR, 1, 1);
        let end = days_from_civil(MAX_YEAR, 12, 31);
        let mut prev = solar_to_lunar(MIN_YEAR, 1, 1).expect("范围内");
        for z in start + 1..=end {
            let (y, m, d) = civil_from_days(z);
            let cur = solar_to_lunar(y, m, d).expect("范围内");
            let expected_day = if cur.day == 1 {
                assert_ne!(prev.day, 1, "{y}-{m}-{d} 连续两天都是初一");
                assert!(
                    prev.day == 29 || prev.day == 30,
                    "上个月只有 {} 天",
                    prev.day
                );
                1
            } else {
                prev.day + 1
            };
            assert_eq!(cur.day, expected_day, "{y}-{m}-{d} 农历日不连续");
            prev = cur;
        }
    }

    /// 节气：立春在 2 月 3-5 日，冬至在 12 月 21-23 日，春分在 3 月 19-21 日。
    ///
    /// 平气公式与真实节气最多差一天，所以给的是区间而不是定值。
    #[test]
    fn solar_terms_land_in_the_right_windows() {
        for y in 1901..=2099 {
            let (m, d) = term_date(y, 2); // 立春
            assert_eq!(m, 2, "{y} 立春应当在 2 月");
            assert!((3..=5).contains(&d), "{y} 立春在 2 月 {d} 日");
            let (m, d) = term_date(y, 23); // 冬至
            assert_eq!(m, 12, "{y} 冬至应当在 12 月");
            assert!((21..=23).contains(&d), "{y} 冬至在 12 月 {d} 日");
            let (m, d) = term_date(y, 5); // 春分
            assert_eq!(m, 3, "{y} 春分应当在 3 月");
            // 平气公式在 199 年里跨 3 月 19~22 日（多数是 20/21）。
            assert!((19..=22).contains(&d), "{y} 春分在 3 月 {d} 日");
            let (m, d) = term_date(y, 11); // 夏至
            assert_eq!(m, 6, "{y} 夏至应当在 6 月");
            assert!((20..=22).contains(&d), "{y} 夏至在 6 月 {d} 日");
        }
    }

    /// 副标签的优先级：节日压过节气，节气压过农历日。
    #[test]
    fn sub_labels_follow_priority() {
        // 2026-01-01 元旦（公历节日）
        assert_eq!(sub_label(2026, 1, 1).0, "元旦");
        // 2026-02-17 春节
        assert_eq!(sub_label(2026, 2, 17).0, "春节");
        // 2026-02-04 立春（节气）
        assert_eq!(sub_label(2026, 2, 4).0, "立春");
        // 普通日子显示农历日名。春节之后逐日找第一个既不是节日也不是
        // 节气的日子 —— 2026-02-18 正好是雨水，所以要跳过。
        let (d, (label, _)) = (18..=28)
            .map(|d| (d, sub_label(2026, 2, d)))
            .find(|(_, (_, hl))| !hl)
            .expect("春节后总能找到普通日子");
        assert!(
            label.starts_with('初') || label.starts_with('十') || label.starts_with('廿'),
            "2026-02-{d} 应当显示农历日名，实际是 {label}"
        );
        // 任意一天都应当有副标签（农历日名或月名），不会是空串。
        for d in 1..=28 {
            assert!(
                !sub_label(2026, 11, d).0.is_empty(),
                "2026-11-{d} 没有副标签"
            );
        }
    }

    /// 干支：2024 年是甲辰（龙）年，立春之后才算。
    #[test]
    fn ganzhi_year_turns_at_lichun() {
        // 2024-02-03 还没立春，算癸卯年。
        assert!(
            ganzhi_text(2024, 2, 3).starts_with("癸卯年"),
            "{}",
            ganzhi_text(2024, 2, 3)
        );
        // 2024-02-05 已过立春，算甲辰年。
        assert!(
            ganzhi_text(2024, 2, 5).starts_with("甲辰年"),
            "{}",
            ganzhi_text(2024, 2, 5)
        );
    }

    /// 干支日：六十甲子循环，相邻两天必须相邻。
    #[test]
    fn ganzhi_day_cycles_by_sixty() {
        let a = ganzhi_text(2026, 11, 2);
        let b = ganzhi_text(2026, 11, 3);
        let day_of = |s: &str| s.rsplit("  ").next().unwrap_or("").to_owned();
        assert_ne!(day_of(&a), day_of(&b));

        // 60 天之后日柱完全相同（年/月柱可能变了，只比日柱）。
        let later = civil_from_days(days_from_civil(2026, 11, 2) + 60);
        let c = ganzhi_text(later.0, later.1, later.2);
        assert_eq!(day_of(&a), day_of(&c), "60 天后日柱应当相同");
    }

    /// 下一个节气：当天之后，且不超过 16 天（两个节气之间最多 16 天）。
    #[test]
    fn next_term_is_within_a_fortnight() {
        for (y, m, d) in [(2026, 11, 2), (2024, 2, 4), (1999, 12, 31)] {
            let (_, n, days) = next_term(y, m, d).expect("总能找到");
            assert!(
                (1..=16).contains(&days),
                "{y}-{m}-{d} 下一个节气还有 {days} 天"
            );
            assert!(n < 24);
        }
    }

    /// 网格：42 格、周一起始、首格是当月 1 号所在周的周一。
    #[test]
    fn grid_starts_on_monday() {
        let s = CalendarState::new((2026, 11, 2));
        let cells = s.cells();
        // 2026-11-01 是周日 → 它所在周的周一是 10-26。
        assert_eq!((cells[0].month, cells[0].day), (10, 26));
        assert!(!cells[0].in_month, "10-26 不属于 11 月");
        // 第 7 格才是 11-01。
        assert_eq!((cells[6].month, cells[6].day), (11, 1));
        assert!(cells[6].in_month);
        // 42 格覆盖 6 整周。
        let first = days_from_civil(cells[0].year, cells[0].month, cells[0].day);
        let last = days_from_civil(cells[41].year, cells[41].month, cells[41].day);
        assert_eq!(last - first, 41);
    }

    /// 翻页：12 月 +1 变成次年 1 月；1 月 -1 变成上年 12 月。
    #[test]
    fn stepping_wraps_across_years() {
        let mut s = CalendarState::new((2026, 12, 15));
        s.step(0, 1);
        assert_eq!((s.year, s.month), (2027, 1));
        s.step(0, -1);
        assert_eq!((s.year, s.month), (2026, 12));
        s.step(1, 0);
        assert_eq!(s.year, 2027);
    }

    /// 翻页到边界要夹住，不能越界读农历表。
    #[test]
    fn stepping_is_clamped_to_the_table() {
        let mut s = CalendarState::new((2026, 1, 1));
        s.step(-200, 0);
        assert_eq!(s.year, MIN_YEAR);
        let mut s = CalendarState::new((2026, 1, 1));
        s.step(200, 0);
        assert_eq!(s.year, MAX_YEAR - 1);
        // 夹住之后月份仍然合法。
        assert!((1..=12).contains(&s.month));
    }

    /// 点前后月的格子会把视图翻过去。
    #[test]
    fn selecting_a_leading_cell_flips_the_month() {
        let mut s = CalendarState::new((2026, 11, 2));
        let cell = &s.cells()[0];
        let (y, m, d) = (cell.year, cell.month, cell.day);
        s.select(y, m, d);
        assert_eq!((s.year, s.month), (2026, 10));
        assert_eq!(s.selected, (2026, 10, 26));
    }
}
