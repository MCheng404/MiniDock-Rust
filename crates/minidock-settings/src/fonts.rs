//! 中文字体装载。
//!
//! egui 的 `default_fonts` 只带 Ubuntu-Light 与 emoji，**没有 CJK**：
//! 不装字体的话整个界面是豆腐块，而 a11y 树里的文字仍然正确 ——
//! 也就是说单元测试会全绿，人眼看全是方块。这类"测试通过但产品报废"
//! 的坑必须在第一层就堵住，所以字体装载是 settings crate 的入口步骤。
//!
//! 为什么运行时读系统字体而不是打包子集：
//! - `msyh.ttc` 是集合字体（TTC），epaint 的 `FontData.index` 支持选 face；
//! - 打包 Noto Sans SC 子集要 +1~3 MB，而系统字体本来就在；
//! - 代价是"换机器可能没有这个字体"，所以按优先级逐个尝试并返回是否成功。

use std::sync::Arc;

/// 按优先级尝试的系统字体。第一个能读到的就装。
const CANDIDATES: &[(&str, u32)] = &[
    (r"C:\Windows\Fonts\msyh.ttc", 0),   // 微软雅黑
    (r"C:\Windows\Fonts\msyhbd.ttc", 0), // 微软雅黑 Bold
    (r"C:\Windows\Fonts\Deng.ttf", 0),   // 等线
    (r"C:\Windows\Fonts\simhei.ttf", 0), // 黑体
    (r"C:\Windows\Fonts\simsun.ttc", 0), // 宋体
];

/// 装载 CJK 字体。返回是否成功（失败时界面仍可用，只是中文会变方块）。
pub fn install_cjk(ctx: &egui::Context) -> bool {
    let Some((path, index)) = CANDIDATES
        .iter()
        .find(|(p, _)| std::path::Path::new(p).is_file())
    else {
        return false;
    };
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };

    let mut data = egui::FontData::from_owned(bytes);
    data.index = *index; // TTC 里选第几个 face

    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert("system-cjk".to_owned(), Arc::new(data));
    // 插到族表最前：中文优先用系统字体，拉丁字形仍会回落到后面自带的字体。
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, "system-cjk".to_owned());
    }
    ctx.set_fonts(fonts);
    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn font_file_is_discoverable_on_this_machine() {
        // 这条不是断言"一定有字体"，而是给出诊断：开发机没有中文字体时
        // 界面会变方块，但测试仍然全绿 —— 所以单独打印出来。
        let found = super::CANDIDATES
            .iter()
            .any(|(p, _)| std::path::Path::new(p).is_file());
        eprintln!("CJK 字体可用: {found}");
    }
}
