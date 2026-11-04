//! 把 `assets/minidock.ico` 嵌进 minidock-dock.exe 的资源表 ——
//! 资源管理器、任务栏、Alt+Tab 看到的应用图标就是它。
//!
//! 只在 Windows 目标上跑；找不到 rc.exe（没装 Windows SDK）时只警告，
//! 不让整个构建挂掉 —— 图标是外观，不是编译前提。

fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    if !target.contains("windows") {
        return;
    }

    let manifest = std::path::PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_owned()),
    );
    let icon = manifest.join("../../assets/minidock.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    if !icon.exists() {
        println!(
            "cargo:warning=找不到 {}，exe 将没有应用图标",
            icon.display()
        );
        return;
    }

    let mut res = winresource::WindowsResource::new();
    res.set_icon(icon.to_str().unwrap_or("assets/minidock.ico"));
    res.set("FileDescription", "MiniDock");
    res.set("ProductName", "MiniDock");
    res.set("CompanyName", "MiniDock Rust port contributors");
    // 只给主程序嵌，probe 之类的工具 exe 不需要。
    if let Err(e) = res.compile_for("minidock-dock") {
        println!("cargo:warning=应用图标资源编译失败（不影响运行）：{e}");
    }
}
