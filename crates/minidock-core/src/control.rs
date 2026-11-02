//! 控制中心的磁贴模型。
//!
//! 只有"有哪些磁贴、按什么顺序排、每行几个"这几件事 —— 全部可单测。
//! 真正的系统调用在 `platform::sysact` / `platform::audio`，绘制在 `gpu::cc_win`。
//!
//! 对应上游 v1.6.0 的 `ShowCtrlCenter()`（minidock.c:6716-6735）：磁贴顺序是
//! 硬编码的，末尾两个（节能 / 热点）按配置开关追加。

/// 一个磁贴。
///
/// 下标即上游的 `u->id[k]`，取位顺序不能改 —— 上游用 `id` 去查名字表和图标表。
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
    /// 磁贴上的文字（上游 `nm[9]`）。
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

    /// 这个磁贴是否"当前是开着的"（决定底色用主题色还是灰）。
    ///
    /// 只有三个磁贴有开关语义：深色模式、静音、节能。其余都是"点一下跳转"
    /// 或者"点一下弹系统面板"，没有开/关状态。
    #[must_use]
    pub const fn is_switch(self) -> bool {
        matches!(self, Self::DarkMode | Self::Mute | Self::Energy)
    }
}

/// 磁贴每行几个（上游固定 3 列）。
pub const COLS: usize = 3;

/// 按配置与网卡状态排出磁贴顺序。
///
/// 顺序照抄上游：`[无线/有线] 蓝牙 深色 定位 静音 设置 [节能] [热点]`。
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

/// 磁贴占几行。上游把这里写死成 2（`2 * (th + 10)`），于是开关两个可选磁贴
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

    /// 行数按实际磁贴数算（修掉上游写死 2 行的问题）。
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
}
