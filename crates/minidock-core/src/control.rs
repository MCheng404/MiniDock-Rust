//! 控制中心的磁贴模型。
//!
//! 只有"有哪些磁贴、按什么顺序排、每行几个"这几件事 —— 全部可单测。
//! 真正的系统调用在 `platform::sysact` / `platform::audio`，绘制在 `gpu::cc_win`。
//!
//! 默认顺序是固定的，末尾两个（节能 / 热点）按配置开关追加；用户在面板上
//! 拖动过之后，顺序存在 `[widgets] Order`（见 [`parse_order`] / [`order_text`]）。
//! 存的是**磁贴 id**而不是位置：开关一个磁贴不该把其它磁贴的顺序挤乱。

/// 一个磁贴。
///
/// 下标即`u->id[k]`，取位顺序不能改 —— 用 `id` 去查名字表和图标表。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Tile {
    /// 无线网络。
    Wifi,
    /// 蓝牙。
    Bluetooth,
    /// 深色模式。
    DarkMode,
    /// 定位。
    Location,
    /// 静音。
    Mute,
    /// 打开 Windows 设置。
    Settings,
    /// 节能模式。
    Energy,
    /// 移动热点。
    Hotspot,
    /// 有线网络（检测到有线网卡时替换掉 [`Tile::Wifi`]）。
    Ethernet,
}

impl Tile {
    /// 磁贴上的文字。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Wifi => "无线网络",
            Self::Bluetooth => "蓝牙",
            Self::DarkMode => "深色模式",
            Self::Location => "定位",
            Self::Mute => "静音",
            Self::Settings => "设置",
            Self::Energy => "节能模式",
            Self::Hotspot => "移动热点",
            Self::Ethernet => "有线网络",
        }
    }

    /// 磁贴的稳定 id。
    ///
    /// 数值与上游 ini 里 `[Cc] Order` 的写法一致（0..=7，有线网卡用 8），
    /// 这样从 C 版迁移过来的配置不用翻译。**不能跟着枚举顺序改** ——
    /// 存进 ini 的是这些数字。
    #[must_use]
    pub const fn id(self) -> u8 {
        match self {
            Self::Wifi => 0,
            Self::Bluetooth => 1,
            Self::DarkMode => 2,
            Self::Location => 3,
            Self::Mute => 4,
            Self::Settings => 5,
            Self::Energy => 6,
            Self::Hotspot => 7,
            Self::Ethernet => 8,
        }
    }

    /// 从 id 反查。未知 id 返回 `None`（老版本写下的、或手改错的）。
    #[must_use]
    pub const fn from_id(id: u8) -> Option<Self> {
        match id {
            0 => Some(Self::Wifi),
            1 => Some(Self::Bluetooth),
            2 => Some(Self::DarkMode),
            3 => Some(Self::Location),
            4 => Some(Self::Mute),
            5 => Some(Self::Settings),
            6 => Some(Self::Energy),
            7 => Some(Self::Hotspot),
            8 => Some(Self::Ethernet),
            _ => None,
        }
    }

    /// 是不是"网络"那一格。
    ///
    /// 有线与无线**共用同一格**：顺序里存的是这一格的位置，具体显示哪个
    /// 由当前网卡状态决定。否则插上网线时那一格会跳到别处去。
    #[must_use]
    pub const fn is_net(self) -> bool {
        matches!(self, Self::Wifi | Self::Ethernet)
    }

    /// 这个磁贴是否"当前是开着的"（决定底色用主题色还是灰）。
    ///
    /// 只有三个磁贴有开关语义：深色模式、静音、节能。其余都是"点一下跳转"
    /// 或者"点一下弹系统面板"，没有开/关状态。
    #[must_use]
    pub const fn is_switch(self) -> bool {
        matches!(self, Self::DarkMode | Self::Mute | Self::Energy)
    }
}

/// 磁贴每行几个。
pub const COLS: usize = 3;

/// 按配置与网卡状态排出磁贴顺序。
///
/// 顺序沿用：`[无线/有线] 蓝牙 深色 定位 静音 设置 [节能] [热点]`。
/// 末尾两个是可选的，`CcEnergy` / `CcHotspot` 关了就不出现。
#[must_use]
pub fn tiles(energy: bool, hotspot: bool, wired: bool) -> Vec<Tile> {
    let mut v = vec![
        if wired { Tile::Ethernet } else { Tile::Wifi },
        Tile::Bluetooth,
        Tile::DarkMode,
        Tile::Location,
        Tile::Mute,
        Tile::Settings,
    ];
    if energy {
        v.push(Tile::Energy);
    }
    if hotspot {
        v.push(Tile::Hotspot);
    }
    v
}

/// 解析 `[widgets] Order`：逗号分隔的 id。
///
/// 容忍空格、未知 id、重复项 —— 这个键用户会手改（想调个顺序），
/// 一个写错就让整个面板乱掉不值得。空串解析成空表（= 用默认顺序）。
#[must_use]
pub fn parse_order(text: &str) -> Vec<Tile> {
    let mut out: Vec<Tile> = Vec::new();
    for part in text.split(',') {
        let Ok(id) = part.trim().parse::<u8>() else {
            continue;
        };
        let Some(t) = Tile::from_id(id) else {
            continue;
        };
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// 存回 ini 的写法。与 [`parse_order`] 互逆。
#[must_use]
pub fn order_text(tiles: &[Tile]) -> String {
    tiles
        .iter()
        .map(|t| t.id().to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// 按保存的顺序 + 当前开关状态排出磁贴。
///
/// 三条规则：
///
/// 1. **被关掉的磁贴不出现**（`CcEnergy` / `CcHotspot`）。它的位置留在
///    `order` 里，重新打开时还回到原处。
/// 2. **网络那一格按当前网卡决定**是无线还是有线，位置跟着 `order` 走。
/// 3. **`order` 里没提到的磁贴追加在末尾**。新版本加了磁贴、或者用户手改
///    配置只写了一部分，都靠这条兜住 —— 少一个磁贴比多一个更难查。
#[must_use]
pub fn tiles_ordered(order: &[Tile], energy: bool, hotspot: bool, wired: bool) -> Vec<Tile> {
    let net = if wired { Tile::Ethernet } else { Tile::Wifi };
    let enabled = |t: Tile| match t {
        Tile::Energy => energy,
        Tile::Hotspot => hotspot,
        _ => true,
    };
    let mut out: Vec<Tile> = Vec::new();
    for t in order {
        let t = if t.is_net() { net } else { *t };
        if enabled(t) && !out.contains(&t) {
            out.push(t);
        }
    }
    // 追加 order 里没提到的（含从未拖过的全新磁贴）。
    for t in tiles(energy, hotspot, wired) {
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// 磁贴占几行。把这里写死成 2（`2 * (th + 10)`），于是开关两个可选磁贴
/// （默认就是开的）时第三行会被音量滑块压住 —— 顺手修掉。
#[must_use]
pub const fn rows(n: usize) -> usize {
    n.div_ceil(COLS)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 默认配置（节能 + 热点都开）是 8 个磁贴。
    #[test]
    fn default_tiles_are_eight() {
        let t = tiles(true, true, false);
        assert_eq!(t.len(), 8);
        assert_eq!(t[0], Tile::Wifi);
        assert_eq!(t[5], Tile::Settings);
        assert_eq!(t[6], Tile::Energy);
        assert_eq!(t[7], Tile::Hotspot);
    }

    /// 关掉两个可选磁贴就只剩 6 个。
    #[test]
    fn optional_tiles_can_be_disabled() {
        let t = tiles(false, false, false);
        assert_eq!(t.len(), 6);
        assert!(!t.contains(&Tile::Energy));
        assert!(!t.contains(&Tile::Hotspot));
    }

    /// 有线网卡时第一个磁贴换掉。
    #[test]
    fn wired_replaces_the_wifi_tile() {
        let t = tiles(false, false, true);
        assert_eq!(t[0], Tile::Ethernet);
        assert!(!t.contains(&Tile::Wifi));
    }

    /// 行数按实际磁贴数算。
    #[test]
    fn rows_follow_the_tile_count() {
        assert_eq!(rows(6), 2);
        assert_eq!(rows(7), 3);
        assert_eq!(rows(8), 3);
        assert_eq!(rows(9), 3);
    }

    /// 只有三个磁贴有开/关语义。
    #[test]
    fn only_three_tiles_are_switches() {
        let switches: Vec<&str> = [
            Tile::Wifi,
            Tile::Bluetooth,
            Tile::DarkMode,
            Tile::Location,
            Tile::Mute,
            Tile::Settings,
            Tile::Energy,
            Tile::Hotspot,
            Tile::Ethernet,
        ]
        .into_iter()
        .filter(|t| t.is_switch())
        .map(Tile::label)
        .collect();
        assert_eq!(switches, ["深色模式", "静音", "节能模式"]);
    }

    /// 每个磁贴都有文案，不能漏。
    #[test]
    fn every_tile_has_a_label() {
        for t in [
            Tile::Wifi,
            Tile::Bluetooth,
            Tile::DarkMode,
            Tile::Location,
            Tile::Mute,
            Tile::Settings,
            Tile::Energy,
            Tile::Hotspot,
            Tile::Ethernet,
        ] {
            assert!(!t.label().is_empty());
        }
    }

    // ---- 顺序：id / 解析 / 排列 ----

    /// id 是 ini 里的数字，必须与枚举顺序解耦且互不重复。
    #[test]
    fn tile_ids_are_unique_and_roundtrip() {
        let all = [
            Tile::Wifi,
            Tile::Bluetooth,
            Tile::DarkMode,
            Tile::Location,
            Tile::Mute,
            Tile::Settings,
            Tile::Energy,
            Tile::Hotspot,
            Tile::Ethernet,
        ];
        let mut seen = Vec::new();
        for t in all {
            assert_eq!(Tile::from_id(t.id()), Some(t), "{t:?} 的 id 对不上");
            assert!(!seen.contains(&t.id()), "id {} 重复", t.id());
            seen.push(t.id());
        }
        assert_eq!(Tile::from_id(9), None);
    }

    /// 手改的配置里出现空格、未知 id、重复项都不能把整个顺序带崩。
    #[test]
    fn parse_order_skips_junk() {
        let got = parse_order(" 1, 9, x, 2,2 ,,5");
        assert_eq!(
            got,
            [Tile::Bluetooth, Tile::DarkMode, Tile::Settings],
            "未知 id / 非数字 / 重复项都跳过"
        );
        assert!(parse_order("").is_empty());
        assert!(parse_order("   ").is_empty());
        assert!(parse_order("abc").is_empty());
    }

    /// 存回去再读回来要一模一样（ini 是用户会手改的东西）。
    #[test]
    fn order_text_roundtrips() {
        let order = [Tile::Settings, Tile::Wifi, Tile::Mute, Tile::Energy];
        assert_eq!(order_text(&order), "5,0,4,6");
        assert_eq!(parse_order(&order_text(&order)), order);
    }

    /// 保存的顺序决定磁贴位置。
    #[test]
    fn tiles_ordered_follows_saved_order() {
        let order = parse_order("5,0,4,1,2,3,6,7");
        let got = tiles_ordered(&order, true, true, false);
        assert_eq!(
            got,
            [
                Tile::Settings,
                Tile::Wifi,
                Tile::Mute,
                Tile::Bluetooth,
                Tile::DarkMode,
                Tile::Location,
                Tile::Energy,
                Tile::Hotspot,
            ]
        );
    }

    /// 关掉的磁贴不出现，但它的位置留着 —— 重新打开时回到原处，
    /// 而不是被踢到末尾。
    #[test]
    fn disabled_tile_keeps_its_slot() {
        let order = parse_order("0,6,1,2,3,4,5,7");
        let off = tiles_ordered(&order, false, false, false);
        assert!(!off.contains(&Tile::Energy));
        assert!(!off.contains(&Tile::Hotspot));
        let on = tiles_ordered(&order, true, true, false);
        assert_eq!(on[1], Tile::Energy, "节能回到第 2 格");
        assert_eq!(on[7], Tile::Hotspot);
    }

    /// 有线 / 无线共用一格：换网卡不改变位置，也不会两个都在。
    #[test]
    fn net_slot_switches_in_place() {
        let order = parse_order("1,0,2,3,4,5");
        let wifi = tiles_ordered(&order, false, false, false);
        assert_eq!(wifi[1], Tile::Wifi);
        let wired = tiles_ordered(&order, false, false, true);
        assert_eq!(wired[1], Tile::Ethernet);
        assert!(!wired.contains(&Tile::Wifi));
        assert_eq!(wired.len(), wifi.len(), "只是换了一个，不是多一个");
    }

    /// 顺序里没提到的磁贴追加在末尾：新版本加了磁贴、或用户手改只写了一部分。
    #[test]
    fn unlisted_tiles_are_appended() {
        let got = tiles_ordered(&[Tile::Settings], false, false, false);
        assert_eq!(got[0], Tile::Settings);
        assert_eq!(got.len(), 6, "其余五个（含网络）都要补上");
        assert_eq!(got[1], Tile::Wifi, "补上的按默认顺序");
    }

    /// 空顺序 = 默认顺序（没拖过的用户）。
    #[test]
    fn empty_order_equals_default() {
        assert_eq!(
            tiles_ordered(&[], true, true, false),
            tiles(true, true, false)
        );
    }
}
