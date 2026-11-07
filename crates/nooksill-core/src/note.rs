//! 速记：纯数据模型与 `[Notes]` 段的读写。
//!
//! 上游把速记做成一个便签窗口，我们沿用语义但**不照抄它的存储**：
//!
//! ```ini
//! [Notes]
//! N=2
//! T0=购物清单
//! B0=牛奶\n面包\n鸡蛋
//! T1=想法
//! B1=...
//! ```
//!
//! 正文按**转义后的单行**存。ini 的值区不能出现真实换行（会把后面所有
//! 行吞掉、整段作废），所以 `\` → `\\`、`\n` → 字面 `\n`。往返必须完全
//! 还原，转义函数是纯函数，有单测钉住。
//!
//! 这个段是 Rust 版独有的，上游不认识；`Ini` 会原样保留未知段，所以
//! 用同一份 ini 在上游和我们之间切换不会丢数据。

use crate::ini::Ini;

/// `[Notes]` 段名。
pub const SECTION: &str = "Notes";

/// 速记条数上限。
pub const MAX_NOTES: usize = 32;
/// 标题上限（字符数，不是字节）。
pub const MAX_TITLE: usize = 64;
/// 正文上限（字符数）。
pub const MAX_BODY: usize = 4096;

/// 一条速记。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Note {
    pub title: String,
    pub body: String,
}

impl Note {
    #[must_use]
    pub fn new(title: &str, body: &str) -> Self {
        Self {
            title: clean_one_line(title, MAX_TITLE),
            body: clean_body(body),
        }
    }

    /// 列表里的预览：正文第一行，没有正文时给个占位。
    #[must_use]
    pub fn preview(&self) -> String {
        self.body
            .lines()
            .find(|l| !l.trim().is_empty())
            .map_or_else(|| "（空）".to_owned(), |l| truncate_chars(l, 48))
    }

    /// 显示名：没标题时退回正文首行。
    #[must_use]
    pub fn display_title(&self) -> String {
        if self.title.trim().is_empty() {
            let first = self.preview();
            if first == "（空）" {
                "未命名速记".to_owned()
            } else {
                first
            }
        } else {
            self.title.clone()
        }
    }
}

/// 一整本速记。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NoteBook {
    pub notes: Vec<Note>,
}

impl NoteBook {
    /// 从 ini 读。`N` 会被夹紧，坏行跳过而不是中止加载。
    #[must_use]
    pub fn load(ini: &Ini) -> Self {
        let n = ini
            .get_i32(SECTION, "N")
            .unwrap_or(0)
            .clamp(0, MAX_NOTES as i32);
        let mut notes = Vec::new();
        for i in 0..n {
            let title = ini.get(SECTION, &format!("T{i}")).unwrap_or_default();
            let body = ini.get(SECTION, &format!("B{i}")).unwrap_or_default();
            // 标题和正文都空的行没有意义，跳过（也顺带挡住 Count 被手改大
            // 之后塞进来的一堆空便签）。
            if title.is_empty() && body.is_empty() {
                continue;
            }
            notes.push(Note {
                title: clean_one_line(title, MAX_TITLE),
                body: clean_body(&unescape(body)),
            });
        }
        Self { notes }
    }

    /// 写回 `[Notes]`。
    ///
    /// 先整段删掉再写：条数变少时不会留下旧的 `T{i}` / `B{i}`。
    /// 这是 Rust 版独有的段，没有"保留未知键"的兼容义务。
    pub fn save(&self, ini: &mut Ini) {
        ini.remove_section(SECTION);
        let n = self.notes.len().min(MAX_NOTES);
        ini.set_i32(SECTION, "N", n as i32);
        for (i, note) in self.notes.iter().take(n).enumerate() {
            ini.set(SECTION, &format!("T{i}"), &note.title);
            ini.set(SECTION, &format!("B{i}"), escape(&note.body));
        }
    }

    /// 追加一条，返回它的下标。满了返回 `None`。
    pub fn add(&mut self, title: &str, body: &str) -> Option<usize> {
        if self.notes.len() >= MAX_NOTES {
            return None;
        }
        self.notes.push(Note::new(title, body));
        Some(self.notes.len() - 1)
    }

    /// 改一条。下标越界返回 `false`。
    pub fn update(&mut self, index: usize, title: &str, body: &str) -> bool {
        let Some(n) = self.notes.get_mut(index) else {
            return false;
        };
        *n = Note::new(title, body);
        true
    }

    /// 删一条。下标越界返回 `false`。
    pub fn remove(&mut self, index: usize) -> bool {
        if index >= self.notes.len() {
            return false;
        }
        self.notes.remove(index);
        true
    }
}

/// 正文转义：反斜杠与换行。其它控制字符换成空格。
#[must_use]
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// 反转义。遇到 `\x` 这种不认识的组合就原样保留，不吞字符。
#[must_use]
pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// 标题清洗：去掉换行与控制字符，按字符数截断。
fn clean_one_line(s: &str, max: usize) -> String {
    let t: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    truncate_chars(t.trim(), max)
}

/// 正文清洗：保留换行，去掉其它控制字符，按字符数截断。
fn clean_body(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
        .collect();
    truncate_chars(&t, MAX_BODY)
}

fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_roundtrips_newlines_and_backslashes() {
        let s = "第一行\n含\\反斜杠\n\\n 是字面量";
        assert_eq!(unescape(&escape(s)), s);
    }

    #[test]
    fn escape_never_emits_a_real_newline() {
        let out = escape("a\nb\r\nc\td");
        assert!(!out.contains('\n'));
        assert!(!out.contains('\r'));
        assert!(out.contains("\\n"));
    }

    #[test]
    fn unknown_escape_is_kept_not_swallowed() {
        assert_eq!(unescape(r"a\qb"), r"a\qb");
        assert_eq!(unescape("末尾\\"), "末尾\\");
    }

    #[test]
    fn notebook_roundtrips_through_ini() {
        let mut book = NoteBook::default();
        book.add("购物清单", "牛奶\n面包\\坚果").unwrap();
        book.add("想法", "第二行\n第四行").unwrap();
        let mut ini = Ini::new();
        book.save(&mut ini);
        assert_eq!(ini.get_i32(SECTION, "N"), Some(2));
        assert!(!ini.get(SECTION, "B0").unwrap().contains('\n'));

        let back = NoteBook::load(&ini);
        assert_eq!(back, book);
    }

    #[test]
    fn save_clears_stale_entries() {
        let mut book = NoteBook::default();
        for i in 0..3 {
            book.add(&format!("n{i}"), "x").unwrap();
        }
        let mut ini = Ini::new();
        book.save(&mut ini);
        assert!(book.remove(0));
        assert!(book.remove(0));
        book.save(&mut ini);
        assert_eq!(ini.get_i32(SECTION, "N"), Some(1));
        assert!(ini.get(SECTION, "T1").is_none());
        assert!(ini.get(SECTION, "B2").is_none());
        assert_eq!(NoteBook::load(&ini).notes.len(), 1);
    }

    #[test]
    fn load_clamps_a_hostile_count() {
        let mut ini = Ini::new();
        ini.set(SECTION, "N", "999999");
        ini.set(SECTION, "T0", "only one");
        ini.set(SECTION, "B0", "body");
        let book = NoteBook::load(&ini);
        assert_eq!(book.notes.len(), 1);
        assert!(book.notes.len() <= MAX_NOTES);
    }

    #[test]
    fn titles_are_one_line_and_capped() {
        let n = Note::new("坏\n标题\u{1}带控制符", "正文");
        assert!(!n.title.contains('\n'));
        assert!(!n.title.contains('\u{1}'));
        let long = "字".repeat(MAX_TITLE + 20);
        assert_eq!(Note::new(&long, "").title.chars().count(), MAX_TITLE);
    }

    #[test]
    fn display_title_falls_back_to_the_first_body_line() {
        let n = Note::new("", "第一行\n第二行");
        assert_eq!(n.display_title(), "第一行");
        assert_eq!(n.preview(), "第一行");
        assert_eq!(Note::new("", "").display_title(), "未命名速记");
    }

    #[test]
    fn add_and_update_respect_bounds() {
        let mut book = NoteBook::default();
        for i in 0..MAX_NOTES {
            assert!(book.add(&format!("n{i}"), "x").is_some());
        }
        assert!(book.add("overflow", "x").is_none());
        assert!(book.update(0, "改过", "新正文"));
        assert_eq!(book.notes[0].title, "改过");
        assert!(!book.update(999, "x", "y"));
        assert!(!book.remove(999));
    }
}
