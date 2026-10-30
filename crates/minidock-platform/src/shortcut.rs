//! 解析快捷方式，得到它真正指向的可执行文件。
//!
//! 对应 C 版 `ReadLnk()` 的"取 target"部分（它还有取图标的部分，
//! 我们不解析图标，图标走 `platform::icon` 的 shell 图像列表）。
//!
//! # 为什么需要它
//!
//! Dock 条目多数是 `.lnk`。要判断"这个应用是否在运行"、以及"点击时该切过去
//! 还是新开"，都得把这个 `.lnk` 还原成 `EXCEL.EXE` 这样的真实路径。
//!
//! # 为什么直接调 vtable
//!
//! `windows` 0.62 给 `IShellLinkW::GetPath` / `GetArguments` 生成的 wrapper
//! 签名与 COM vtable 不符（vtable 要 `(PWSTR, cch, WIN32_FIND_DATAW*, flags)`，
//! wrapper 只传了两个参数）。照 wrapper 调用是未定义行为，因此这里绕过
//! wrapper，直接用 `Interface::vtable()` 按 COM ABI 调用。
//!
//! 代价是这段代码依赖 vtable 布局。升级 `windows` 大版本时要重新核对 ——
//! 有一个 `#[ignore]` 的真实解析测试可以立刻发现回归。

use windows::core::{Interface, PCWSTR, PWSTR};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    STGM_READ,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

/// 快捷方式解析结果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shortcut {
    /// 目标可执行文件的完整路径（空 = 解析不出来，比如指向 URL 或 UWP 应用）。
    pub target: String,
    /// 启动参数。直接启动 exe 时为空（参数由条目自己带）。
    pub args: String,
}

/// 保证 COM 初始化一次。
///
/// 选 STA（`COINIT_APARTMENTTHREADED`）而不是 MTA：shell 的 IShellLink
/// 在 STA 下行为最稳。我们不 `CoUninitialize` —— 进程退出时系统会回收，
/// 而提前反初始化会让之后任何 COM 调用都失败。
///
/// 若调用方已经把线程初始化成 MTA，这里会返回 `RPC_E_CHANGED_MODE`，
/// 此时**仍然可以**用 IShellLink（它不要求特定套间），所以忽略错误。
fn ensure_com() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: 只执行一次，且没有配对的反初始化（见函数文档）。
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
    });
}

/// 解析 `.lnk`，取它指向的 exe。
fn resolve_lnk(path: &str) -> Option<Shortcut> {
    ensure_com();

    // SAFETY: 全部是 COM 标准调用；接口指针由 COM 所有权管理（RAII 释放）。
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let persist: IPersistFile = link.cast().ok()?;
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        persist.Load(PCWSTR(wide.as_ptr()), STGM_READ).ok()?;

        let mut buf = [0u16; 1024];
        let vt = Interface::vtable(&link);
        // vtable 的真实签名：`(this, PWSTR, cch, WIN32_FIND_DATAW*, flags)`。
        // flags = 0 表示要标准长路径（`SLGP_RAWPATH` 会给带环境变量的形式）。
        let hr = (vt.GetPath)(
            Interface::as_raw(&link),
            PWSTR(buf.as_mut_ptr()),
            buf.len() as i32,
            std::ptr::null_mut(),
            0,
        );
        if hr.is_err() {
            return None;
        }

        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        if n == 0 {
            // 指向 URL / UWP 应用的快捷方式没有文件路径，属于正常情况。
            return None;
        }
        let target = String::from_utf16_lossy(&buf[..n]);

        // 参数解析失败不影响主功能（target 才是关键），忽略。
        let mut abuf = [0u16; 1024];
        let has_args = (vt.GetArguments)(
            Interface::as_raw(&link),
            PWSTR(abuf.as_mut_ptr()),
            abuf.len() as i32,
        )
        .is_ok();
        let args = if has_args {
            let m = abuf.iter().position(|&c| c == 0).unwrap_or(abuf.len());
            String::from_utf16_lossy(&abuf[..m])
        } else {
            String::new()
        };

        Some(Shortcut { target, args })
    }
}

/// 把一个条目路径解析成"可执行文件路径"。
///
/// - `.exe` → 原样返回（参数为空）
/// - `.lnk` → 解析出 target；target 不是 exe（指向文档/URL）时返回 `None`
/// - 其它 → `None`
///
/// 返回 `None` 表示"这个条目没有可匹配的 exe"，调用方应直接走启动流程，
/// 不要尝试按 exe 匹配已运行实例。
#[must_use]
pub fn resolve(path: &str) -> Option<Shortcut> {
    let lower = path.to_lowercase();
    if lower.ends_with(".exe") {
        return Some(Shortcut {
            target: path.to_owned(),
            args: String::new(),
        });
    }
    if !lower.ends_with(".lnk") {
        return None;
    }
    let sc = resolve_lnk(path)?;
    // 快捷方式可以指向文档或 URL，那种情况没有 exe 可匹配。
    if !sc.target.to_lowercase().ends_with(".exe") {
        return None;
    }
    Some(sc)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 非快捷方式、非 exe 的路径不该被解析。
    #[test]
    fn non_lnk_non_exe_has_no_target() {
        assert!(resolve("C:\\Users\\Public\\Documents").is_none());
        assert!(resolve("https://example.com").is_none());
        assert!(resolve("C:\\a\\readme.txt").is_none());
    }

    /// exe 直接返回自身，参数为空。
    #[test]
    fn exe_resolves_to_itself() {
        let s = resolve("C:\\Windows\\System32\\notepad.exe").expect("应当解析出自身");
        assert_eq!(s.target, "C:\\Windows\\System32\\notepad.exe");
        assert!(s.args.is_empty());
        // 大小写不同的扩展名同样要识别。
        let s = resolve("C:\\X\\NOTEPAD.EXE").expect("扩展名比较要忽略大小写");
        assert!(s.target.ends_with("NOTEPAD.EXE"));
    }

    /// 不存在的 .lnk 返回 None，不能 panic 也不能编造路径。
    #[test]
    fn missing_lnk_returns_none() {
        assert!(resolve("C:\\definitely\\missing\\thing.lnk").is_none());
    }

    /// 真实解析：对着系统里必然存在的快捷方式验证。
    ///
    /// 标 `ignore` 是因为它依赖具体机器上装了什么。要验证 COM 通路
    /// （尤其是绕过 wrapper 直接调 vtable 那段）时显式跑：
    ///
    /// ```text
    /// cargo test -p minidock-platform --lib shortcut -- --ignored
    /// ```
    #[test]
    #[ignore = "依赖本机已安装的应用，需显式 --ignored"]
    fn resolves_a_real_start_menu_shortcut() {
        let root = std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".to_owned());
        let dir = std::path::Path::new(&root).join("Microsoft\\Windows\\Start Menu\\Programs");
        let mut tried = 0;
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("lnk") {
                continue;
            }
            tried += 1;
            let Some(sc) = resolve(&p.to_string_lossy()) else {
                continue;
            };
            assert!(
                sc.target.to_lowercase().ends_with(".exe"),
                "解析结果不是 exe: {}",
                sc.target
            );
            assert!(
                std::path::Path::new(&sc.target).exists(),
                "解析出的 exe 不存在: {}",
                sc.target
            );
            println!("{} -> {}", p.display(), sc.target);
            return;
        }
        // 一个 .lnk 都没找到时不判失败（可能装在别处），但要说明。
        println!("开始菜单里只有 {tried} 个 .lnk，跳过");
    }
}
