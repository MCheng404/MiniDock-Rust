//! `[Items]` / `[Groups]` 的读写。对应 C 版 `LoadItems()` / `SaveNow()`。
//!
//! 编号规则（**必须**与 C 版一致，否则旧配置读不出来）：
//! `I{n}` 的 `n` 是"可持久化条目"的下标，临时条目（运行中但未固定）
//! 不占号；组内的 `N{n}` / `G{n}_{k}` 用同一个 `n`。

use crate::ini::Ini;
use crate::item::{Item, ItemKind};
use crate::MAX_ITEMS;
use crate::MAX_KIDS;

/// 从 ini 读出全部条目。解析失败的行被**跳过**，而不是中止加载 ——
/// 一个坏条目不应该让整条 Dock 消失。
#[must_use]
pub fn load_items(ini: &Ini) -> Vec<Item> {
    let count = ini
        .get_i32("Items", "Count")
        .unwrap_or(0)
        .clamp(0, MAX_ITEMS as i32);

    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        let line = ini.get("Items", &format!("I{i}")).unwrap_or_default();
        let Some(mut item) = Item::parse_line(line) else {
            continue;
        };
        if item.kind == ItemKind::Group {
            let nk = ini
                .get_i32("Groups", &format!("N{i}"))
                .unwrap_or(0)
                .clamp(0, MAX_KIDS as i32);
            for k in 0..nk {
                let kid_line = ini.get("Groups", &format!("G{i}_{k}")).unwrap_or_default();
                if let Some(kid) = Item::parse_line(kid_line) {
                    item.add_kid(kid);
                }
            }
        }
        out.push(item);
    }
    out
}

/// 写回全部条目。会先清空两个段，避免旧编号残留。
pub fn save_items(items: &[Item], ini: &mut Ini) {
    ini.remove_section("Items");
    ini.remove_section("Groups");

    let mut n = 0usize;
    for item in items {
        if !item.is_persistable() {
            continue;
        }
        ini.set("Items", &format!("I{n}"), item.to_line());
        if item.kind == ItemKind::Group {
            ini.set_i32("Groups", &format!("N{n}"), item.kids.len() as i32);
            for (k, kid) in item.kids.iter().enumerate() {
                ini.set("Groups", &format!("G{n}_{k}"), kid.to_line());
            }
        }
        n += 1;
        if n >= MAX_ITEMS {
            break;
        }
    }
    ini.set_i32("Items", "Count", n as i32);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_roundtrip() {
        let mut g = Item::new(ItemKind::Group, "开发", "");
        g.add_kid(Item::new(
            ItemKind::File,
            "记事本",
            r"C:\Windows\notepad.exe",
        ));
        g.add_kid(Item::new(ItemKind::Url, "GitHub", "https://github.com"));
        let items = vec![
            Item::new(ItemKind::File, "资源管理器", r"C:\Windows\explorer.exe"),
            g,
            Item::new(ItemKind::Separator, "", ""),
        ];

        let mut ini = Ini::new();
        save_items(&items, &mut ini);
        assert_eq!(ini.get_i32("Items", "Count"), Some(3));
        assert_eq!(ini.get("Groups", "N1"), Some("2"));
        assert!(ini.get("Groups", "G1_1").unwrap().contains("GitHub"));

        let back = load_items(&ini);
        assert_eq!(back.len(), 3);
        assert_eq!(back[1].kind, ItemKind::Group);
        assert_eq!(back[1].kids.len(), 2);
        assert_eq!(back[1].kids[1].name, "GitHub");
        assert_eq!(back[2].kind, ItemKind::Separator);
    }

    #[test]
    fn temp_items_do_not_shift_numbering() {
        let mut temp = Item::new(ItemKind::File, "运行中", "x.exe");
        temp.temp = true;
        let items = vec![
            Item::new(ItemKind::File, "A", "a.exe"),
            temp,
            Item::new(ItemKind::File, "B", "b.exe"),
        ];
        let mut ini = Ini::new();
        save_items(&items, &mut ini);
        assert_eq!(ini.get_i32("Items", "Count"), Some(2));
        assert_eq!(ini.get("Items", "I1").unwrap().split('|').nth(2), Some("B"));
    }

    #[test]
    fn bad_lines_are_skipped_not_fatal() {
        let mut ini = Ini::new();
        ini.set_i32("Items", "Count", 3);
        ini.set("Items", "I0", "0|0|好|a.exe||0|");
        ini.set("Items", "I1", "垃圾数据");
        ini.set("Items", "I2", "1|0|文件夹|C:\\||0|");
        let items = load_items(&ini);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "好");
        assert_eq!(items[1].kind, ItemKind::Folder);
    }

    #[test]
    fn count_is_clamped_to_max_items() {
        let mut ini = Ini::new();
        // 手改 ini 把 Count 写成天文数字，不能让加载循环爆掉。
        ini.set("Items", "Count", "999999");
        let items = load_items(&ini);
        assert!(items.len() <= MAX_ITEMS);
    }

    #[test]
    fn save_clears_stale_entries() {
        let mut ini = Ini::new();
        ini.set("Items", "I7", "0|0|旧|old.exe||0|");
        ini.set("Groups", "N7", "1");
        let items = vec![Item::new(ItemKind::File, "新", "new.exe")];
        save_items(&items, &mut ini);
        assert_eq!(ini.get_i32("Items", "Count"), Some(1));
        assert!(ini.get("Items", "I7").is_none());
        assert!(ini.get("Groups", "N7").is_none());
    }
}
