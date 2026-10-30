//! 启动条目。对应原版 `LaunchPtr()` / `OpenUrlItem()`。
//!
//! 这里只负责"把东西跑起来"，不做任何 UI 反馈 —— 弹跳动画、运行状态扫描
//! 都在调用方。分成两层的原因是：启动是唯一会**产生外部副作用**的动作，
//! 单独一个模块才能在测试里显式地不打桩就跑不起来。

use minidock_core::item::Item;
use windows::core::PCWSTR;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// 启动失败的原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchError {
    /// 该类型还没实现（组、速记、文件夹扇面）。
    Unsupported(&'static str),
    /// Shell 拒绝执行，`code` 是 `ShellExecuteW` 的返回值。
    Shell(i32),
    /// 要打开的路径是空的。
    EmptyTarget,
    /// 路径不存在（或类型不对，例如要求目录却给了文件）。
    NotFound(String),
}

impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(what) => write!(f, "{what} 尚未实现"),
            // 原版同样以 32 为界；把原始码打出来比"失败"两个字有用得多。
            Self::EmptyTarget => write!(f, "路径为空"),
            Self::NotFound(p) => write!(f, "路径不存在：{p}"),
            Self::Shell(0) => write!(f, "系统内存不足"),
            Self::Shell(2) => write!(f, "找不到文件"),
            Self::Shell(3) => write!(f, "找不到路径"),
            Self::Shell(5) => write!(f, "访问被拒绝"),
            Self::Shell(c) => write!(f, "Shell 错误码 {c}"),
        }
    }
}

/// UTF-16 + NUL。`ShellExecuteW` 要求以 NUL 结尾的宽字符串。
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 调一次 `ShellExecuteW("open", ...)`。
///
/// 返回值 <= 32 表示失败（这是 Win32 的约定，不是我们的选择）。
fn shell_open(file: &str, params: Option<&str>, dir: Option<&str>) -> Result<(), LaunchError> {
    let file_w = wide(file);
    let params_w = params.map(wide);
    let dir_w = dir.map(wide);
    let op = wide("open");

    let ret = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(op.as_ptr()),
            PCWSTR(file_w.as_ptr()),
            params_w
                .as_ref()
                .map_or(PCWSTR::null(), |v| PCWSTR(v.as_ptr())),
            dir_w
                .as_ref()
                .map_or(PCWSTR::null(), |v| PCWSTR(v.as_ptr())),
            SW_SHOWNORMAL,
        )
    };
    // `HINSTANCE` 装的是 isize；<= 32 是错误码。
    let code = ret.0 as i32;
    if code <= 32 {
        return Err(LaunchError::Shell(code));
    }
    Ok(())
}

/// 网页应用：`app_mode == 1` 时优先用 Edge / Chrome 的 `--app` 窗口。
///
/// 顺序照抄原版：Edge 优先于 Chrome（Windows 上 Edge 一定存在），
/// 两者都失败才退回默认浏览器 —— 不带 `--app` 的普通标签页。
fn open_url(url: &str, app_mode: i32) -> Result<(), LaunchError> {
    if app_mode == 1 {
        let params = format!("--app={url}");
        if shell_open("msedge.exe", Some(&params), None).is_ok() {
            return Ok(());
        }
        if shell_open("chrome.exe", Some(&params), None).is_ok() {
            return Ok(());
        }
    }
    shell_open(url, None, None)
}

/// 启动一个条目。
///
/// 与 `ItemKind` 的对应关系：
///
/// | 类型 | 行为 |
/// |---|---|
/// | 分割线 | 什么都不做（不是错误） |
/// | 网址 | `--app` 窗口或默认浏览器 |
/// | 回收站 | `shell:RecycleBinFolder` |
/// | 文件 | 工作目录设为文件所在目录 |
/// | 文件夹 | 直接打开，不设工作目录 |
/// | 组 / 速记 | 未实现（要等弹窗层） |
///
/// 文件与文件夹的差别是原版就有的：给文件设工作目录，是为了让那些
/// "在当前目录找资源"的老程序能找到旁边的 dll/配置。
pub fn launch(item: &Item) -> Result<(), LaunchError> {
    let params = if item.args.is_empty() {
        None
    } else {
        Some(item.args.as_str())
    };

    match item.kind {
        minidock_core::item::ItemKind::Separator => Ok(()),
        minidock_core::item::ItemKind::Url => open_url(&item.path, item.app_mode),
        minidock_core::item::ItemKind::Trash => shell_open("shell:RecycleBinFolder", None, None),
        minidock_core::item::ItemKind::File => {
            let dir = std::path::Path::new(&item.path)
                .parent()
                .map(|p| p.to_string_lossy().into_owned());
            shell_open(&item.path, params, dir.as_deref())
        }
        minidock_core::item::ItemKind::Folder => shell_open(&item.path, params, None),
        minidock_core::item::ItemKind::Group => Err(LaunchError::Unsupported("应用组")),
        minidock_core::item::ItemKind::Note => Err(LaunchError::Unsupported("速记")),
    }
}

/// 在资源管理器中打开一个目录（右键菜单的"在资源管理器中打开"）。
///
/// 与 [`launch`] 的区别：这里**总是**用 Explorer，不看条目类型，
/// 也不传条目自带的参数 —— 那个参数是给应用启动用的，
/// 塞给 `explorer.exe` 可能被当成开关解析。
pub fn open_folder(path: &str) -> Result<(), LaunchError> {
    if path.is_empty() {
        return Err(LaunchError::EmptyTarget);
    }
    if !std::path::Path::new(path).is_dir() {
        // 不是目录就别假装能打开：`ShellExecute` 对不存在的路径
        // 也可能"成功"返回（转交给默认处理程序），错误会静默消失。
        return Err(LaunchError::NotFound(path.to_owned()));
    }
    shell_open(path, None, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use minidock_core::item::{Item, ItemKind};

    fn item(kind: ItemKind) -> Item {
        Item::new(kind, "测试", "")
    }

    /// 分割线不是"启动失败"，调用方不该因为它弹错误提示。
    #[test]
    fn separator_is_a_noop() {
        assert_eq!(launch(&item(ItemKind::Separator)), Ok(()));
    }

    /// 组与速记要返回明确的"未实现"，而不是静默成功 ——
    /// 否则用户点了没反应，也没法从日志里看出原因。
    #[test]
    fn group_and_note_report_unsupported() {
        assert_eq!(
            launch(&item(ItemKind::Group)),
            Err(LaunchError::Unsupported("应用组"))
        );
        assert_eq!(
            launch(&item(ItemKind::Note)),
            Err(LaunchError::Unsupported("速记"))
        );
    }

    /// 真的启动一个进程并检查它产生了副作用。
    ///
    /// 标 `ignore` 是因为它会**真的弹出进程**（测试跑在桌面上会闪黑框）。
    /// 需要验证启动链路时显式跑：
    ///
    /// ```text
    /// cargo test -p minidock-platform --lib launch -- --ignored
    /// ```
    #[test]
    #[ignore = "会真的启动进程，需显式 --ignored"]
    fn launches_a_real_command_and_it_has_an_effect() {
        let marker = std::env::temp_dir().join("minidock_launch_probe.txt");
        let _ = std::fs::remove_file(&marker);

        let mut it = Item::new(ItemKind::File, "探针", r"C:\Windows\System32\cmd.exe");
        // 用 `>` 重定向写文件：能证明命令行参数真的被传下去了，
        // 而不只是"进程起来了"。
        it.args = format!("/c echo ok > {}", marker.display());
        launch(&it).expect("启动失败");

        // ShellExecute 是异步的，轮询等它落盘。
        let mut seen = false;
        for _ in 0..40 {
            if marker.exists() {
                seen = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(seen, "标记文件没生成，说明命令没真正执行");
        let text = std::fs::read_to_string(&marker).unwrap_or_default();
        assert!(text.contains("ok"), "标记文件内容不对: {text:?}");
        let _ = std::fs::remove_file(&marker);
    }

    /// 错误码要能翻译成人话，至少把常见几个覆盖掉。
    #[test]
    fn error_codes_have_messages() {
        assert_eq!(LaunchError::Shell(2).to_string(), "找不到文件");
        assert_eq!(LaunchError::Shell(5).to_string(), "访问被拒绝");
        assert!(LaunchError::Shell(1234).to_string().contains("1234"));
    }
}
