//! 手工验收 updater 的整条链路：查最新版 → 选镜像 → 下载 → 校验。
//!
//! ```text
//! cargo run -p nooksill-platform --example update_check           # 只查
//! cargo run -p nooksill-platform --example update_check -- stage  # 查到就下载并校验
//! ```
//!
//! 会真的发网络请求；`stage` 会把整包下载到 exe 同级目录的
//! `NookSill.update.exe`，校验不过就报错，不会覆盖任何东西。

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let want_stage = std::env::args().nth(1).as_deref() == Some("stage");
    let installed = nooksill_platform::update::Version::parse(nooksill_core::VERSION)
        .ok_or("当前版本号解析失败")?;
    println!("installed: {}", installed.text);

    let release = nooksill_platform::update::latest_release()?;
    println!(
        "latest:    {}  exe={}  sha={}  prerelease={}",
        release.version.text, release.exe.name, release.sha256.name, release.prerelease
    );
    println!(
        "newer:     {}",
        if release.version.is_newer_than(&installed) {
            "yes"
        } else {
            "no"
        }
    );

    let target = &release.sha256.url;
    let base = nooksill_platform::update::pick_mirror(target, false, true)
        .ok_or("没有可用镜像（连 GitHub 直连都失败）")?;
    println!(
        "mirror:    {}",
        if base.is_empty() { "(direct)" } else { &base }
    );

    if want_stage {
        let path = nooksill_platform::update::stage(&base, &release)?;
        let size = std::fs::metadata(&path)?.len();
        println!("staged:    {} ({} bytes)", path.display(), size);
    }
    Ok(())
}
