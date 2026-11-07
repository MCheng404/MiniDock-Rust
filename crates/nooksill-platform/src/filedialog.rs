//! 系统文件对话框。
//!
//! 用 `IFileOpenDialog` 而不是老式 `GetOpenFileNameW`：后者没有文件夹模式，
//! 也不能设默认扩展名；前者是 Vista 之后的标准，视觉与资源管理器一致。
//!
//! 对话框是**模态**的，会阻塞调用它的线程 —— 所以只能在 UI 线程上、由用户
//! 点击触发。我们不自己开线程：文件对话框与调用方的窗口有父子关系，跨线程
//! 反而会变成"对话框沉在窗口后面点不到"。

use std::path::PathBuf;

use windows::core::HSTRING;
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FileOpenDialog, IFileOpenDialog, IShellItem, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM,
    FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
};

use crate::win;

/// 一条过滤器：`("应用", "*.exe;*.lnk")`。
pub type Filter<'a> = (&'a str, &'a str);

/// 选一个文件。`owner` 是父窗口，`None` 时以桌面为父。
#[must_use]
pub fn open_file(title: &str, owner: Option<isize>, filters: &[Filter<'_>]) -> Option<PathBuf> {
    show(owner, title, filters, Mode::Open)
}

/// 选一个文件夹。
#[must_use]
pub fn open_folder(title: &str, owner: Option<isize>) -> Option<PathBuf> {
    show(owner, title, &[], Mode::Folder)
}

/// 另存为。`default_name` 只是预填的文件名，用户仍可改。
#[must_use]
pub fn save_file(title: &str, owner: Option<isize>, default_name: &str) -> Option<PathBuf> {
    let dialog = build(title, &[], Mode::Save)?;
    unsafe {
        let _ = dialog.SetFileName(&HSTRING::from(default_name));
    }
    finish(dialog, owner)
}

enum Mode {
    Open,
    Folder,
    Save,
}

fn build(title: &str, filters: &[Filter<'_>], mode: Mode) -> Option<IFileOpenDialog> {
    win::ensure_com();
    // SAFETY: `ensure_com` 保证本线程已初始化；CoCreateInstance 是标准的
    // shell 对话框创建路径，失败只会返回 Err。
    let dialog: IFileOpenDialog =
        unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()? };
    unsafe {
        let mut options = FOS_FORCEFILESYSTEM;
        match mode {
            Mode::Folder => options |= FOS_PICKFOLDERS | FOS_PATHMUSTEXIST,
            Mode::Open => options |= FOS_FILEMUSTEXIST | FOS_PATHMUSTEXIST,
            Mode::Save => options |= FOS_OVERWRITEPROMPT,
        }
        dialog.SetOptions(options).ok()?;
        dialog.SetTitle(&HSTRING::from(title)).ok()?;

        if !filters.is_empty() {
            // 名字与模式都要活到 `SetFileTypes` 返回之后：COM 只拷贝指针
            // 指向的内容？不，它按接口约定拷贝，但把 HSTRING 放进 Vec 再
            // 逐项取 ptr 更省心 —— 这里两段都留在作用域内。
            let names: Vec<HSTRING> = filters.iter().map(|(n, _)| HSTRING::from(*n)).collect();
            let specs: Vec<HSTRING> = filters.iter().map(|(_, s)| HSTRING::from(*s)).collect();
            let specs_ref: Vec<COMDLG_FILTERSPEC> = names
                .iter()
                .zip(&specs)
                .map(|(n, s)| COMDLG_FILTERSPEC {
                    pszName: windows::core::PCWSTR(n.as_ptr()),
                    pszSpec: windows::core::PCWSTR(s.as_ptr()),
                })
                .collect();
            dialog.SetFileTypes(&specs_ref).ok()?;
        }
    }
    Some(dialog)
}

fn finish(dialog: IFileOpenDialog, owner: Option<isize>) -> Option<PathBuf> {
    // SAFETY: `owner` 是调用方给的 HWND，`None` 表示无父窗口。
    unsafe {
        let parent = owner.map(|h| windows::Win32::Foundation::HWND(h as *mut _));
        match dialog.Show(parent) {
            Ok(()) => {}
            // 用户按取消是**最常见**的返回，不是错误，别往日志里塞。
            Err(e) if e.code() == windows::Win32::Foundation::ERROR_CANCELLED.to_hresult() => {
                return None;
            }
            Err(e) => {
                // 其它错误（COM 没初始化、shell 拒绝等）与"取消"在调用方
                // 看来都是 None —— 至少留一行日志，否则"点了没反应"无从查起。
                eprintln!("[dialog] 打开文件对话框失败：{e}");
                return None;
            }
        }
        let item: IShellItem = dialog.GetResult().ok()?;
        let raw = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let text = raw.to_string().ok()?;
        CoTaskMemFree(Some(raw.0.cast()));
        Some(PathBuf::from(text))
    }
}

/// `show` 是上面三个入口的共同实现，只差构造时的模式。
fn show(owner: Option<isize>, title: &str, filters: &[Filter<'_>], mode: Mode) -> Option<PathBuf> {
    let dialog = build(title, filters, mode)?;
    finish(dialog, owner)
}

/// 在 `parent` 下新建一个目录，返回创建后的完整路径。
///
/// 名字要先过 [`check_dir_name`]：路径分隔符、保留字符、空名都要挡掉 ——
/// 用户在输入框里敲 `..\..\Windows` 不该在别处建出目录来。
pub fn create_dir(parent: &std::path::Path, name: &str) -> Result<PathBuf, String> {
    let name = check_dir_name(name)?;
    let path = parent.join(&name);
    std::fs::create_dir(&path).map_err(|e| format!("创建目录失败：{e}"))?;
    Ok(path)
}

/// 校验目录名。返回清洗后的名字。
///
/// 规则：`..` 与路径分隔符一律拒绝（不能借它跳出父目录）；Windows 的
/// 保留字符 `< > : " / \ | ? *` 与控制字符拒绝；结尾的点与空格去掉
/// （Win32 会静默去掉，用户看到的名字与实际不符）。
pub fn check_dir_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("名字不能为空".into());
    }
    if trimmed == "." || trimmed == ".." {
        return Err("名字不能是 . 或 ..".into());
    }
    if trimmed.chars().any(|c| {
        c == '/'
            || c == '\\'
            || c == ':'
            || c == '*'
            || c == '?'
            || c == '"'
            || c == '<'
            || c == '>'
            || c == '|'
            || (c as u32) < 0x20
    }) {
        return Err("名字里有不能用的字符".into());
    }
    let cleaned = trimmed.trim_end_matches([' ', '.']).to_owned();
    if cleaned.is_empty() {
        return Err("名字不能为空".into());
    }
    Ok(cleaned)
}

#[cfg(test)]
mod tests {
    use super::check_dir_name;

    #[test]
    fn accepts_ordinary_names() {
        assert_eq!(check_dir_name("工具").unwrap(), "工具");
        assert_eq!(check_dir_name("  下载  ").unwrap(), "下载");
        assert_eq!(check_dir_name("a b").unwrap(), "a b");
    }

    #[test]
    fn rejects_path_traversal_and_separators() {
        assert!(check_dir_name("..").is_err());
        assert!(check_dir_name(".").is_err());
        assert!(check_dir_name("a/b").is_err());
        assert!(check_dir_name("a\\b").is_err());
        assert!(check_dir_name("C:x").is_err());
    }

    #[test]
    fn rejects_empty_and_reserved_characters() {
        assert!(check_dir_name("").is_err());
        assert!(check_dir_name("   ").is_err());
        assert!(check_dir_name("a<b").is_err());
        assert!(check_dir_name("a|b").is_err());
        assert!(check_dir_name("a\u{1}b").is_err());
    }

    #[test]
    fn trims_trailing_dots_and_spaces() {
        // Win32 会静默去掉结尾的点与空格，不处理的话用户看到的名字
        // 与实际建出来的目录不一样。
        assert_eq!(check_dir_name("abc. ").unwrap(), "abc");
    }
}
