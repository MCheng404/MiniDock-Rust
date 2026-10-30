//! 判断条目对应的应用是否在运行。
//!
//! 对应 C 版 `ScanProc` 里 `items[i].running = c.seen[i]` 那一段。
//!
//! # 为什么放在 core 而不是 platform
//!
//! `platform::app_window::running_apps()` 负责"系统里现在有哪些 exe 在跑"，
//! 那需要 Win32；而"某个条目该拿什么去那张表里查"是纯逻辑 ——
//! 放这里就能脱离桌面单测，也不必让 core 依赖 `HWND`。
//!
//! # 匹配规则（按优先级）
//!
//! 1. `item.target` 非空 → 用它。这是 `.lnk` 解析出来的真实 exe 路径，
//!    最准。
//! 2. `item.path` 本身以 `.exe` 结尾 → 用它。
//! 3. 其余（未解析的 `.lnk`、文件夹、网址、分割线）→ 不参与匹配。
//!
//! 第 3 条很关键：**不能退化成"按文件名猜测"**。`网易云音乐.lnk` 指向的
//! 是 `cloudmusic.exe`，按名字猜必然猜错，而不猜的结果是"指示点不亮"，
//! 比"猜错了乱亮"要好 —— 后者会让用户以为某个没开的程序正在运行。

use std::collections::HashSet;

use crate::item::{Item, ItemKind};

/// 这个条目应该拿什么键去"运行中的 exe 集合"里查。
///
/// 返回值统一小写（Windows 路径不区分大小写）。
#[must_use]
pub fn running_key(item: &Item) -> Option<String> {
    if item.kind != ItemKind::File {
        return None;
    }
    if !item.target.is_empty() {
        return Some(item.target.to_lowercase());
    }
    if item.path.to_lowercase().ends_with(".exe") {
        return Some(item.path.to_lowercase());
    }
    None
}

/// 按 `running`（运行中的 exe 小写路径集合）刷新所有条目的 `running` 字段。
///
/// 返回**是否有条目状态发生了变化** —— 主循环据此决定要不要重绘。
/// 每次扫描都无条件重绘会让空闲 CPU 从 0% 掉到十几帧。
pub fn refresh_running(items: &mut [Item], running: &HashSet<String>) -> bool {
    let mut changed = false;
    for item in items.iter_mut() {
        let now = running_key(item).is_some_and(|k| running.contains(&k));
        if item.running != now {
            item.running = now;
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_item(name: &str, path: &str) -> Item {
        Item::new(ItemKind::File, name, path)
    }

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    /// `target` 优先于 `path`：`.lnk` 的路径不是 exe，靠 target 才对得上。
    #[test]
    fn target_wins_over_path() {
        let mut it = file_item(
            "Excel",
            r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Excel.lnk",
        );
        it.target = r"C:\Program Files\Microsoft Office\root\Office16\EXCEL.EXE".to_owned();
        assert_eq!(
            running_key(&it),
            Some(r"c:\program files\microsoft office\root\office16\excel.exe".to_owned())
        );
    }

    /// 没有 target 的 `.exe` 条目直接用自己的路径。
    #[test]
    fn exe_path_is_used_when_no_target() {
        let it = file_item("记事本", r"C:\Windows\System32\notepad.exe");
        assert_eq!(
            running_key(&it),
            Some(r"c:\windows\system32\notepad.exe".to_owned())
        );
    }

    /// 未解析的 `.lnk` 不参与匹配 —— 宁可指示点不亮，也不要按名字猜。
    #[test]
    fn unresolved_lnk_never_matches() {
        let it = file_item("网易云音乐", r"C:\Users\X\Desktop\网易云音乐.lnk");
        assert_eq!(running_key(&it), None);
        // 即使集合里有个同名的 exe，也不该匹配上。
        let mut items = [it];
        let changed = refresh_running(&mut items, &set(&[r"c:\x\网易云音乐.exe"]));
        assert!(!changed);
        assert!(!items[0].running);
    }

    /// 非文件条目（分割线、网址、文件夹、回收站）一律不参与。
    #[test]
    fn non_file_kinds_never_match() {
        for kind in [
            ItemKind::Separator,
            ItemKind::Url,
            ItemKind::Folder,
            ItemKind::Trash,
        ] {
            let mut it = Item::new(kind, "x", r"C:\Windows\System32\notepad.exe");
            // 故意把 target 也设上，确认 kind 判断在前面。
            it.target = r"C:\Windows\System32\notepad.exe".to_owned();
            assert_eq!(running_key(&it), None, "{kind:?} 不该参与匹配");
        }
    }

    /// 大小写必须忽略：配置里存的大小写与系统返回的不一定一致。
    #[test]
    fn matching_is_case_insensitive() {
        let mut items = [file_item("记事本", r"C:\Windows\System32\NOTEPAD.EXE")];
        let changed = refresh_running(&mut items, &set(&[r"c:\windows\system32\notepad.exe"]));
        assert!(changed);
        assert!(items[0].running);
    }

    /// 只在状态真的变化时返回 true。
    ///
    /// 这条是性能约束：主循环每 1.2 秒扫一次，若每次都返回 true 就会
    /// 无条件重绘，空闲 CPU 直接失守（M0 花了不少力气才压到 0%）。
    #[test]
    fn reports_change_only_when_state_flips() {
        let mut items = [file_item("记事本", r"C:\Windows\System32\notepad.exe")];
        let running = set(&[r"c:\windows\system32\notepad.exe"]);

        assert!(refresh_running(&mut items, &running), "第一次应当算变化");
        assert!(!refresh_running(&mut items, &running), "状态没变不该再报");
        assert!(refresh_running(&mut items, &set(&[])), "退出后应当算变化");
        assert!(!items[0].running);
    }

    /// 分割线与其它条目共存时，只影响该影响的那些。
    #[test]
    fn separator_is_left_alone() {
        let mut items = [
            file_item("记事本", r"C:\Windows\System32\notepad.exe"),
            Item::new(ItemKind::Separator, "", ""),
        ];
        let changed = refresh_running(&mut items, &set(&[r"c:\windows\system32\notepad.exe"]));
        assert!(changed);
        assert!(items[0].running);
        assert!(!items[1].running, "分割线的 running 必须保持 false");
    }
}
