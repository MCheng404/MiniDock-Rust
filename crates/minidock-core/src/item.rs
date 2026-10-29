//! Dock 条目模型与 `[Items]` / `[Groups]` 的序列化。
//!
//! 存储格式（与原版 1:1 兼容，**不要**轻易改）：
//!
//! ```text
//! [Items]
//! Count=3
//! I0=<type>|<appmode>|<name>|<path>|<icon_src>|<fit>|<args>
//! I1=...
//! [Groups]
//! N0=2                       ; 第 0 个条目是组，含 2 个子项
//! G0_0=<同上 7 字段>
//! G0_1=<同上 7 字段>
//! ```
//!
//! `type`：0 文件 / 1 文件夹 / 2 网址 / 3 分割线 / 4 应用组 / 5 回收站 / 6 速记。
//!
//! 字段上限来自原版的 `lstrcpynW` 调用（`sizeof` 含结尾 NUL，故可用字符数少 1）。
//! 这里用 [`truncate_units`] 按 **UTF-16 单元**截断，而不是按 `char` ——
//! 一个 emoji 占 2 个单元，按 `char` 截会得到比 C 版更长的字符串，
//! 进而让 `I0` 行的总长度越过 profile API 的缓冲区边界。

use crate::MAX_KIDS;

/// `name` 缓冲区大小（含 NUL）。对应 `Item.name[128]`。
pub const NAME_CAP: usize = 128;
/// `path` 缓冲区大小。对应 `Item.path[1024]`。
pub const PATH_CAP: usize = 1024;
/// `icon_src` 缓冲区大小。对应 `Item.icon_src[512]`。
pub const ICON_CAP: usize = 512;
/// `args` 缓冲区大小。对应 `Item.args[260]`。
pub const ARGS_CAP: usize = 260;
/// 解析后的启动目标缓冲区大小。对应 `Item.target[MAX_PATH]`。
pub const TARGET_CAP: usize = 260;

/// 一个条目行的字段数。
const FIELD_COUNT: usize = 7;

/// 条目类型。数值即 ini 里的 `type` 字段。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum ItemKind {
    #[default]
    File = 0,
    Folder = 1,
    Url = 2,
    Separator = 3,
    Group = 4,
    Trash = 5,
    Note = 6,
}

impl ItemKind {
    /// 越界返回 `None`（解析时应当**丢弃整条**，而不是猜一个类型）。
    #[must_use]
    pub const fn from_code(v: i32) -> Option<Self> {
        Some(match v {
            0 => Self::File,
            1 => Self::Folder,
            2 => Self::Url,
            3 => Self::Separator,
            4 => Self::Group,
            5 => Self::Trash,
            6 => Self::Note,
            _ => return None,
        })
    }

    #[must_use]
    pub const fn code(self) -> i32 {
        self as i32
    }

    #[must_use]
    pub const fn is_separator(self) -> bool {
        matches!(self, Self::Separator)
    }

    /// 组和速记有专用绘制路径，不画底盘（对应 `RenderFrame` 里的判断）。
    #[must_use]
    pub const fn has_own_icon(self) -> bool {
        matches!(self, Self::Group | Self::Note)
    }
}

/// Dock 上的一个条目。
///
/// 前半部分是**持久化字段**（写进 ini），后半部分是**运行期状态**
/// （图标位图、动画插值、运行指示）。运行期状态不参与序列化。
#[derive(Clone, Debug)]
pub struct Item {
    pub kind: ItemKind,
    /// 网页应用的打开方式：0 = 默认浏览器，1 = `--app` 窗口。
    pub app_mode: i32,
    /// 图标填充方式（0 = 等比留白，其它值见 `FitBitmap`）。
    pub fit: i32,
    /// 图标是否已缓存到 `icons/` 下（对应 `fav`）。
    pub fav: bool,
    /// 临时条目（运行中但未固定）。**不写入 ini**。
    pub temp: bool,

    pub name: String,
    pub path: String,
    pub icon_src: String,
    pub args: String,
    /// 解析后的启动目标（.lnk 解引用、环境变量展开之后）。
    pub target: String,

    /// 当前图标边长（放大动画的插值结果）。
    pub cur: f32,
    /// 重排时的横向偏移（回弹动画）。
    pub slide: f32,
    /// 上一帧的槽位左边界，用于计算 `slide` 的初值。
    pub prev_l: f32,
    /// 是否正在运行（由窗口扫描填充）。
    pub running: bool,
    /// 启动弹跳的起始 tick（`GetTickCount` 语义）。
    pub bounce: u32,

    /// 子项。仅 `ItemKind::Group` 使用，上限 [`MAX_KIDS`]。
    pub kids: Vec<Item>,
}

impl Item {
    /// 构造一个只有持久化字段的条目。
    #[must_use]
    pub fn new(kind: ItemKind, name: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            kind,
            app_mode: 0,
            fit: 0,
            fav: false,
            temp: false,
            name: truncate_units(&name.into(), NAME_CAP - 1),
            path: truncate_units(&path.into(), PATH_CAP - 1),
            icon_src: String::new(),
            args: String::new(),
            target: String::new(),
            cur: 0.0,
            slide: 0.0,
            prev_l: 0.0,
            running: false,
            bounce: 0,
            kids: Vec::new(),
        }
    }

    /// 解析一行 `type|appmode|name|path|icon_src|fit|args`。
    ///
    /// 失败条件与原版 `ParseItemLine` 一致：字段少于 4 个，或 `type` 不在 0..=6。
    /// 注意 `icon_src` 为空串时视作"无自定义图标"。
    #[must_use]
    pub fn parse_line(line: &str) -> Option<Self> {
        let mut f = line.splitn(FIELD_COUNT, '|');
        let f0 = f.next()?;
        let f1 = f.next();
        let f2 = f.next();
        let f3 = f.next()?;
        let kind = ItemKind::from_code(atoi_like(f0))?;
        let app_mode = f1.map_or(0, atoi_like);
        let name = f2.map_or("", |v| v);
        let path = f3;
        let icon_src = f.next().unwrap_or("");
        let fit = f.next().map_or(0, atoi_like);
        let args = f.next().unwrap_or("");

        let mut it = Self::new(kind, name, path);
        it.app_mode = app_mode;
        it.fit = fit;
        it.icon_src = truncate_units(icon_src, ICON_CAP - 1);
        it.args = truncate_units(args, ARGS_CAP - 1);
        Some(it)
    }

    /// 序列化为一行。写入前会清洗控制字符与 `|`（见 [`Self::sanitize_for_ini`]）。
    #[must_use]
    pub fn to_line(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}|{}|{}",
            self.kind.code(),
            self.app_mode,
            Self::sanitize_for_ini(&self.name),
            Self::sanitize_for_ini(&self.path),
            Self::sanitize_for_ini(&self.icon_src),
            self.fit,
            Self::sanitize_for_ini(&self.args),
        )
    }

    /// 是否应写入 ini。临时条目与"回收站/速记"这类内置条目例外：
    /// 原版只跳过 `temp`，内置条目仍会持久化（它们的 path 是 `shell:` 协议）。
    #[must_use]
    pub fn is_persistable(&self) -> bool {
        !self.temp
    }

    /// 加入子项。超过 [`MAX_KIDS`] 返回 `false`（原版是静默丢弃）。
    pub fn add_kid(&mut self, kid: Item) -> bool {
        if self.kids.len() >= MAX_KIDS {
            return false;
        }
        self.kids.push(kid);
        true
    }

    /// 清洗一个字段，使它不会破坏 ini 的行/字段结构。
    ///
    /// - `\r` `\n` `\0` → 空格：profile API 的值区不能换行，否则整段作废。
    /// - `|` → 全角 `｜`：它是字段分隔符。原版不做处理，名字里带 `|` 的条目
    ///   会在下一次保存后**整体错位**（type/path 全部左移一格），属于静默数据损坏。
    #[must_use]
    pub fn sanitize_for_ini(s: &str) -> String {
        s.chars()
            .map(|c| match c {
                '\r' | '\n' | '\0' => ' ',
                '|' => '｜',
                other => other,
            })
            .collect()
    }
}

/// 按 UTF-16 单元数截断，语义等价于 `lstrcpynW(dst, src, cap)` 写入的字符数
/// （即最多 `cap - 1` 个单元）。不会切开代理对。
#[must_use]
pub fn truncate_units(s: &str, max_units: usize) -> String {
    let mut used = 0usize;
    let mut out = String::with_capacity(s.len().min(max_units));
    for ch in s.chars() {
        let n = ch.len_utf16();
        if used + n > max_units {
            break;
        }
        used += n;
        out.push(ch);
    }
    out
}

/// 模拟 `_wtoi`：跳过前导空白，取可选符号 + 连续数字，遇到非数字即停止。
///
/// **不能**用 `str::parse` —— 后者对 `"12abc"` 返回错误，而 `_wtoi` 返回 12。
/// ini 是手改过的文件，这种"半合法"值很常见。
#[must_use]
pub fn atoi_like(s: &str) -> i32 {
    let t = s.trim_start();
    let bytes = t.as_bytes();
    let mut i = 0;
    let mut sign = 1i64;
    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
        if bytes[i] == b'-' {
            sign = -1;
        }
        i += 1;
    }
    let start = i;
    let mut acc: i64 = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        acc = (acc * 10 + i64::from(bytes[i] - b'0')).min(i64::from(i32::MAX) + 1);
        i += 1;
    }
    if i == start {
        return 0;
    }
    (sign * acc).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atoi_matches_wtoi() {
        assert_eq!(atoi_like("42"), 42);
        assert_eq!(atoi_like("-7"), -7);
        assert_eq!(atoi_like("  12abc"), 12);
        assert_eq!(atoi_like("abc"), 0);
        assert_eq!(atoi_like(""), 0);
        assert_eq!(atoi_like("+3"), 3);
        assert_eq!(atoi_like("99999999999999999999"), i32::MAX);
    }

    #[test]
    fn parses_a_full_line() {
        let it = Item::parse_line(r"2|1|GitHub|https://github.com|C:\icons\gh.png|1|--profile=x")
            .expect("valid");
        assert_eq!(it.kind, ItemKind::Url);
        assert_eq!(it.app_mode, 1);
        assert_eq!(it.name, "GitHub");
        assert_eq!(it.path, "https://github.com");
        assert_eq!(it.icon_src, r"C:\icons\gh.png");
        assert_eq!(it.fit, 1);
        assert_eq!(it.args, "--profile=x");
    }

    #[test]
    fn rejects_short_or_unknown_lines() {
        assert!(Item::parse_line("0|0|name").is_none()); // 只有 3 段
        assert!(Item::parse_line("9|0|name|path").is_none()); // 未知 type
        assert!(Item::parse_line("-1|0|name|path").is_none());
        assert!(Item::parse_line("").is_none());
    }

    #[test]
    fn minimal_line_defaults_missing_fields() {
        let it = Item::parse_line("3|0|sep|").expect("4 fields is enough");
        assert_eq!(it.kind, ItemKind::Separator);
        assert_eq!(it.icon_src, "");
        assert_eq!(it.fit, 0);
        assert_eq!(it.args, "");
    }

    #[test]
    fn line_roundtrip_is_stable() {
        let it = Item::parse_line(r"0|0|记事本|C:\Windows\notepad.exe||0|").unwrap();
        assert_eq!(Item::parse_line(&it.to_line()).unwrap().name, "记事本");
        assert_eq!(it.to_line(), r"0|0|记事本|C:\Windows\notepad.exe||0|");
    }

    #[test]
    fn separators_in_fields_do_not_corrupt_the_line() {
        let mut it = Item::new(ItemKind::File, "a|b", r"C:\x");
        it.args = "line1\nline2".into();
        let line = it.to_line();
        assert_eq!(
            line.matches('|').count(),
            6,
            "字段分隔符必须恰好 6 个: {line}"
        );
        assert!(!line.contains('\n'));
        let back = Item::parse_line(&line).unwrap();
        assert_eq!(back.name, "a｜b");
        assert_eq!(back.args, "line1 line2");
    }

    #[test]
    fn truncation_counts_utf16_units() {
        // BMP 字符 1 单元，emoji 2 单元。
        assert_eq!(truncate_units("abcdef", 4), "abcd");
        assert_eq!(truncate_units("😀😀😀", 4), "😀😀");
        assert_eq!(truncate_units("😀😀😀", 5), "😀😀");
        assert_eq!(truncate_units("中文名称", 3), "中文名");
    }

    #[test]
    fn kids_are_capped() {
        let mut g = Item::new(ItemKind::Group, "组", "");
        for i in 0..MAX_KIDS {
            assert!(g.add_kid(Item::new(ItemKind::File, format!("k{i}"), "")));
        }
        assert!(!g.add_kid(Item::new(ItemKind::File, "overflow", "")));
        assert_eq!(g.kids.len(), MAX_KIDS);
    }

    #[test]
    fn temp_items_are_not_persisted() {
        let mut it = Item::new(ItemKind::File, "x", "");
        assert!(it.is_persistable());
        it.temp = true;
        assert!(!it.is_persistable());
    }
}
