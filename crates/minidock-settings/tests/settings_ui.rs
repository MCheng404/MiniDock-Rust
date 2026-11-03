//! 用 kittest 无头驱动设置界面。
//!
//! 这一组测试证明的是"可测试性"这条路线成立：没有窗口、没有 GPU、
//! 没有显示器，却能真的点中控件并断言配置变化。
//!
//! 注意：kittest 通过 **AccessKit 树**查询控件，所以断言的是
//! "这个控件对人可见且可操作"，而不是"这段代码被调用了"。

use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use minidock_core::config::{BgMode, Config};
use minidock_settings::{settings_ui, Command, Page, SettingsState};

#[derive(Debug)]
struct St {
    cfg: Config,
    ui: SettingsState,
    commands: Vec<Command>,
}

fn new_harness() -> Harness<'static, St> {
    harness(false)
}

/// `shell = true` 时模拟 `gpu::settings_win::shell`：先画标题栏，再把
/// **剩下的**区域交给 `settings_ui`。
///
/// 这个变体存在的理由：`settings_ui` 曾经用 `ui.max_rect()` 取起点，而
/// `max_rect` 不随已用内容收缩 —— 两列会直接盖到标题上。测试里不模拟
/// 外壳就永远发现不了。
fn harness(shell: bool) -> Harness<'static, St> {
    let state = St {
        cfg: Config::defaults(96),
        ui: SettingsState {
            config_dir: r"C:\Users\test\AppData\Roaming\MiniDock".to_owned(),
            ..SettingsState::default()
        },
        commands: Vec::new(),
    };
    // 窗口给得足够大，避免 ScrollArea 把控件裁掉导致查不到节点。
    let size = if shell {
        [900.0, 720.0]
    } else {
        [900.0, 1200.0]
    };
    let builder = Harness::builder()
        .with_size(size)
        // 必须在第一帧之前装字体：egui 0.37 在缺字形时**直接 panic**
        // （MissingGlyphPolicy::Panic），不是画成豆腐块。
        .with_context_setup(|ctx| {
            assert!(
                minidock_settings::fonts::install_cjk(ctx),
                "本机找不到中文字体，界面无法渲染"
            );
        });

    if shell {
        builder.build_ui_state(
            |ui, st: &mut St| {
                ui.heading("MiniDock");
                ui.separator();
                run(ui, st);
            },
            state,
        )
    } else {
        builder.build_ui_state(|ui, st: &mut St| run(ui, st), state)
    }
}

fn run(ui: &mut egui::Ui, st: &mut St) {
    let outcome = settings_ui(ui, &mut st.cfg, &mut st.ui);
    if let Some(cmd) = outcome.command {
        st.commands.push(cmd);
    }
}

#[test]
fn 点击开关会写入配置() {
    let mut h = new_harness();
    assert!(!h.state().cfg.lock, "默认不锁定");

    h.get_by_label("锁定 Dock 布局").click();
    h.run();

    assert!(h.state().cfg.lock, "点击后应当锁定");
}

#[test]
fn 页面切换后能看到该页控件() {
    let mut h = new_harness();
    // 常规页没有 "图标大小"，外观页才有 —— 用它验证切换真的发生了。
    assert_eq!(h.query_all_by_label("图标大小").count(), 0);

    h.get_by_label("外观").click();
    h.run();
    assert_eq!(h.state().ui.page, Page::Appearance);
    // 滑杆本体与它的文字标签都会命中，所以用 all 而不是单体查询。
    assert!(
        h.query_all_by_label("图标大小").count() >= 1,
        "外观页应出现尺寸滑杆"
    );
}

#[test]
fn 磨砂开关映射到背景材质() {
    let mut h = new_harness();
    h.get_by_label("性能").click();
    h.run();
    assert_eq!(h.state().cfg.bg_mode, BgMode::Frost);

    h.get_by_label("磨砂玻璃背景").click();
    h.run();

    // 关闭磨砂 = 退到渐变，而不是"另一个布尔键"。这条断言守住
    // "设置项与配置键不是一对一"这种最容易在重构中错位的映射。
    assert_eq!(h.state().cfg.bg_mode, BgMode::Gradient);
}

#[test]
fn 重置按钮产出命令而不是直接改配置() {
    let mut h = new_harness();
    h.get_by_label("添加").click();
    h.run();

    h.get_by_label("恢复默认设置").click();
    h.run();

    assert_eq!(h.state().commands.last(), Some(&Command::ResetDefaults));
    // UI 层不碰副作用：配置要等主程序执行命令后才变。
    assert!(!h.state().cfg.lock);
}

#[test]
fn 每一页都能渲染且不崩溃() {
    let mut h = new_harness();
    for page in Page::ALL {
        h.state_mut().ui.page = page;
        h.run();
    }
}

#[test]
fn 显卡默认自动且可切换到具体适配器() {
    let mut h = new_harness();
    h.state_mut().ui.adapters = vec![
        "AMD Radeon(TM) 610M".to_owned(),
        "NVIDIA GeForce RTX 5070 Laptop GPU".to_owned(),
    ];
    h.state_mut().ui.active_adapter = "AMD Radeon(TM) 610M".to_owned();
    h.state_mut().ui.page = Page::Performance;
    h.run();

    assert_eq!(h.state().cfg.adapter, "", "默认必须是自动");
    // 实际在用的那块卡要显示出来，不能只显示配置值。
    // 注意这是一个整句标签（"当前实际使用：X"），不能按裸名字查。
    assert!(
        h.query_all_by_label("当前实际使用：AMD Radeon(TM) 610M")
            .count()
            >= 1,
        "应显示当前实际使用的适配器"
    );

    h.get_by_label("显卡").click();
    h.run();
    h.get_by_label("NVIDIA GeForce RTX 5070 Laptop GPU").click();
    h.run();

    assert_eq!(h.state().cfg.adapter, "NVIDIA GeForce RTX 5070 Laptop GPU");
}

#[test]
fn 显卡可以切回自动() {
    let mut h = new_harness();
    h.state_mut().ui.adapters = vec!["AMD Radeon(TM) 610M".to_owned()];
    h.state_mut().cfg.adapter = "AMD Radeon(TM) 610M".to_owned();
    h.state_mut().ui.page = Page::Performance;
    h.run();

    h.get_by_label("显卡").click();
    h.run();
    h.get_by_label("自动（推荐）").click();
    h.run();

    assert_eq!(h.state().cfg.adapter, "", "切回自动要落成空串");
}

#[test]
fn 适配器回退时给出提示() {
    let mut h = new_harness();
    h.state_mut().ui.page = Page::Performance;
    h.state_mut().ui.adapter_note = "配置的「X」未找到，已回退自动".to_owned();
    h.run();

    assert!(
        h.query_all_by_label("配置的「X」未找到，已回退自动")
            .count()
            >= 1,
        "回退必须可见，否则用户会以为选择生效了"
    );
}

// ============================ 布局契约 ============================
//
// 这三条测的不是"配置对不对"，而是"东西画在哪"。视觉回归没法在 CI 上做
// （截图要 wgpu，而这一层刻意不依赖 wgpu），但"导航在左、内容在右、
// 谁也不盖谁"是可以用矩形断言守住的。

/// 导航项必须整行可点，且一律在内容区左侧。
#[test]
fn 导航栏在左且整行可点() {
    let mut h = new_harness();
    h.get_by_label("外观").click();
    h.run();
    assert_eq!(h.state().ui.page, Page::Appearance);

    // 按角色查：页头的大标题也叫「外观」，只按文字会命中两个节点。
    let nav = h.get_by_role_and_label(egui::Role::Tab, "外观").rect();
    // 滑杆本身（`Role::Slider`）：只按文字会同时命中左边的标题。
    let content = h
        .get_by_role_and_label(egui::Role::Slider, "图标大小")
        .rect();
    assert!(
        nav.right() <= content.left(),
        "导航项与内容区重叠：nav.right={} content.left={}",
        nav.right(),
        content.left()
    );
    assert!(
        nav.width() >= 140.0,
        "导航项应撑满整行（文字宽度只有几十像素），实得 {}",
        nav.width()
    );
}

/// 外壳先画了标题栏时，两列不能盖上去。
#[test]
fn 有标题栏时两列不会盖住标题() {
    let mut h = harness(true);
    h.run();

    let title = h.get_by_label("MiniDock").rect();
    let nav = h.get_by_role_and_label(egui::Role::Tab, "常规").rect();
    assert!(
        nav.top() >= title.bottom() - 1.0,
        "导航栏盖住了标题：nav.top={} title.bottom={}",
        nav.top(),
        title.bottom()
    );
}

/// 行的说明**不再排版**：收进标题后的圆形感叹号，点击展开成浮层。
///
/// 这是设计变更（雏形 2026-11 改版）的直接契约：说明占一行 → 收进圆点。
#[test]
fn 描述收进感叹号气泡() {
    let mut h = new_harness();
    h.run();

    // ① 说明文字默认不可见 —— 它现在只是气泡的内容。
    assert_eq!(
        h.query_all_by_label("防止误拖动、误移除图标").count(),
        0,
        "说明文字不该再排版出来"
    );

    // ② 圆点存在，且可访问名带"说明："前缀。
    let hint = h.get_by_role_and_label(egui::Role::Button, "说明：防止误拖动、误移除图标");
    hint.click();
    h.run();

    // ③ 点击后气泡展开，说明文字可见。
    assert!(
        h.query_all_by_label("防止误拖动、误移除图标").count() >= 1,
        "点击感叹号后应展开说明气泡"
    );
}

/// 点气泡之外收起 —— 雏形是"点别处就关"，钉住也不例外。
#[test]
fn 点击气泡外收起() {
    let mut h = new_harness();
    h.run();

    h.get_by_role_and_label(egui::Role::Button, "说明：防止误拖动、误移除图标")
        .click();
    h.run();
    assert!(
        h.query_all_by_label("防止误拖动、误移除图标").count() >= 1,
        "前置：气泡应已展开"
    );

    // 点一个无关的位置（页头标题）—— 点击事件照样落下去
    h.get_by_label("外观").click();
    h.run();

    assert_eq!(
        h.query_all_by_label("防止误拖动、误移除图标").count(),
        0,
        "点气泡外应收起"
    );
}

/// 顶栏搜索：只显示命中的导航标签；当前页被过滤掉时自动跳到第一个命中页。
#[test]
fn 搜索过滤导航并自动切页() {
    let mut h = new_harness();
    h.state_mut().ui.nav_filter = "图".to_owned();
    h.run();

    assert_eq!(
        h.state().ui.page,
        Page::Icons,
        "常规页不含'图'，应自动跳到图标页"
    );
    assert_eq!(h.state().ui.nav_filter, "图", "过滤词要一直留在输入框里");

    // 清空过滤词后切回逻辑不受影响：Page 仍是图标页，不回跳。
    h.state_mut().ui.nav_filter = String::new();
    h.run();
    assert_eq!(h.state().ui.page, Page::Icons);
}

/// 开关与下拉的右边缘必须对齐。
///
/// 同一张卡片里既有开关又有下拉时最容易错开：`Checkbox` 自带文字内边距，
/// 不给定尺寸就会比下拉缩进几个像素（附件页的「小组件」卡片就是这个组合）。
#[test]
fn 控件右边缘对齐() {
    let mut h = new_harness();
    h.state_mut().ui.page = Page::Extras;
    h.run();

    let toggle = h.get_by_role_and_label(egui::Role::CheckBox, "通知").rect();
    let combo = h
        .get_by_role_and_label(egui::Role::ComboBox, "小组件位置")
        .rect();
    assert!(
        (toggle.right() - combo.right()).abs() < 1.0,
        "开关右边缘 {} 与下拉右边缘 {} 没对齐",
        toggle.right(),
        combo.right()
    );
}
