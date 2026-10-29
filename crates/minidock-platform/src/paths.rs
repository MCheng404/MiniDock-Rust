//! 路径解析：便携模式、配置目录、图标缓存目录。
//!
//! 规则与原版 `InitPaths()` 完全一致（顺序也一致，因为"先建目录还是先判便携"
//! 会影响迁移结果）：
//!
//! 1. 默认配置目录 = `%APPDATA%\MiniDock`，配置文件 `MiniDock.ini`。
//! 2. exe 同目录存在 `MiniDock.portable`（空文件即可）→ 整个目录改用 exe 所在目录。
//! 3. 否则若 `%APPDATA%` 下还没有 ini、但 exe 旁边有旧 ini → 迁移一次
//!    （连同 `icons\` 一起复制），之后就以 `%APPDATA%` 为准。
//! 4. 最后确保 ini 与 `icons\` 存在。

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use windows::Win32::UI::Shell::SHGetFolderPathW;

use crate::win::{from_wide, to_wide};

/// `CSIDL_APPDATA` = `FOLDERID_RoamingAppData`。手写常量是为了不引入
/// `Win32_UI_Shell_Common` 里那一整组 CSIDL 常量（它们全是 u32，容易混用）。
const CSIDL_APPDATA: i32 = 0x001A;

const MAX_PATH: usize = 260;

#[derive(Debug)]
pub enum PathsError {
    ExePath,
    AppData,
    Io(io::Error),
}

impl fmt::Display for PathsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExePath => write!(f, "无法取得 exe 路径（GetModuleFileNameW 失败）"),
            Self::AppData => write!(f, "无法取得 %APPDATA% 路径（SHGetFolderPathW 失败）"),
            Self::Io(e) => write!(f, "文件操作失败: {e}"),
        }
    }
}

impl std::error::Error for PathsError {}

impl From<io::Error> for PathsError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// 解析后的运行期路径。
#[derive(Clone, Debug)]
pub struct Paths {
    /// 配置根目录（`%APPDATA%\MiniDock` 或 exe 目录）。
    pub dir: PathBuf,
    /// `MiniDock.ini` 完整路径。
    pub ini: PathBuf,
    /// 图标缓存目录。
    pub icon_dir: PathBuf,
    /// 当前 exe 路径。
    pub exe: PathBuf,
    /// 是否处于便携模式。
    pub portable: bool,
}

impl Paths {
    /// 解析并创建必要目录。
    pub fn resolve() -> Result<Self, PathsError> {
        let exe = exe_path()?;
        let exe_dir = exe.parent().ok_or(PathsError::ExePath)?.to_path_buf();
        let appdata = appdata_path()?;

        let mut dir = appdata.join("MiniDock");
        let mut ini = dir.join("MiniDock.ini");
        let local_ini = exe_dir.join("MiniDock.ini");
        let marker = exe_dir.join("MiniDock.portable");

        let mut portable = false;
        if marker.exists() {
            portable = true;
            dir = exe_dir.clone();
            ini = local_ini.clone();
        } else if !ini.exists() && local_ini.exists() {
            // 迁移一次：配置 + 图标缓存。
            fs::create_dir_all(&dir)?;
            let _ = fs::copy(&local_ini, &ini);
            let src_icons = exe_dir.join("icons");
            let dst_icons = dir.join("icons");
            if src_icons.is_dir() {
                let _ = fs::create_dir_all(&dst_icons);
                if let Ok(entries) = fs::read_dir(&src_icons) {
                    for entry in entries.flatten() {
                        let from = entry.path();
                        if from.is_file() {
                            let to = dst_icons.join(entry.file_name());
                            let _ = fs::copy(from, to);
                        }
                    }
                }
            }
        }

        fs::create_dir_all(&dir)?;
        let icon_dir = dir.join("icons");
        fs::create_dir_all(&icon_dir)?;
        ensure_empty_ini(&ini)?;

        Ok(Self {
            dir,
            ini,
            icon_dir,
            exe,
            portable,
        })
    }

    /// 归档已用过的图标缓存文件（供设置界面"清理图标缓存"用，暂未接入）。
    #[must_use]
    pub fn icon_for(&self, file_name: &str) -> PathBuf {
        self.icon_dir.join(file_name)
    }
}

/// 若 ini 不存在则创建一个只含 UTF-16 BOM 的空文件。
///
/// 直接 `fs::write(path, b"")` 会得到 0 字节文件，随后 `WritePrivateProfileStringW`
/// 会按 ANSI 写回，中文名立刻变问号。BOM 是编码契约的一部分。
fn ensure_empty_ini(path: &Path) -> io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    fs::write(path, [0xFFu8, 0xFE])
}

fn exe_path() -> Result<PathBuf, PathsError> {
    let mut buf = [0u16; MAX_PATH];
    let len = unsafe { windows::Win32::System::LibraryLoader::GetModuleFileNameW(None, &mut buf) };
    if len == 0 {
        return Err(PathsError::ExePath);
    }
    Ok(PathBuf::from(from_wide(&buf[..len as usize])))
}

fn appdata_path() -> Result<PathBuf, PathsError> {
    let mut buf = [0u16; MAX_PATH];
    unsafe { SHGetFolderPathW(None, CSIDL_APPDATA, None, 0, &mut buf) }
        .map_err(|_| PathsError::AppData)?;
    let s = from_wide(&buf);
    if s.is_empty() {
        return Err(PathsError::AppData);
    }
    Ok(PathBuf::from(s))
}

/// 给 `SetAutostart` 用：把 exe 路径包成带引号的命令行。
#[must_use]
pub fn quoted_exe(exe: &Path) -> Vec<u16> {
    let s = format!("\"{}\"", exe.display());
    to_wide(&s)
}
