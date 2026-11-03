//! 起动台的数据模型：应用、文件夹、格子顺序、隐藏列表。
//!
//! 与 UI 无关，于是"拖到图标上会合并成文件夹"这类行为能用单测覆盖 ——
//! 这是最容易出错、又最没法靠截图验证的部分。
//!
//! 持久化格式
//!
//! ```text
//! T\t<path>\t<name>     一格 = 一个应用
//! F\t<name>            一格 = 一个文件夹（后面跟着若干 f 行，直到 E）
//! f\t<path>\t<name>    文件夹里的应用
//! E                     文件夹结束
//! H\t<path>             从起动台隐藏
//! ```
//!
//! 兼容这个格式的好处是：用户升级过来，格子顺序和文件夹都还在。

/// 应用数量上限。
pub const MAX_APPS: usize = 700;
/// 文件夹数量上限。
pub const MAX_FOLDERS: usize = 60;
/// 格子数量上限。
pub const MAX_TOP: usize = 760;
/// 一个文件夹最多装几个。
pub const MAX_IN_FOLDER: usize = 24;

/// 扫描出来的一个候选应用（平台层产出、模型层消费）。
///
/// 放在 core 而不是 platform，是为了让 [`Launchpad::absorb`] 有一个
/// 两边都认识的具体类型 —— 否则平台层要依赖模型层、模型层又要依赖平台层。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppEntry {
    /// 显示名（通常是没有扩展名的文件名）。
    pub name: String,
    /// 完整路径。
    pub path: String,
}

/// 一个应用。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    /// 显示名。
    pub name: String,
    /// 路径（`.lnk` / exe / 网址 / 文件夹）。
    pub path: String,
}

/// 一个文件夹。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    /// 文件夹名。
    pub name: String,
    /// 里面的应用（[`Launchpad::apps`] 的下标）。
    pub apps: Vec<usize>,
}

/// 顶层的每一格。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Top {
    /// 直接是一个应用。
    App(usize),
    /// 是一个文件夹。
    Folder(usize),
}

/// 从文件夹里拿走一个应用之后怎么处理它。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TakeOut {
    /// 放回顶层（追加到末尾）。
    ToTop,
    /// 从起动台隐藏（`H` 行）。
    Hide,
    /// 彻底丢掉（只从文件夹里删掉，应用本身还留在表里备用）。
    Drop,
}

/// 起动台数据。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Launchpad {
    apps: Vec<App>,
    folders: Vec<Folder>,
    top: Vec<Top>,
    hidden: Vec<String>,
}

impl Launchpad {
    /// 空起动台。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn apps(&self) -> &[App] {
        &self.apps
    }

    #[must_use]
    pub fn folders(&self) -> &[Folder] {
        &self.folders
    }

    #[must_use]
    pub fn top(&self) -> &[Top] {
        &self.top
    }

    /// 顶层有多少格。
    #[must_use]
    pub fn len(&self) -> usize {
        self.top.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.top.is_empty()
    }

    /// 查重：路径或名字任一相同就算同一个应用。
    fn find_app(&self, path: &str, name: &str) -> Option<usize> {
        self.apps
            .iter()
            .position(|a| a.path.eq_ignore_ascii_case(path) || a.name.eq_ignore_ascii_case(name))
    }

    /// 加一个应用（已存在则复用）。
    fn ensure_app(&mut self, path: &str, name: &str) -> Option<usize> {
        if let Some(i) = self.find_app(path, name) {
            return Some(i);
        }
        if self.apps.len() >= MAX_APPS {
            return None;
        }
        self.apps.push(App {
            name: name.to_owned(),
            path: path.to_owned(),
        });
        Some(self.apps.len() - 1)
    }

    /// 是否在隐藏列表里。
    #[must_use]
    pub fn is_hidden(&self, path: &str) -> bool {
        self.hidden.iter().any(|h| h.eq_ignore_ascii_case(path))
    }

    /// 这个应用是否已经摆在台面上（顶层或某个文件夹里）。
    fn is_placed(&self, idx: usize) -> bool {
        self.top.contains(&Top::App(idx)) || self.folders.iter().any(|f| f.apps.contains(&idx))
    }

    /// 吸收一次扫描结果：把还没摆上台面的新应用追加到末尾。
    ///
    /// 三个跳过条件：
    /// 已隐藏、已经摆过、表里已满。
    pub fn absorb(&mut self, scanned: &[AppEntry]) {
        for e in scanned {
            if self.is_hidden(&e.path) {
                continue;
            }
            if let Some(i) = self.find_app(&e.path, &e.name) {
                if self.is_placed(i) {
                    continue;
                }
            }
            let Some(i) = self.ensure_app(&e.path, &e.name) else {
                break;
            };
            if self.top.len() >= MAX_TOP {
                break;
            }
            self.top.push(Top::App(i));
        }
    }

    /// 手动添加一个应用（拖文件进来 / 加网址）。
    ///
    /// 返回它的下标；已经在台面上时返回 `None`（避免重复格子）。
    pub fn add(&mut self, name: &str, path: &str) -> Option<usize> {
        if self.is_hidden(path) {
            self.hidden.retain(|h| !h.eq_ignore_ascii_case(path));
        }
        let i = self.ensure_app(path, name)?;
        if self.is_placed(i) || self.top.len() >= MAX_TOP {
            return None;
        }
        self.top.push(Top::App(i));
        Some(i)
    }

    /// 搜索过滤：返回命中的应用下标（大小写不敏感的子串匹配）。
    #[must_use]
    pub fn filter(&self, query: &str) -> Vec<usize> {
        if query.is_empty() {
            return Vec::new();
        }
        let q = query.to_lowercase();
        self.apps
            .iter()
            .enumerate()
            .filter(|(_, a)| !self.is_hidden(&a.path) && a.name.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect()
    }

    /// 顶层某一格显示什么名字（文件夹取文件夹名）。
    #[must_use]
    pub fn name_of(&self, t: Top) -> &str {
        match t {
            Top::App(i) => &self.apps[i].name,
            Top::Folder(i) => &self.folders[i].name,
        }
    }

    /// 文件夹里有几个应用。
    #[must_use]
    pub fn folder_len(&self, fi: usize) -> usize {
        self.folders[fi].apps.len()
    }

    /// 一页放 `per` 个时共有几页（至少 1 页）。
    #[must_use]
    pub fn pages(count: usize, per: usize) -> usize {
        if per == 0 {
            return 1;
        }
        count.div_ceil(per).max(1)
    }

    /// 把一个格子拖到另一个格子上。
    ///
    /// `on_icon` 表示松开时指针在**图标本体**上（而不是同一格里的空白处）。
    /// 是按"离格心多近"判的（`LpCellAt` 里的 `center`），这里由 UI 传进来 ——
    /// 模型层不该知道像素。
    ///
    /// 三种结果：
    /// - 落在**文件夹**图标上：塞进去（满了就什么都不做）；
    /// - 落在**应用**图标上、且拖的也是应用：两者合并成新文件夹；
    /// - 其余情况（落在空白 / 拖的是文件夹）：按"插到目标位置"重排。
    ///
    /// 返回是否真的变了（变了调用方要存盘）。
    pub fn drop_at(&mut self, src: usize, target: usize, on_icon: bool) -> bool {
        if src == target || src >= self.top.len() {
            return false;
        }
        let target = target.min(self.top.len() - 1);

        if on_icon {
            // 落进已有文件夹。
            if let (Top::App(ai), Top::Folder(fi)) = (self.top[src], self.top[target]) {
                if self.folders[fi].apps.len() < MAX_IN_FOLDER {
                    self.folders[fi].apps.push(ai);
                    self.top.remove(src);
                    return true;
                }
            }
            // 两个应用合成一个新文件夹。
            if let (Top::App(ai), Top::App(ti)) = (self.top[src], self.top[target]) {
                if self.folders.len() < MAX_FOLDERS {
                    self.folders.push(Folder {
                        name: "文件夹".to_owned(),
                        apps: vec![ti, ai],
                    });
                    let fi = self.folders.len() - 1;
                    self.top[target] = Top::Folder(fi);
                    self.top.remove(src);
                    return true;
                }
            }
        }

        // 重排：抽出来再插到目标位置。
        let t = self.top.remove(src);
        // 移除源之后目标的位置可能左移一格。
        let dst = if src < target { target - 1 } else { target };
        let dst = dst.min(self.top.len());
        self.top.insert(dst, t);
        true
    }

    /// 解散一个文件夹：里面的应用按原顺序铺回它所占的那一格。
    pub fn dissolve(&mut self, k: usize) -> bool {
        let Some(Top::Folder(fi)) = self.top.get(k).copied() else {
            return false;
        };
        let members = std::mem::take(&mut self.folders[fi].apps);
        self.top.remove(k);
        for (n, ai) in members.into_iter().enumerate() {
            self.top.insert(k + n, Top::App(ai));
        }
        true
    }

    /// 从文件夹里拿走一个应用。
    pub fn take_from_folder(&mut self, fi: usize, idx: usize, how: TakeOut) -> bool {
        let Some(fo) = self.folders.get(fi) else {
            return false;
        };
        if idx >= fo.apps.len() {
            return false;
        }
        let ai = self.folders[fi].apps.remove(idx);
        match how {
            TakeOut::ToTop => {
                if self.top.len() < MAX_TOP {
                    self.top.push(Top::App(ai));
                }
            }
            TakeOut::Hide => {
                let path = self.apps[ai].path.clone();
                if !self.is_hidden(&path) {
                    self.hidden.push(path);
                }
            }
            TakeOut::Drop => {}
        }
        self.cleanup_folders();
        true
    }

    /// 文件夹空了就消失，只剩一个就散开。
    pub fn cleanup_folders(&mut self) {
        let mut k = self.top.len();
        while k > 0 {
            k -= 1;
            let Top::Folder(fi) = self.top[k] else {
                continue;
            };
            match self.folders[fi].apps.len() {
                0 => {
                    self.top.remove(k);
                }
                1 => {
                    let only = self.folders[fi].apps[0];
                    self.top[k] = Top::App(only);
                }
                _ => {}
            }
        }
    }

    /// 重命名一个格子（应用或文件夹）。
    pub fn rename(&mut self, k: usize, name: &str) -> bool {
        if name.trim().is_empty() {
            return false;
        }
        match self.top.get(k).copied() {
            Some(Top::App(i)) => self.apps[i].name = name.to_owned(),
            Some(Top::Folder(i)) => self.folders[i].name = name.to_owned(),
            None => return false,
        }
        true
    }

    /// 从台面上移除一格（文件夹里的应用一起消失，但仍在 [`Self::apps`] 里）。
    pub fn remove_top(&mut self, k: usize) -> bool {
        if k >= self.top.len() {
            return false;
        }
        self.top.remove(k);
        true
    }

    // ---------------------------------------------------------------- 持久化

    /// 解析 `launchpad.dat`。格式见模块文档；坏行一律跳过，不报错。
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut lp = Self::new();
        let mut cur: Option<usize> = None;
        for line in text.lines() {
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                continue;
            }
            let mut it = line.split('\t');
            let tag = it.next().unwrap_or("");
            match tag {
                "T" => {
                    let (Some(path), Some(name)) = (it.next(), it.next()) else {
                        continue;
                    };
                    if lp.top.len() >= MAX_TOP {
                        continue;
                    }
                    if let Some(i) = lp.ensure_app(path, name) {
                        lp.top.push(Top::App(i));
                    }
                }
                "F" => {
                    let Some(name) = it.next() else { continue };
                    if lp.folders.len() >= MAX_FOLDERS || lp.top.len() >= MAX_TOP {
                        cur = None;
                        continue;
                    }
                    lp.folders.push(Folder {
                        name: name.to_owned(),
                        apps: Vec::new(),
                    });
                    cur = Some(lp.folders.len() - 1);
                    lp.top.push(Top::Folder(lp.folders.len() - 1));
                }
                "f" => {
                    let (Some(path), Some(name)) = (it.next(), it.next()) else {
                        continue;
                    };
                    let Some(fi) = cur else { continue };
                    if lp.folders[fi].apps.len() >= MAX_IN_FOLDER {
                        continue;
                    }
                    if let Some(i) = lp.ensure_app(path, name) {
                        lp.folders[fi].apps.push(i);
                    }
                }
                "E" => cur = None,
                "H" => {
                    if let Some(path) = it.next() {
                        lp.hidden.push(path.to_owned());
                    }
                }
                _ => {}
            }
        }
        lp
    }

    /// 序列化成 `launchpad.dat`。
    #[must_use]
    pub fn serialize(&self) -> String {
        use std::fmt::Write as _;
        let mut s = String::new();
        for t in &self.top {
            match *t {
                Top::App(i) => {
                    let a = &self.apps[i];
                    let _ = writeln!(s, "T\t{}\t{}", a.path, a.name);
                }
                Top::Folder(fi) => {
                    let f = &self.folders[fi];
                    let _ = writeln!(s, "F\t{}", f.name);
                    for &ai in &f.apps {
                        let a = &self.apps[ai];
                        let _ = writeln!(s, "f\t{}\t{}", a.path, a.name);
                    }
                    let _ = writeln!(s, "E");
                }
            }
        }
        for h in &self.hidden {
            let _ = writeln!(s, "H\t{}", h);
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> AppEntry {
        AppEntry {
            name: name.to_owned(),
            path: format!(r"C:\Start\{name}.lnk"),
        }
    }

    fn with(names: &[&str]) -> Launchpad {
        let mut lp = Launchpad::new();
        let scanned: Vec<AppEntry> = names.iter().map(|n| entry(n)).collect();
        lp.absorb(&scanned);
        lp
    }

    /// 存 → 读 → 存 必须完全一致（含文件夹与隐藏项）。
    #[test]
    fn dat_roundtrip_keeps_order_and_folders() {
        let mut lp = with(&["A", "B", "C", "D"]);
        // 把 B 拖到 A 上 → 合成文件夹。
        assert!(lp.drop_at(1, 0, true));
        // 隐藏 D。
        lp.hidden.push(r"C:\Start\D.lnk".to_owned());

        let dat = lp.serialize();
        let back = Launchpad::parse(&dat);
        assert_eq!(back.serialize(), dat, "存读之后内容应当完全一致");
        assert_eq!(back.top(), lp.top());
        assert_eq!(back.folders().len(), 1);
        assert_eq!(back.folder_len(0), 2);
    }

    /// 解析写的文件：T/F/f/E/H 五种行。
    #[test]
    fn parses_upstream_file() {
        let dat = "T\tC:\\A.lnk\tA\nF\t开发\nf\tC:\\B.lnk\tB\nf\tC:\\C.lnk\tC\nE\nH\tC:\\D.lnk\n";
        let lp = Launchpad::parse(dat);
        assert_eq!(lp.len(), 2);
        assert_eq!(lp.name_of(lp.top()[0]), "A");
        assert_eq!(lp.name_of(lp.top()[1]), "开发");
        assert_eq!(lp.folder_len(0), 2);
        assert!(lp.is_hidden(r"C:\D.lnk"));
    }

    /// 吸收扫描结果：已经摆上台面的不重复添加。
    #[test]
    fn absorb_skips_placed_and_hidden() {
        let mut lp = with(&["A", "B"]);
        lp.hidden.push(r"C:\Start\C.lnk".to_owned());
        lp.absorb(&[entry("A"), entry("C"), entry("D")]);
        // C 被隐藏，A 已摆过，只有 D 是新的。
        assert_eq!(lp.len(), 3);
        assert_eq!(lp.name_of(lp.top()[2]), "D");
    }

    /// 在文件夹里的应用也算"已摆过"，不能再冒到顶层。
    #[test]
    fn absorb_skips_apps_inside_folders() {
        let mut lp = with(&["A", "B"]);
        assert!(lp.drop_at(1, 0, true));
        lp.absorb(&[entry("A"), entry("B")]);
        assert_eq!(lp.len(), 1, "文件夹里的 A/B 不该再出现在顶层");
    }

    /// 拖到文件夹上：塞进去。
    #[test]
    fn drop_into_existing_folder() {
        let mut lp = with(&["A", "B", "C"]);
        assert!(lp.drop_at(1, 0, true)); // A+B 成文件夹，占第 0 格
        assert_eq!(lp.len(), 2);
        assert!(lp.drop_at(1, 0, true)); // C 塞进文件夹
        assert_eq!(lp.len(), 1);
        assert_eq!(lp.folder_len(0), 3);
    }

    /// 拖到空格位置：重排，不合并。
    #[test]
    fn drop_reorders_around_the_target() {
        let mut lp = with(&["A", "B", "C"]);
        // 把 C（第 2 格）拖到第 0 格。
        assert!(lp.drop_at(2, 0, false));
        let names: Vec<&str> = lp.top().iter().map(|t| lp.name_of(*t)).collect();
        assert_eq!(names, ["C", "A", "B"]);
    }

    /// 文件夹空了消失、只剩一个散开。
    #[test]
    fn folders_dissolve_when_they_shrink() {
        let mut lp = with(&["A", "B", "C"]);
        assert!(lp.drop_at(1, 0, true)); // 文件夹(A,B) + C
        assert_eq!(lp.len(), 2);
        // 从文件夹里拿走 B 且丢弃 → 只剩 A，文件夹散开成一格应用。
        assert!(lp.take_from_folder(0, 1, TakeOut::Drop));
        assert_eq!(lp.len(), 2, "散开后 A 回到顶层");
        assert!(matches!(lp.top()[0], Top::App(_)));

        // 空文件夹只在"从文件里读到"或"成员被全部清空"时出现：
        // 一旦只剩一个，上面的逻辑就把它散开了。这里直接构造一个空的。
        let mut lp2 = Launchpad::parse("F\t空\nE\nT\tC:\\A.lnk\tA\n");
        assert_eq!(lp2.len(), 2);
        lp2.cleanup_folders();
        assert_eq!(lp2.len(), 1, "空文件夹应当整格消失");
        assert!(matches!(lp2.top()[0], Top::App(_)));
    }

    /// 从文件夹移出到顶层：追加在末尾。
    #[test]
    fn take_out_to_top_appends() {
        let mut lp = with(&["A", "B", "C"]);
        assert!(lp.drop_at(1, 0, true));
        assert!(lp.take_from_folder(0, 0, TakeOut::ToTop));
        let names: Vec<&str> = lp.top().iter().map(|t| lp.name_of(*t)).collect();
        assert_eq!(names.last().copied(), Some("A"));
    }

    /// 从文件夹隐藏：进隐藏列表，且以后扫描不再出现。
    #[test]
    fn take_out_hide_feeds_the_hidden_list() {
        let mut lp = with(&["A", "B", "C"]);
        assert!(lp.drop_at(1, 0, true));
        assert!(lp.take_from_folder(0, 1, TakeOut::Hide));
        assert!(lp.is_hidden(r"C:\Start\B.lnk"));
        lp.absorb(&[entry("B")]);
        assert_eq!(lp.len(), 2, "隐藏之后不该再被扫描补回来");
    }

    /// 解散文件夹：成员按原顺序铺回原位。
    #[test]
    fn dissolve_puts_members_back_in_order() {
        let mut lp = with(&["A", "B", "C"]);
        assert!(lp.drop_at(1, 0, true)); // 文件夹(A,B) + C
        assert!(lp.dissolve(0));
        let names: Vec<&str> = lp.top().iter().map(|t| lp.name_of(*t)).collect();
        assert_eq!(names, ["A", "B", "C"]);
    }

    /// 搜索：大小写不敏感的子串，隐藏项不出现。
    #[test]
    fn filter_is_case_insensitive_and_skips_hidden() {
        let mut lp = with(&["Visual Studio", "WeChat", "Word"]);
        lp.hidden.push(r"C:\Start\Word.lnk".to_owned());
        let hit = lp.filter("w");
        let names: Vec<&str> = hit.iter().map(|i| lp.apps()[*i].name.as_str()).collect();
        assert_eq!(names, ["WeChat"], "Word 被隐藏，Visual Studio 不含 w");
        assert_eq!(lp.filter("").len(), 0, "空查询不算搜索");
    }

    /// 分页：向上取整，至少一页。
    #[test]
    fn page_count_rounds_up() {
        assert_eq!(Launchpad::pages(0, 28), 1);
        assert_eq!(Launchpad::pages(1, 28), 1);
        assert_eq!(Launchpad::pages(28, 28), 1);
        assert_eq!(Launchpad::pages(29, 28), 2);
        assert_eq!(Launchpad::pages(56, 28), 2);
    }

    /// 重命名：应用与文件夹都支持，空名字被拒绝。
    #[test]
    fn rename_works_for_both_kinds() {
        let mut lp = with(&["A", "B"]);
        assert!(lp.rename(0, "记事本"));
        assert_eq!(lp.name_of(lp.top()[0]), "记事本");
        assert!(!lp.rename(0, "   "));
        assert!(lp.drop_at(1, 0, true));
        assert!(lp.rename(0, "工具"));
        assert_eq!(lp.name_of(lp.top()[0]), "工具");
    }

    /// 手动添加：重名的不会再加一格；隐藏过的加回来会解除隐藏。
    #[test]
    fn add_is_idempotent_and_unhides() {
        let mut lp = with(&["A"]);
        assert_eq!(lp.add("A", r"C:\Start\A.lnk"), None, "同一路径不该重复");
        lp.hidden.push(r"C:\New\X.exe".to_owned());
        assert!(lp.add("X", r"C:\New\X.exe").is_some());
        assert!(!lp.is_hidden(r"C:\New\X.exe"));
    }

    /// 拖到自己身上什么也不做。
    #[test]
    fn dropping_on_itself_is_a_noop() {
        let mut lp = with(&["A", "B"]);
        assert!(!lp.drop_at(1, 1, true));
        assert_eq!(lp.len(), 2);
    }
}
