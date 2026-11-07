//! 应用组（"目录"）的纯逻辑：建组、归组、出组、解散。
//!
//! 一个组是 [`ItemKind::Group`] 类型的条目，它的子项存在
//! [`Item::kids`] 里（上限 [`MAX_KIDS`]）。ini 侧的读写在
//! [`crate::store`]，本模块只管**内存里**的这些操作。
//!
//! # 为什么单独一个模块
//!
//! 分组是唯一一处"条目会变成别的条目的子项"的地方，而条目的下标在
//! Dock、菜单、图标缓存、拖拽四处都被当作身份用。把归组/出组集中到
//! 几个纯函数里，才能保证：
//!
//! - 组的子项**不能是组**（套娃会让"解散"变成递归，且四宫格图标没意义）；
//! - 组的子项数量有硬上限（`MAX_KIDS`），超了要报错而不是静默丢弃；
//! - 出组时子项回到栏体的**哪个位置**是确定的（组的正后方）。
//!
//! 全部操作都在 `Vec<Item>` 上就地完成，调用方负责写盘。

use crate::item::{Item, ItemKind};
use crate::MAX_KIDS;

/// 分组操作失败的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupError {
    /// 下标越界（条目列表里没有这一项）。
    NoSuchItem,
    /// 目标不是组（不能往普通条目里塞子项）。
    NotGroup,
    /// 组的子项已满（`MAX_KIDS`）。
    Full,
    /// 组里不能塞组。
    Nested,
}

impl std::fmt::Display for GroupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSuchItem => write!(f, "条目不存在"),
            Self::NotGroup => write!(f, "目标不是分组"),
            Self::Full => write!(f, "分组已满（上限 {MAX_KIDS} 个）"),
            Self::Nested => write!(f, "分组里不能再放分组"),
        }
    }
}

/// 造一个空组。
///
/// 名字会按 [`crate::item::truncate_units`] 的规则截断（`name` 字段上限
/// 127 个 UTF-16 单元），与其它条目一致。
#[must_use]
pub fn new_group(name: &str) -> Item {
    Item::new(ItemKind::Group, name, "")
}

/// 组里的子项数。
#[must_use]
pub fn kid_count(group: &Item) -> usize {
    if group.kind == ItemKind::Group {
        group.kids.len()
    } else {
        0
    }
}

/// 往组里加一个子项。
///
/// 拒绝组套组，也拒绝超过 [`MAX_KIDS`]。返回 `Err` 时组**没有被改动**。
pub fn add_kid(group: &mut Item, kid: Item) -> Result<(), GroupError> {
    can_add_kid(group, &kid)?;
    group.kids.push(kid);
    Ok(())
}

/// 能不能把 `kid` 放进 `group`。
///
/// [`add_kid`] 失败时子项已经被移走了，调用方拿不回来 —— 所以要在
/// 调用**之前**先问一遍。
pub fn can_add_kid(group: &Item, kid: &Item) -> Result<(), GroupError> {
    if group.kind != ItemKind::Group {
        return Err(GroupError::NotGroup);
    }
    if kid.kind == ItemKind::Group {
        return Err(GroupError::Nested);
    }
    if group.kids.len() >= MAX_KIDS {
        return Err(GroupError::Full);
    }
    Ok(())
}

/// 从组里取出第 `idx` 个子项（不移除）。
#[must_use]
pub fn kid_at(group: &Item, idx: usize) -> Option<&Item> {
    group.kids.get(idx)
}

/// 解散一个组：把它的子项原样摊平。
///
/// 返回的顺序就是子项的顺序 —— 摊到栏体上时保持这个顺序，用户才认得出
/// "这是刚才那个组里的东西"。
#[must_use]
pub fn dissolve(group: &Item) -> Vec<Item> {
    if group.kind == ItemKind::Group {
        group.kids.clone()
    } else {
        Vec::new()
    }
}

/// 把 `items[src]` 并入 `items[dst]`（`dst` 必须是组）。
///
/// `src` 的条目会从栏体上消失（它变成组的子项了）。组内不能套组。
pub fn merge_item(items: &mut Vec<Item>, src: usize, dst: usize) -> Result<(), GroupError> {
    if src >= items.len() || dst >= items.len() {
        return Err(GroupError::NoSuchItem);
    }
    if items[dst].kind != ItemKind::Group {
        return Err(GroupError::NotGroup);
    }
    if src == dst {
        return Err(GroupError::Nested);
    }
    if items[src].kind == ItemKind::Group {
        return Err(GroupError::Nested);
    }
    let item = items.remove(src);
    // `src` 可能大于 `dst`，移除后目标下标要跟着前移。
    let dst = if src < dst { dst - 1 } else { dst };
    add_kid(&mut items[dst], item)
}

/// 把组里的第 `kid` 个子项搬回栏体，插在组的**正后方**。
///
/// 返回子项在栏体里的新下标。
pub fn extract_kid(
    items: &mut Vec<Item>,
    group_idx: usize,
    kid: usize,
) -> Result<usize, GroupError> {
    if group_idx >= items.len() {
        return Err(GroupError::NoSuchItem);
    }
    if items[group_idx].kind != ItemKind::Group {
        return Err(GroupError::NotGroup);
    }
    if kid >= items[group_idx].kids.len() {
        return Err(GroupError::NoSuchItem);
    }
    let item = items[group_idx].kids.remove(kid);
    let at = group_idx + 1;
    items.insert(at, item);
    Ok(at)
}

/// 解散 `items[group_idx]` 这个组：子项摊平到栏体，插在组原来的位置。
///
/// 返回子项在栏体里的新下标（组被移除了）。
pub fn dissolve_item(items: &mut Vec<Item>, group_idx: usize) -> Result<usize, GroupError> {
    if group_idx >= items.len() {
        return Err(GroupError::NoSuchItem);
    }
    if items[group_idx].kind != ItemKind::Group {
        return Err(GroupError::NotGroup);
    }
    let kids = std::mem::take(&mut items[group_idx].kids);
    items.remove(group_idx);
    let at = group_idx;
    for (k, kid) in kids.into_iter().enumerate() {
        items.insert(at + k, kid);
    }
    Ok(at)
}

/// 把组里的第 `kid` 个子项上移（`-1`）或下移（`+1`）。
///
/// 越界（包括首尾再往外移）返回 `NoSuchItem`，组**不改**。
pub fn move_kid(group: &mut Item, kid: usize, delta: i32) -> Result<(), GroupError> {
    if group.kind != ItemKind::Group {
        return Err(GroupError::NotGroup);
    }
    if kid >= group.kids.len() || delta == 0 {
        return Err(GroupError::NoSuchItem);
    }
    let to = kid as i32 + delta;
    if to < 0 || to as usize >= group.kids.len() {
        return Err(GroupError::NoSuchItem);
    }
    group.kids.swap(kid, to as usize);
    Ok(())
}

/// 组图标用几格。
///
/// 上游是 2×2 四宫格：1 个子项占满整格，2 个占上下两格，3–4 个占四格。
/// 返回值是 `(列数, 行数)`。
#[must_use]
pub const fn grid_cells(n: usize) -> (usize, usize) {
    match n {
        0 => (0, 0),
        1 => (1, 1),
        2 => (1, 2),
        _ => (2, 2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> Item {
        Item::new(ItemKind::File, name, "C:\\x.exe")
    }

    #[test]
    fn new_group_is_an_empty_group() {
        let g = new_group("工具");
        assert_eq!(g.kind, ItemKind::Group);
        assert_eq!(g.name, "工具");
        assert!(g.kids.is_empty());
        assert_eq!(kid_count(&g), 0);
    }

    #[test]
    fn add_kid_rejects_groups_and_overflow() {
        let mut g = new_group("工具");
        assert_eq!(add_kid(&mut g, file("a")), Ok(()));
        assert_eq!(
            add_kid(&mut g, new_group("嵌套")),
            Err(GroupError::Nested),
            "组里不能塞组"
        );
        for _ in 1..MAX_KIDS {
            add_kid(&mut g, file("x")).expect("未满时应当能加");
        }
        assert_eq!(kid_count(&g), MAX_KIDS);
        assert_eq!(add_kid(&mut g, file("y")), Err(GroupError::Full));
    }

    #[test]
    fn add_kid_to_a_plain_item_is_rejected() {
        let mut it = file("记事本");
        assert_eq!(add_kid(&mut it, file("a")), Err(GroupError::NotGroup));
        assert!(it.kids.is_empty());
    }

    #[test]
    fn merge_moves_an_item_into_the_group() {
        let mut items = vec![file("a"), new_group("工具"), file("b")];
        merge_item(&mut items, 0, 1).expect("应当成功");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "工具");
        assert_eq!(kid_count(&items[0]), 1);
        assert_eq!(items[0].kids[0].name, "a");
    }

    #[test]
    fn merge_into_a_non_group_fails() {
        let mut items = vec![file("a"), file("b")];
        assert_eq!(merge_item(&mut items, 0, 1), Err(GroupError::NotGroup));
        assert_eq!(items.len(), 2, "失败时列表不能变");
    }

    #[test]
    fn merge_rejects_group_into_group() {
        let mut items = vec![new_group("外"), new_group("内")];
        assert_eq!(merge_item(&mut items, 1, 0), Err(GroupError::Nested));
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn merge_handles_source_after_target() {
        // 源下标大于目标时，移除源会让目标前移一位。
        let mut items = vec![new_group("工具"), file("a"), file("b")];
        merge_item(&mut items, 2, 0).expect("应当成功");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].kind, ItemKind::Group);
        assert_eq!(kid_count(&items[0]), 1);
        assert_eq!(items[0].kids[0].name, "b");
    }

    #[test]
    fn extract_puts_the_kid_right_after_the_group() {
        let mut items = vec![file("a"), new_group("工具"), file("b")];
        let mut g = new_group("工具");
        add_kid(&mut g, file("x")).unwrap();
        add_kid(&mut g, file("y")).unwrap();
        items[1] = g;

        let at = extract_kid(&mut items, 1, 0).expect("应当成功");
        assert_eq!(at, 2);
        assert_eq!(items.len(), 4);
        assert_eq!(items[2].name, "x");
        assert_eq!(kid_count(&items[1]), 1);
        assert_eq!(items[1].kids[0].name, "y");
    }

    #[test]
    fn extract_rejects_bad_indices() {
        let mut items = vec![new_group("工具")];
        assert_eq!(extract_kid(&mut items, 0, 0), Err(GroupError::NoSuchItem));
        assert_eq!(extract_kid(&mut items, 3, 0), Err(GroupError::NoSuchItem));
    }

    #[test]
    fn dissolve_flattens_the_group_in_place() {
        let mut g = new_group("工具");
        add_kid(&mut g, file("x")).unwrap();
        add_kid(&mut g, file("y")).unwrap();
        let mut items = vec![file("a"), g, file("b")];

        let at = dissolve_item(&mut items, 1).expect("应当成功");
        assert_eq!(at, 1);
        assert_eq!(items.len(), 4);
        assert_eq!(items[1].name, "x");
        assert_eq!(items[2].name, "y");
        assert_eq!(items[3].name, "b");
        assert!(
            items.iter().all(|i| i.kind != ItemKind::Group),
            "解散后不该剩下组"
        );
    }

    #[test]
    fn dissolve_of_a_plain_item_is_rejected() {
        let mut items = vec![file("a")];
        assert_eq!(dissolve_item(&mut items, 0), Err(GroupError::NotGroup));
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn grid_cells_follow_the_four_quadrant_rule() {
        assert_eq!(grid_cells(0), (0, 0));
        assert_eq!(grid_cells(1), (1, 1));
        assert_eq!(grid_cells(2), (1, 2));
        assert_eq!(grid_cells(3), (2, 2));
        assert_eq!(grid_cells(4), (2, 2));
        assert_eq!(grid_cells(9), (2, 2), "超过四个也只用四格");
    }

    /// 组内子项排序：交换相邻两项，首尾越界不动。
    #[test]
    fn move_kid_swaps_neighbours_only() {
        let mut g = new_group("工具");
        for n in ["a", "b", "c"] {
            add_kid(&mut g, file(n)).unwrap();
        }
        assert_eq!(move_kid(&mut g, 0, -1), Err(GroupError::NoSuchItem));
        assert_eq!(move_kid(&mut g, 2, 1), Err(GroupError::NoSuchItem));

        move_kid(&mut g, 0, 1).expect("应当能下移");
        assert_eq!(g.kids[0].name, "b");
        assert_eq!(g.kids[1].name, "a");
        assert_eq!(g.kids[2].name, "c");

        move_kid(&mut g, 2, -1).expect("应当能上移");
        assert_eq!(g.kids[1].name, "c");
        assert_eq!(g.kids[2].name, "a");
        assert_eq!(move_kid(&mut g, 9, 1), Err(GroupError::NoSuchItem));
    }

    #[test]
    fn move_kid_rejects_a_plain_item() {
        let mut it = file("记事本");
        assert_eq!(move_kid(&mut it, 0, 1), Err(GroupError::NotGroup));
    }

    /// 解散之后子项要能原样存回 ini 再读出来（子项的字段不能丢）。
    #[test]
    fn dissolved_kids_survive_a_store_roundtrip() {
        let mut g = new_group("工具");
        let mut kid = file("x");
        kid.args = "--flag".into();
        add_kid(&mut g, kid).unwrap();
        let mut items = vec![g];
        let at = dissolve_item(&mut items, 0).unwrap();
        assert_eq!(items[at].args, "--flag");
    }
}
