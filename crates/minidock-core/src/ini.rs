//! `MiniDock.ini` 的读、写与保真。
//!
//! 为什么不用第三方 ini crate：这个文件不是通用配置，它有三个硬约束，
//! 现成 crate 一个都不满足：
//!
//! 1. **UTF-16LE + BOM**。原版用 `WritePrivateProfileStringW`，文件是
//!    UTF-16LE 且带 `FF FE` 头。用 UTF-8 写回去，C 版再读就会乱码。
//! 2. **键序稳定**。配置要在 C 版与 Rust 版之间来回读写；每次保存都把
//!    键序打乱会产生无意义的 diff，也让"改了什么"无从判断。
//! 3. **保留未知键**。老版本读新版本写的 ini 时，不认识的键必须原样留着，
//!    否则一次运行就抹掉别人的配置。
//!
//! 这里实现的是"够用的 profile API 子集"：段、`k=v`、`;` 注释、
//! 大小写不敏感的查找。不做转义 —— 原版也不做，`|` 分隔的条目格式
//! 见 `CONFIG.md` 的已知限制。

use std::fs;
use std::io;
use std::path::Path;

/// 文件编码。读进来是什么，写回去就是什么。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Encoding {
    /// `FF FE` + 每字符 2 字节小端。原版行为。
    #[default]
    Utf16Le,
    /// `FE FF` + 大端。
    Utf16Be,
    /// 无 BOM 的 UTF-8（含纯 ASCII）。手工编辑过的文件常见。
    Utf8,
}

/// 一个段。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub name: String,
    pub entries: Vec<(String, String)>,
}

/// 整个 ini 文件。
#[derive(Clone, Debug, Default)]
pub struct Ini {
    pub encoding: Encoding,
    pub sections: Vec<Section>,
}

impl Ini {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 解码字节流。BOM 决定编码；没有 BOM 时按 UTF-8 处理，
    /// 失败再退回 UTF-16LE 猜测（Windows 上偶尔会遇到无 BOM 的宽字符文件）。
    #[must_use]
    pub fn decode(bytes: &[u8]) -> (String, Encoding) {
        if bytes.starts_with(&[0xFF, 0xFE]) {
            let (pairs, _) = bytes[2..].as_chunks::<2>();
            let units: Vec<u16> = pairs.iter().map(|c| u16::from_le_bytes(*c)).collect();
            (String::from_utf16_lossy(&units), Encoding::Utf16Le)
        } else if bytes.starts_with(&[0xFE, 0xFF]) {
            let (pairs, _) = bytes[2..].as_chunks::<2>();
            let units: Vec<u16> = pairs.iter().map(|c| u16::from_be_bytes(*c)).collect();
            (String::from_utf16_lossy(&units), Encoding::Utf16Be)
        } else if bytes.is_empty() {
            (String::new(), Encoding::Utf8)
        } else {
            match std::str::from_utf8(bytes) {
                Ok(s) => (s.to_owned(), Encoding::Utf8),
                // 不是合法 UTF-8，又没 BOM：按 UTF-16LE 再试一次。
                Err(_) => {
                    let (pairs, _) = bytes.as_chunks::<2>();
                    let units: Vec<u16> = pairs.iter().map(|c| u16::from_le_bytes(*c)).collect();
                    (String::from_utf16_lossy(&units), Encoding::Utf16Le)
                }
            }
        }
    }

    /// 编码成字节流，写入 BOM。
    #[must_use]
    pub fn encode(&self, encoding: Encoding) -> Vec<u8> {
        let text = self.to_text();
        match encoding {
            Encoding::Utf8 => text.into_bytes(),
            Encoding::Utf16Le => {
                let mut out = vec![0xFF, 0xFE];
                for u in text.encode_utf16() {
                    out.extend_from_slice(&u.to_le_bytes());
                }
                out
            }
            Encoding::Utf16Be => {
                let mut out = vec![0xFE, 0xFF];
                for u in text.encode_utf16() {
                    out.extend_from_slice(&u.to_be_bytes());
                }
                out
            }
        }
    }

    /// 解析文本。
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut ini = Ini::new();
        let mut current: Option<usize> = None;

        for raw in text.split('\n') {
            let line = raw.trim_end_matches('\r').trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix('[') {
                if let Some(end) = rest.find(']') {
                    let name = rest[..end].trim().to_owned();
                    current = Some(ini.ensure_section(&name));
                    continue;
                }
            }
            if let Some(eq) = line.find('=') {
                let key = line[..eq].trim();
                if key.is_empty() {
                    continue;
                }
                let value = line[eq + 1..].trim().to_owned();
                // 段外的键：Windows profile API 会忽略，这里同样丢弃。
                if let Some(idx) = current {
                    ini.set_in_section(idx, key, value);
                }
            }
        }
        ini
    }

    /// 序列化为文本（CRLF，与 profile API 一致）。
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for (i, sec) in self.sections.iter().enumerate() {
            if i > 0 {
                out.push_str("\r\n");
            }
            out.push('[');
            out.push_str(&sec.name);
            out.push_str("]\r\n");
            for (k, v) in &sec.entries {
                out.push_str(k);
                out.push('=');
                out.push_str(v);
                out.push_str("\r\n");
            }
        }
        out
    }

    fn section_index(&self, name: &str) -> Option<usize> {
        self.sections
            .iter()
            .position(|s| s.name.eq_ignore_ascii_case(name.trim()))
    }

    fn ensure_section(&mut self, name: &str) -> usize {
        if let Some(i) = self.section_index(name) {
            return i;
        }
        self.sections.push(Section {
            name: name.trim().to_owned(),
            entries: Vec::new(),
        });
        self.sections.len() - 1
    }

    fn set_in_section(&mut self, idx: usize, key: &str, value: String) {
        let sec = &mut self.sections[idx];
        if let Some(pos) = sec
            .entries
            .iter()
            .position(|(k, _)| k.eq_ignore_ascii_case(key))
        {
            sec.entries[pos].1 = value;
        } else {
            sec.entries.push((key.trim().to_owned(), value));
        }
    }

    /// 读字符串。段或键不存在返回 `None`，存在但为空返回 `Some("")`。
    #[must_use]
    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        let idx = self.section_index(section)?;
        self.sections[idx]
            .entries
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }

    /// 读整数。语义对齐 `GetPrivateProfileInt`：键不存在返回 `None`，
    /// 存在但非数字返回 `Some(0)`。
    #[must_use]
    pub fn get_i32(&self, section: &str, key: &str) -> Option<i32> {
        self.get(section, key)
            .map(|v| v.trim().parse::<i32>().unwrap_or(0))
    }

    /// 写字符串。段/键不存在则追加（保持首次出现的顺序）。
    pub fn set(&mut self, section: &str, key: &str, value: impl Into<String>) {
        let idx = self.ensure_section(section);
        self.set_in_section(idx, key, value.into());
    }

    pub fn set_i32(&mut self, section: &str, key: &str, value: i32) {
        self.set(section, key, value.to_string());
    }

    /// 删除整个段（原版 `WritePrivateProfileStringW(sec, NULL, NULL, ...)`）。
    pub fn remove_section(&mut self, section: &str) {
        self.sections
            .retain(|s| !s.name.eq_ignore_ascii_case(section));
    }

    /// 段内键值对数量。用于 `Count` 这类自校验字段。
    #[must_use]
    pub fn len_of(&self, section: &str) -> usize {
        self.section_index(section)
            .map_or(0, |i| self.sections[i].entries.len())
    }

    /// 读文件。文件不存在时返回空 ini（而不是错误）——首次运行就是这种状态。
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        match fs::read(path) {
            Ok(bytes) => {
                let (text, encoding) = Self::decode(&bytes);
                let mut ini = Self::parse(&text);
                ini.encoding = encoding;
                Ok(ini)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::new()),
            Err(e) => Err(e),
        }
    }

    /// 原子写入：先写 `<path>.tmp`，再改名覆盖。
    ///
    /// 原版是逐键 `WritePrivateProfileStringW`，进程在保存中途被杀就会留下
    /// 半截文件；`pr_en.md` 提到的"部分写入的 ini"就是这么来的。
    pub fn save(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        let tmp = path.with_extension("ini.tmp");
        fs::write(&tmp, self.encode(self.encoding))?;
        // Windows 上 rename 会覆盖已存在的目标（MoveFileEx + REPLACE_EXISTING）。
        match fs::rename(&tmp, path) {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sections_and_keys() {
        let ini = Ini::parse("[Settings]\r\nA=1\r\nB = two \r\n; comment\r\n[Items]\r\nI0=x\r\n");
        assert_eq!(ini.get("Settings", "A"), Some("1"));
        assert_eq!(ini.get("Settings", "B"), Some("two"));
        assert_eq!(ini.get("Items", "I0"), Some("x"));
        assert_eq!(ini.len_of("Settings"), 2);
    }

    #[test]
    fn lookup_is_case_insensitive() {
        let ini = Ini::parse("[settings]\niconsize=64\n");
        assert_eq!(ini.get("Settings", "IconSize"), Some("64"));
        assert_eq!(ini.get("SETTINGS", "ICONSIZE"), Some("64"));
    }

    #[test]
    fn values_may_contain_equals_and_pipes() {
        let ini = Ini::parse("[Items]\nI0=0|0|a=b|C:\\x| |0|\n");
        assert_eq!(ini.get("Items", "I0"), Some("0|0|a=b|C:\\x| |0|"));
    }

    #[test]
    fn set_keeps_first_position() {
        let mut ini = Ini::parse("[Settings]\nA=1\nB=2\n");
        ini.set("Settings", "a", "9");
        let text = ini.to_text();
        let a = text.find("A=9").unwrap();
        let b = text.find("B=2").unwrap();
        assert!(a < b, "覆盖已有键不应改变顺序:\n{text}");
    }

    #[test]
    fn unknown_sections_survive_roundtrip() {
        let src = "[Settings]\nOpacity=60\n[Future]\nFoo=bar\n";
        let mut ini = Ini::parse(src);
        ini.set("Settings", "Opacity", "70");
        let again = Ini::parse(&ini.to_text());
        assert_eq!(again.get("Future", "Foo"), Some("bar"));
        assert_eq!(again.get("Settings", "Opacity"), Some("70"));
    }

    #[test]
    fn utf16le_bom_roundtrip() {
        let mut ini = Ini::parse("[Settings]\nOpacity=60\n");
        ini.set("Settings", "Name", "中文名称");
        let bytes = ini.encode(Encoding::Utf16Le);
        assert_eq!(&bytes[..2], &[0xFF, 0xFE]);
        let (text, enc) = Ini::decode(&bytes);
        assert_eq!(enc, Encoding::Utf16Le);
        let back = Ini::parse(&text);
        assert_eq!(back.get("Settings", "Name"), Some("中文名称"));
    }

    #[test]
    fn utf8_without_bom_is_kept() {
        let bytes = b"[Settings]\nOpacity=60\n";
        let (text, enc) = Ini::decode(bytes);
        assert_eq!(enc, Encoding::Utf8);
        assert_eq!(Ini::parse(&text).get("Settings", "Opacity"), Some("60"));
    }

    #[test]
    fn get_i32_matches_profile_api_semantics() {
        let ini = Ini::parse("[S]\nA=42\nB=abc\n");
        assert_eq!(ini.get_i32("S", "A"), Some(42));
        assert_eq!(ini.get_i32("S", "B"), Some(0));
        assert_eq!(ini.get_i32("S", "Missing"), None);
    }

    #[test]
    fn file_roundtrip_is_atomic_and_encoding_stable() {
        let dir = std::env::temp_dir().join("minidock_ini_test");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("MiniDock.ini");

        let mut ini = Ini::new();
        ini.set("Settings", "Opacity", "66");
        ini.save(&path).expect("save");

        let loaded = Ini::load(&path).expect("load");
        assert_eq!(loaded.encoding, Encoding::Utf16Le);
        assert_eq!(loaded.get("Settings", "Opacity"), Some("66"));
        // 临时文件必须已被 rename 掉，不能留垃圾。
        assert!(!path.with_extension("ini.tmp").exists());

        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&dir);
    }

    #[test]
    fn missing_file_is_empty_not_error() {
        let ini = Ini::load("Z:/definitely/not/here/MiniDock.ini").expect("missing -> empty");
        assert!(ini.sections.is_empty());
    }
}
