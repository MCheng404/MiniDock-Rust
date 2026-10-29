//! 生成 `CONFIG.md` 里的 `[Settings]` 键表。
//!
//! 为什么要用代码生成：这张表有 76 行，手写必然和实现漂移。
//! 上游每加一个键，这里重新跑一次就能拿到最新的表。
//!
//! 用法：
//!
//! ```text
//! cargo run -q -p minidock-core --example config_table > /tmp/table.md
//! ```
//!
//! 说明文字（`含义` 列）来自上游设置界面的行标题 —— 那是权威的语义来源。

use minidock_core::config::{Config, Key};

/// 每个键的含义。文字取自上游设置行的标题。
fn describe(key: Key) -> &'static str {
    match key {
        Key::IconSize => "图标边长（px，随 DPI 存物理像素）",
        Key::ZoomSize => "悬停放大后的边长",
        Key::Spacing => "图标间距",
        Key::Margin => "距屏幕边缘",
        Key::Padding => "栏内边距（决定栏高 = S + 2·pad + 4）",
        Key::Radius => "栏圆角",
        Key::Opacity => "栏体不透明度（%）",
        Key::Blur => "磨砂模糊半径",
        Key::Zoom => "悬停放大",
        Key::Theme => "Dock 主题：0 深 / 1 浅 / 2 跟随",
        Key::Dots => "运行指示",
        Key::Labels => "悬停名称标签",
        Key::Bounce => "启动弹跳",
        Key::WebAppMode => "网页用 `--app` 窗口打开",
        Key::Border => "栏体描边",
        Key::BgMode => "0 纯色 / 1 渐变 / 2 磨砂",
        Key::Mode => "0 常驻 / 1 自动隐藏 / 2 智能隐藏",
        Key::Reserve => "AppBar 预留桌面空间",
        Key::Edge => "0 底 / 1 顶 / 2 左 / 3 右",
        Key::Monitor => "显示器索引（越界回落到 0）",
        Key::Preview => "悬停窗口预览",
        Key::ShowRunning => "显示未固定的运行中应用",
        Key::Lock => "锁定布局",
        Key::Hotkey => "全局快捷键 Ctrl+Alt+D",
        Key::DotStyle => "0 横条 / 1 圆点",
        Key::PlateRadius => "底盘圆角（%，相对图标边长）",
        Key::BgColor1 => "自定义颜色 1，`-1` = 未启用；值为 COLORREF",
        Key::BgColor2 => "渐变终点",
        Key::AccentTone => "跟随系统强调色",
        Key::Reflect => "图标倒影（仅底部停靠）",
        Key::Clock => "时间与日期卡片",
        Key::FolderFan => "文件夹弧形堆叠",
        Key::MinAnim => "最小化飞入动画",
        Key::Notes => "速记工具",
        Key::UiTheme => "**设置窗口**主题：0 浅 / 1 深 / 2 跟随",

        // ---- 上游 v1.4.0 新增 ----
        Key::PLATE_MODE => "底盘类型：0 无 / 1 纯色 / 2 半透明",
        Key::PLATE_THEME => "底盘配色：0 浅 / 1 深 / 2 跟随 Dock 主题",
        Key::PLATE_OPACITY => "底盘不透明度（%）",
        Key::BG_COLOR3 => "渐变中间色（可选）",
        Key::MENU_THEME => "右键菜单主题：0 浅 / 1 深 / 2 跟随",
        Key::BORDER_COLOR => "边框颜色，`-1` = 跟随主题",
        Key::BORDER_WIDTH => "边框粗细（px）",
        Key::SEP_COLOR => "分割线颜色",
        Key::DOT_COLOR => "运行指示点颜色",
        Key::GRAD_ANGLE => "渐变方向（°）",
        Key::GRAD_A1 => "颜色 1 不透明度（%）",
        Key::GRAD_A2 => "颜色 2 不透明度（%）",
        Key::GRAD_A3 => "颜色 3 不透明度（%）",
        Key::CLOCK_SHOW => "0 时间+日期 / 1 仅时间 / 2 仅日期",
        Key::CLOCK_COL_LIGHT => "浅色模式下时钟字体颜色",
        Key::CLOCK_COL_DARK => "深色模式下时钟字体颜色",
        Key::REFL_SIZE => "倒影大小（%，相对图标边长）",
        Key::REFL_GAP => "倒影与 Dock 的距离（px）",
        Key::REFL_OPACITY => "倒影不透明度（%）",
        Key::FAN_ARC => "文件夹弧形展开的弧度（0 = 垂直展开）",
        Key::FAN_GAP => "弧形展开的纵向间隔（px）",
        Key::FAN_CUT => "拖出文件时：0 复制 / 1 剪切（移动）",
        Key::GROUP_FROST => "应用组毛玻璃背景",
        Key::GROUP_POS => "应用组打开位置：0 Dock 上方 / 1 屏幕中央",
        Key::GUARDIAN => "守护进程：被清理工具结束后自动重开",
        Key::CLICK_TOGGLE => "点击运行中的应用 = 最小化 / 恢复",
        Key::SEP_GAP => "分割线间距（px，默认随 DPI 缩放）",
        Key::SEP_WIDTH => "分割线宽度（px）",
        Key::CLOCK_SEC => "时间显示秒数",
        Key::CLOCK_WEEK => "日期显示周几",
        Key::DATE_FMT => "日期格式：0 中文 / 1 斜杠 / 2 横杠",
        Key::CLOCK_ORDER => "时间与日期的排列顺序",
        Key::CLOCK_BOLD_T => "时间加粗",
        Key::CLOCK_BOLD_D => "日期加粗",
        Key::BROWSER => "网页使用的浏览器：0 自动 / 1 Edge / 2 Chrome / 3 Firefox / 4 系统默认",
        Key::GRP_OPACITY => "应用组背景不透明度（%）",
        Key::GRP_BG => "应用组背景颜色",
        Key::MIN_STYLE => "最小化动画风格：0 压缩 / 1 弹性",
        Key::MN_AUTO => "右键菜单显示开机自启开关",
        Key::MN_NOTES => "右键菜单显示速记开关",
        Key::MN_ADD => "空白处右键显示新建项",
        Key::Numeric(_) => "",
    }
}

fn main() {
    let cfg = Config::defaults(96);
    println!("| 键 | 默认值(96dpi) | 硬夹紧 | 含义 |");
    println!("|---|---|---|---|");
    for key in Key::ALL.iter().copied() {
        let name = key.name();
        let default = cfg.get_i32(key);
        let (lo, hi) = key.range();
        let range = if key.is_bool() {
            "0/1".to_owned()
        } else if lo < 0 {
            format!("-1 或 0–0x{hi:06X}")
        } else {
            format!("{lo}–{hi}")
        };
        println!("| `{name}` | {default} | {range} | {} |", describe(key));
    }
}
