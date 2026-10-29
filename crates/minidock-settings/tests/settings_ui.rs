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
    let state = St {
        cfg: Config::defaults(96),
        ui: SettingsState {
            config_dir: r"C:\Users\test\AppData\Roaming\MiniDock".to_owned(),
            ..SettingsState::default()
        },
        commands: Vec::new(),
    };
    // 窗口给得足够大，避免 ScrollArea 把控件裁掉导致查不到节点。
    Harness::builder()
        .with_size([900.0, 1200.0])
        // 必须在第一帧之前装字体：egui 0.37 在缺字形时**直接 panic**
        // （MissingGlyphPolicy::Panic），不是画成豆腐块。
        .with_context_setup(|ctx| {
            assert!(
                minidock_settings::fonts::install_cjk(ctx),
                "本机找不到中文字体，界面无法渲染"
            );
        })
        .build_ui_state(
            |ui, st: &mut St| {
                let outcome = settings_ui(ui, &mut st.cfg, &mut st.ui);
                if let Some(cmd) = outcome.command {
                    st.commands.push(cmd);
                }
            },
            state,
        )
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
