//! 起动台的扫描：开始菜单里的 `.lnk`。
//!
//! 递归两个「程序」目录
//! （全局 + 当前用户），收集快捷方式，按名字排序去重。
//!
//! 只读文件系统，不起 COM，也不加载图标 —— 图标由 `gpu` 层按需取。

use std::path::Path;

// 条目类型定义在 core：模型层要用它做"吸收扫描结果"，两边共用同一个类型，
// 省掉一次没有意义的转换。
pub use nooksill_core::launchpad::AppEntry;

/// 名字里带这些词的是卸载器 / 说明文档，不该出现在起动台里。
///
/// 用 `StrStrIW` 做子串匹配，大小写不敏感。
const BLOCK_WORDS: [&str; 5] = ["uninstall", "卸载", "readme", "帮助", "help"];

/// 递归深度上限。是 4（`depth < 4`）。
const MAX_DEPTH: usize = 4;

/// 扫开始菜单。
///
/// 顺序：先全局目录、再用户目录，各自递归；最后按名字排序并去重。
#[must_use]
pub fn scan() -> Vec<AppEntry> {
    let mut out = Vec::new();
    for dir in start_menu_dirs() {
        walk(&dir, 0, &mut out);
    }
    // 是先 qsort 再逐个 `lstrcmpiW` 去重。
    out.sort_by(|a, b| cmp_ci(&a.name, &b.name));
    out.dedup_by(|a, b| cmp_ci(&a.name, &b.name) == std::cmp::Ordering::Equal);
    out
}

/// 大小写不敏感比较。`lstrcmpiW` 的近似 —— 我们要的只是"稳定且近似字典序"。
fn cmp_ci(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase().cmp(&b.to_lowercase())
}

/// 名字是否该被跳过。
#[must_use]
pub fn is_blocked(name: &str) -> bool {
    let lower = name.to_lowercase();
    BLOCK_WORDS.iter().any(|w| lower.contains(w))
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<AppEntry>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for ent in rd.flatten() {
        let name = ent.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with('.') {
            continue;
        }
        let path = ent.path();
        let is_dir = ent.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            if depth + 1 < MAX_DEPTH {
                walk(&path, depth + 1, out);
            }
            continue;
        }
        // 只要 .lnk（`PathMatchSpecW(fd.cFileName, L"*.lnk")`）。
        if !name.to_lowercase().ends_with(".lnk") {
            continue;
        }
        let stem = name.trim_end_matches(".lnk").trim_end_matches(".LNK");
        if is_blocked(stem) {
            continue;
        }
        out.push(AppEntry {
            name: stem.to_owned(),
            path: path.to_string_lossy().into_owned(),
        });
    }
}

/// 两个开始菜单目录（全局 `CSIDL_COMMON_PROGRAMS`、用户 `CSIDL_PROGRAMS`）。
fn start_menu_dirs() -> Vec<std::path::PathBuf> {
    let mut v = Vec::new();
    if let Ok(p) = std::env::var("ProgramData") {
        v.push(Path::new(&p).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    if let Ok(p) = std::env::var("APPDATA") {
        v.push(Path::new(&p).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 卸载器与说明文档要被过滤，正常应用不能误伤。
    #[test]
    fn blocked_words_filter_installers() {
        assert!(is_blocked("Uninstall Steam"));
        assert!(is_blocked("卸载 微信"));
        assert!(is_blocked("readme"));
        assert!(is_blocked("帮助文档"));
        assert!(!is_blocked("微信"));
        assert!(!is_blocked("Visual Studio Code"));
        // "helped" 里没有 "help" 吗？有 —— 也是子串匹配，这里保持一致。
        assert!(is_blocked("Helper"));
    }

    /// 扫描结果必须按名字有序，且没有重名。
    #[test]
    fn scan_is_sorted_and_deduped() {
        let apps = scan();
        for w in apps.windows(2) {
            assert!(
                cmp_ci(&w[0].name, &w[1].name) != std::cmp::Ordering::Greater,
                "{} 排在 {} 前面但字典序更大",
                w[0].name,
                w[1].name
            );
        }
        // 本机装了东西，通常不会是空；空也不失败（CI 环境可能没有开始菜单）。
        for a in &apps {
            assert!(!a.name.is_empty());
            assert!(a.path.to_lowercase().ends_with(".lnk"), "{}", a.path);
        }
    }
}
