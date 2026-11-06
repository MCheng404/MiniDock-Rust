//! 把 `assets/nooksill.ico` 与版本信息嵌进 nooksill-dock.exe 的资源表 ——
//! 资源管理器、任务栏、Alt+Tab 看到的应用图标就是它，属性页里的版本号
//! 来自 `scripts/version.rs`（与 `nooksill_core::VERSION`、产物名同源）。
//!
//! 只在 Windows 目标上跑；找不到 rc.exe（没装 Windows SDK）时只警告，
//! 不让整个构建挂掉 —— 图标是外观，不是编译前提。

include!("../../scripts/version.rs");

fn main() {
    println!("cargo:rerun-if-env-changed=NOOKSILL_VERSION");
    println!("cargo:rerun-if-env-changed=NOOKSILL_BUILD");
    println!("cargo:rerun-if-changed=../../scripts/version.rs");
    println!("cargo:rerun-if-changed=../../.git/logs/HEAD");

    let target = std::env::var("TARGET").unwrap_or_default();
    if !target.contains("windows") {
        return;
    }

    let version = resolve(env!("CARGO_PKG_VERSION"));
    let manifest = std::path::PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_owned()),
    );
    let icon = manifest.join("../../assets/nooksill.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    if !icon.exists() {
        println!(
            "cargo:warning=找不到 {}，exe 将没有应用图标",
            icon.display()
        );
        return;
    }

    let mut res = winresource::WindowsResource::new();
    res.set_icon(icon.to_str().unwrap_or("assets/nooksill.ico"));
    res.set("FileDescription", "NookSill");
    res.set("ProductName", "NookSill");
    res.set("CompanyName", "NookSill Rust port contributors");
    res.set("LegalCopyright", "GNU GPL v3.0 or Later");
    res.set("OriginalFilename", "nooksill-dock.exe");
    res.set("InternalName", "nooksill-dock");
    // 数值版本决定"属性 → 详细信息"里的排序，字符串版本决定显示文本；
    // 两个都要写，否则资源管理器会把 dev 后缀吞掉。
    res.set_version_info(winresource::VersionInfo::FILEVERSION, packed(&version));
    res.set_version_info(winresource::VersionInfo::PRODUCTVERSION, packed(&version));
    res.set("FileVersion", &version.text);
    res.set("ProductVersion", &version.text);

    // 只给主程序嵌，probe 之类的工具 exe 不需要。
    if let Err(e) = res.compile_for("nooksill-dock") {
        println!("cargo:warning=应用图标资源编译失败（不影响运行）：{e}");
    }
}
