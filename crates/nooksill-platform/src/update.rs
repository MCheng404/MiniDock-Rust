//! In-app update: check the GitHub release, pick a mirror, download, verify.
//!
//! The flow is deliberately boring and always ends in a file the user can
//! inspect: nothing is executed until the downloaded bytes match the
//! `.sha256` the release ships.
//!
//! Mirrors exist because `github.com` is often unreachable from the mainland.
//! They are plain reverse proxies: `https://<mirror>/<original-url>`. Which one
//! is used is decided at runtime by timing a small request, not by a hardcoded
//! order — these hosts change weekly.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use sha2::{Digest, Sha256};

use crate::json::{self, Json};
use crate::net;

/// Repository the updater talks to.
pub const REPO: &str = "MCheng404/NookSill-Rust";

/// Reverse proxies, fastest-first by convention; the real order comes from
/// timing. Keep them all with a trailing slash — [`mirror_url`] relies on it.
pub const MIRRORS: &[&str] = &[
    "https://gh-proxy.com/",
    "https://ghfast.top/",
    "https://ghproxy.net/",
    "https://gh.llkk.cc/",
    "https://mirror.houlang.cloud/",
];

/// A mirror answering faster than this is used without testing the rest.
pub const FAST_ENOUGH_MS: u128 = 600;

const TIMEOUT_MS: i32 = 12_000;
const PROBE_TIMEOUT_MS: i32 = 4_000;
const UA: &str = "NookSill-Updater";

#[derive(Debug)]
pub enum UpdateError {
    Net(net::NetError),
    Json,
    Http(String),
    /// The release has no `.exe` asset — packaging changed under us.
    NoAsset,
    /// GitHub 上一个已发布的版本都没有。
    NoRelease,
    Io(std::io::Error),
    /// Downloaded bytes do not match the published checksum.
    Checksum {
        expected: String,
        actual: String,
    },
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // GitHub 匿名接口每小时只有 60 次；撞上限时给一句能懂的话，
            // 免得用户把 403 当成仓库没了。
            Self::Net(net::NetError::Status(code @ (403 | 429))) => {
                write!(
                    f,
                    "GitHub 接口限流（每小时 60 次，当前 HTTP {code}），请稍后再试"
                )
            }
            Self::Net(e) => write!(f, "网络错误：{e}"),
            Self::Json => write!(f, "GitHub 返回的内容无法解析"),
            Self::Http(what) => write!(f, "{what}"),
            Self::NoAsset => write!(f, "该版本没有可用的 exe 资产"),
            Self::NoRelease => write!(f, "GitHub 上还没有发布任何版本"),
            Self::Io(e) => write!(f, "文件读写失败：{e}"),
            Self::Checksum { expected, actual } => {
                write!(f, "校验失败：期望 {expected}，实际 {actual}")
            }
        }
    }
}

impl std::error::Error for UpdateError {}

impl From<net::NetError> for UpdateError {
    fn from(e: net::NetError) -> Self {
        Self::Net(e)
    }
}

impl From<std::io::Error> for UpdateError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// A parsed version. `build` is `Some` only for dev builds (`1.0.0-65-abc1234`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
    pub build: Option<u64>,
    pub text: String,
}

impl Version {
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().trim_start_matches('v');
        let mut groups = text.split('-');
        let core = groups.next()?;
        let mut it = core.split('.');
        let major = it.next()?.parse().ok()?;
        let minor = it.next().unwrap_or("0").parse().ok()?;
        let patch = it.next().unwrap_or("0").parse().ok()?;
        let build = groups.next().and_then(|g| g.parse().ok());
        Some(Self {
            major,
            minor,
            patch,
            build,
            text: text.to_owned(),
        })
    }

    /// Is `self` newer than `installed`?
    ///
    /// Same stable triple: a release beats a dev build (a dev build is a
    /// snapshot *of* that version); dev vs dev compares build numbers.
    #[must_use]
    pub fn is_newer_than(&self, installed: &Self) -> bool {
        let mine = (self.major, self.minor, self.patch);
        let theirs = (installed.major, installed.minor, installed.patch);
        if mine != theirs {
            return mine > theirs;
        }
        match (self.build, installed.build) {
            (None, Some(_)) => true,
            (Some(a), Some(b)) => a > b,
            _ => false,
        }
    }
}

/// One downloadable file from the release.
#[derive(Debug, Clone)]
pub struct Asset {
    pub name: String,
    pub url: String,
}

/// Everything the updater needs from a release.
#[derive(Debug, Clone)]
pub struct Release {
    pub tag: String,
    pub version: Version,
    pub notes: String,
    pub exe: Asset,
    pub sha256: Asset,
    /// 预发布（`dev` 通道）。界面据此把"新版本"说成"新的预发布"。
    pub prerelease: bool,
}

/// `https://github.com/x` → `https://<mirror>/https://github.com/x`.
#[must_use]
pub fn mirror_url(mirror: &str, original: &str) -> String {
    if mirror.is_empty() {
        original.to_owned()
    } else {
        format!("{mirror}{original}")
    }
}

/// Ask GitHub for the newest release we can offer.
///
/// `/releases/latest` answers **404** whenever there is no *stable* release —
/// after a release cleanup, or while the project only has pre-releases. That
/// is a normal state, not an error: fall back to the list endpoint so "check
/// for updates" keeps working in that window.
pub fn latest_release() -> Result<Release, UpdateError> {
    match fetch_release(&format!(
        "https://api.github.com/repos/{REPO}/releases/latest"
    )) {
        Ok(release) => Ok(release),
        Err(UpdateError::Net(net::NetError::Status(404))) => newest_published(),
        Err(e) => Err(e),
    }
}

fn api_headers() -> String {
    format!(
        "Accept: application/vnd.github+json\r\nUser-Agent: {UA}\r\n\
         X-GitHub-Api-Version: 2022-11-28\r\n"
    )
}

fn fetch_release(url: &str) -> Result<Release, UpdateError> {
    let res = net::get(url, &api_headers(), TIMEOUT_MS, net::SMALL_BODY)?;
    let text = String::from_utf8_lossy(&res.body);
    let doc = json::parse(&text).map_err(|_| UpdateError::Json)?;
    parse_release(&doc)
}

/// Newest non-draft entry from `/releases` — pre-releases included.
fn newest_published() -> Result<Release, UpdateError> {
    let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=15");
    let res = net::get(&url, &api_headers(), TIMEOUT_MS, net::SMALL_BODY)?;
    let text = String::from_utf8_lossy(&res.body);
    let doc = json::parse(&text).map_err(|_| UpdateError::Json)?;
    let list = doc.as_arr().ok_or(UpdateError::Json)?;

    pick_newest(list).ok_or(UpdateError::NoRelease)
}

/// 从 `/releases` 的数组里挑出最该推荐的那一个。
///
/// 草稿跳过；缺 `.exe`/`.sha256` 的半成品跳过（否则"检查更新"会卡在一个
/// 永远下不动的版本上）；其余按版本号取最大，不按发布时间 —— 补发的旧版本
/// 不该盖住新的。
fn pick_newest(list: &[Json]) -> Option<Release> {
    let mut best: Option<Release> = None;
    for item in list {
        if item.get("draft").and_then(Json::as_bool).unwrap_or(false) {
            continue;
        }
        let Ok(release) = parse_release(item) else {
            continue;
        };
        if best
            .as_ref()
            .is_none_or(|b| release.version.is_newer_than(&b.version))
        {
            best = Some(release);
        }
    }
    best
}

/// 版本号不一定在 `tag_name` 里。
///
/// 滚动预发布的 tag 固定是 `dev`（每次覆盖同一个 release），版本写在标题
/// （`Development build 2.0.0-8-abc1234`）和资产名
/// （`NookSill-2.0.0-8-abc1234-win64-debug.exe`）里。三个来源依次尝试，
/// 资产名排在标题前：它是构建产物自己带的名字，比人手写的标题可靠。
fn version_of(doc: &Json) -> Option<Version> {
    if let Some(tag) = doc.get("tag_name").and_then(Json::as_str) {
        if let Some(v) = version_in(tag) {
            return Some(v);
        }
    }
    if let Some(assets) = doc.get("assets").and_then(Json::as_arr) {
        for item in assets {
            if let Some(name) = item.get("name").and_then(Json::as_str) {
                if let Some(v) = version_in(name) {
                    return Some(v);
                }
            }
        }
    }
    let title = doc.get("name").and_then(Json::as_str)?;
    title.split_whitespace().find_map(version_in)
}

/// 从一段文本里抠出版本号。
///
/// 认三种写法：`v1.0.0`、`1.0.0-8-abc1234`、`NookSill-1.0.0-8-abc1234-win64.exe`。
/// 做法是找到第一个「数字开头」的位置，从那里截到最后一个点分数字/构建号为止，
/// 再把显示用的 `text` 重建成规整形式 —— 资产名里的前后缀不该进界面。
fn version_in(raw: &str) -> Option<Version> {
    let start = raw.find(|c: char| c.is_ascii_digit())?;
    let rest = &raw[start..];
    let mut parsed = Version::parse(rest)?;
    // 只接受「点分三段都在合理范围」的解析结果，避免把 `2026.11.06` 这种
    // 日期或 `2.0.0.1` 之后的垃圾当成版本。
    if parsed.major > 999 || parsed.minor > 999 || parsed.patch > 999 {
        return None;
    }
    parsed.text = match parsed.build {
        Some(build) => format!("{}.{}.{}-{build}", parsed.major, parsed.minor, parsed.patch),
        None => format!("{}.{}.{}", parsed.major, parsed.minor, parsed.patch),
    };
    Some(parsed)
}

/// Both endpoints return the same object shape.
fn parse_release(doc: &Json) -> Result<Release, UpdateError> {
    let tag = doc
        .get("tag_name")
        .and_then(Json::as_str)
        .ok_or(UpdateError::Json)?
        .to_owned();
    let version = version_of(doc).ok_or(UpdateError::Json)?;
    let notes = doc
        .get("body")
        .and_then(Json::as_str)
        .unwrap_or_default()
        .to_owned();
    let prerelease = doc
        .get("prerelease")
        .and_then(Json::as_bool)
        .unwrap_or(false);

    let assets = doc
        .get("assets")
        .and_then(Json::as_arr)
        .ok_or(UpdateError::Json)?;
    let mut exe = None;
    let mut sha256 = None;
    for item in assets {
        let (Some(name), Some(url)) = (
            item.get("name").and_then(Json::as_str),
            item.get("browser_download_url").and_then(Json::as_str),
        ) else {
            continue;
        };
        let asset = Asset {
            name: name.to_owned(),
            url: url.to_owned(),
        };
        if name.ends_with(".exe.sha256") {
            sha256 = Some(asset);
        } else if name.ends_with(".exe") {
            exe = Some(asset);
        }
    }
    Ok(Release {
        tag,
        version,
        notes,
        exe: exe.ok_or(UpdateError::NoAsset)?,
        sha256: sha256.ok_or(UpdateError::NoAsset)?,
        prerelease,
    })
}

/// 这个版本该不该推给当前这份安装？
///
/// 通道规则：dev 构建跟 dev 通道（预发布也提示），正式版只收正式版 ——
/// 否则稳定版用户会被推一个预发布，等于替他把稳定性降级。
#[must_use]
pub fn is_offered(release: &Release, installed: &Version) -> bool {
    let on_dev_channel = installed.build.is_some();
    (on_dev_channel || !release.prerelease) && release.version.is_newer_than(installed)
}

/// Round-trip time for a tiny request to `url`, in milliseconds.
///
/// This measures DNS + TLS + HTTP, which is what the user actually waits for;
/// an ICMP ping would tell us nothing about a reverse proxy in front of a CDN.
#[must_use]
pub fn probe(url: &str) -> Option<u128> {
    let start = Instant::now();
    let headers = format!("User-Agent: {UA}\r\n");
    net::get(url, &headers, PROBE_TIMEOUT_MS, net::SMALL_BODY).ok()?;
    Some(start.elapsed().as_millis())
}

/// DoH (Cloudflare) A-record lookup. `None` when DoH itself is unreachable.
pub fn resolve_doh(host: &str) -> Option<Vec<String>> {
    let url = format!("https://cloudflare-dns.com/dns-query?name={host}&type=A");
    let res = net::get(
        &url,
        "Accept: application/dns-json\r\n",
        PROBE_TIMEOUT_MS,
        net::SMALL_BODY,
    )
    .ok()?;
    let text = String::from_utf8_lossy(&res.body);
    let doc = json::parse(&text).ok()?;
    let answers = doc.get("Answer")?.as_arr()?;
    let ips: Vec<String> = answers
        .iter()
        .filter_map(|a| a.get("data").and_then(Json::as_str))
        .filter(|ip| ip.parse::<std::net::IpAddr>().is_ok())
        .map(str::to_owned)
        .collect();
    (!ips.is_empty()).then_some(ips)
}

/// What the operating system resolver returns for `host`.
#[must_use]
pub fn system_resolve(host: &str) -> Vec<String> {
    use std::net::ToSocketAddrs;
    (host, 443u16)
        .to_socket_addrs()
        .map(|it| it.map(|a| a.ip().to_string()).collect())
        .unwrap_or_default()
}

/// Do the two resolvers disagree completely?
///
/// This is the practical part of "safe DNS": we cannot route WinHTTP's TLS
/// through a DoH answer (SNI/cert validation is bound to the host name), but
/// a total mismatch is the classic signature of DNS poisoning, and the right
/// response is to stop trusting the direct GitHub path and use a mirror.
#[must_use]
pub fn dns_looks_poisoned(host: &str) -> bool {
    let Some(doh) = resolve_doh(host) else {
        return false;
    };
    let system = system_resolve(host);
    !system.is_empty() && system.iter().all(|ip| !doh.contains(ip))
}

/// Pick a download base for `target`.
///
/// Returns the prefix (`""` = GitHub direct). Mirrors are timed in order; the
/// first one under [`FAST_ENOUGH_MS`] wins immediately unless `test_all` is
/// set, in which case every candidate is measured and the fastest wins.
/// Direct GitHub is the last candidate: it is correct but usually the slow one.
#[must_use]
pub fn pick_mirror(target: &str, test_all: bool, allow_direct: bool) -> Option<String> {
    let mut candidates: Vec<&str> = MIRRORS.to_vec();
    if allow_direct {
        candidates.push("");
    }

    let mut best: Option<(u128, &str)> = None;
    for base in candidates {
        let url = mirror_url(base, target);
        let Some(ms) = probe(&url) else {
            continue;
        };
        if !test_all && ms <= FAST_ENOUGH_MS {
            return Some(base.to_owned());
        }
        if best.is_none_or(|(best_ms, _)| ms < best_ms) {
            best = Some((ms, base));
        }
    }
    best.map(|(_, base)| base.to_owned())
}

/// Download a whole asset through `base` (a mirror prefix, `""` = direct).
pub fn download(base: &str, asset: &Asset) -> Result<Vec<u8>, UpdateError> {
    let url = mirror_url(base, &asset.url);
    let headers = format!("User-Agent: {UA}\r\n");
    // 资产是裸 exe：debug 版本能到 35 MB，别用 API 那档上限。
    let res = net::get(&url, headers.as_str(), TIMEOUT_MS, net::LARGE_BODY)?;
    Ok(res.body)
}

/// Lowercase hex SHA-256 of `data`.
#[must_use]
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// First 64-hex-digit token of a `sha256sum` line.
#[must_use]
pub fn parse_sha256_file(text: &str) -> Option<String> {
    let token = text.split_whitespace().next()?;
    let ok = token.len() == 64 && token.chars().all(|c| c.is_ascii_hexdigit());
    ok.then(|| token.to_ascii_lowercase())
}

/// Where a staged update lives: next to the running exe, so the final `move`
/// stays on one volume (and therefore is atomic-ish).
#[must_use]
pub fn staging_path() -> PathBuf {
    let mut path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("nooksill-dock.exe"));
    path.set_file_name("NookSill.update.exe");
    path
}

/// Download + verify. Returns the staged file, ready to be swapped in.
pub fn stage(base: &str, release: &Release) -> Result<PathBuf, UpdateError> {
    let expected = {
        let bytes = download(base, &release.sha256)?;
        let text = String::from_utf8_lossy(&bytes);
        parse_sha256_file(&text).ok_or_else(|| UpdateError::Http("校验文件格式不对".to_owned()))?
    };
    let bytes = download(base, &release.exe)?;
    let actual = sha256_hex(&bytes);
    if actual != expected {
        return Err(UpdateError::Checksum { expected, actual });
    }
    let path = staging_path();
    std::fs::write(&path, &bytes)?;
    Ok(path)
}

/// Swap the staged exe in on next start.
///
/// A running exe cannot overwrite itself on Windows, so a detached `cmd`
/// waits for this process to exit, moves the file and restarts it. The script
/// deletes itself afterwards; if the move fails the old exe is untouched.
pub fn apply_and_restart(staged: &Path) -> Result<(), UpdateError> {
    let current = std::env::current_exe()?;
    let exe_name = current
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("nooksill-dock.exe")
        .to_owned();
    let script = std::env::temp_dir().join("nooksill-update.cmd");
    let body = format!(
        "@echo off\r\n\
         :wait\r\n\
         tasklist /FI \"IMAGENAME eq {exe_name}\" 2>nul | find /I \"{exe_name}\" >nul\r\n\
         if not errorlevel 1 (\r\n\
         \ttimeout /t 1 /nobreak >nul\r\n\
         \tgoto wait\r\n\
         )\r\n\
         move /Y \"{staged}\" \"{current}\" >nul\r\n\
         start \"\" \"{current}\"\r\n\
         del \"%~f0\"\r\n",
        staged = staged.display(),
        current = current.display(),
    );
    std::fs::write(&script, body)?;
    std::process::Command::new("cmd")
        .args(["/C", "start", "", "/min"])
        .arg(&script)
        .spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_and_compare() {
        let rel = |s: &str| Version::parse(s).unwrap();
        assert!(rel("v1.0.1").is_newer_than(&rel("1.0.0")));
        // 稳定版比同号的 dev 新
        assert!(rel("1.0.0").is_newer_than(&rel("1.0.0-65-abc1234")));
        // dev 之间比构建号
        assert!(rel("1.0.0-66-abc1234").is_newer_than(&rel("1.0.0-65-abc1234")));
        assert!(!rel("1.0.0-65-abc1234").is_newer_than(&rel("1.0.0")));
        assert_eq!(rel("1.0.0-65-abc1234").build, Some(65));
        assert_eq!(rel("v1.0.0").build, None);
    }

    /// `/releases` 的回退路径：草稿、缺资产、旧版本都要被正确跳过。
    #[test]
    fn newest_published_skips_drafts_and_broken_entries() {
        let list = json::parse(
            r#"[
              {"tag_name":"v9.9.9","draft":true,"prerelease":false,
               "assets":[{"name":"a.exe","browser_download_url":"u"},{"name":"a.exe.sha256","browser_download_url":"v"}]},
              {"tag_name":"v2.0.0","draft":false,"prerelease":true,
               "assets":[{"name":"b.exe","browser_download_url":"u"},{"name":"b.exe.sha256","browser_download_url":"v"}]},
              {"tag_name":"v1.0.0","draft":false,"prerelease":false,
               "assets":[{"name":"c.exe","browser_download_url":"u"},{"name":"c.exe.sha256","browser_download_url":"v"}]},
              {"tag_name":"v3.0.0","draft":false,"prerelease":false,"assets":[]}
            ]"#,
        )
        .expect("parses");
        let picked = pick_newest(list.as_arr().expect("array")).expect("one winner");
        assert_eq!(picked.version.text, "2.0.0", "草稿与缺资产的都要跳过");
        assert!(picked.prerelease, "选中的是预发布，界面要说清楚");
    }

    /// 真实载荷：滚动预发布的 tag 是 `dev`，版本在标题与资产名里。
    #[test]
    fn version_is_read_from_title_or_asset_when_tag_is_a_channel_name() {
        let doc = json::parse(
            r#"{"tag_name":"dev","name":"Development build 2.0.0-8-00cb41d",
                "draft":false,"prerelease":true,
                "assets":[{"name":"NookSill-2.0.0-8-00cb41d-win64-debug.exe","browser_download_url":"u"},
                          {"name":"NookSill-2.0.0-8-00cb41d-win64-debug.exe.sha256","browser_download_url":"v"}]}"#,
        )
        .expect("parses");
        let release = parse_release(&doc).expect("dev release parses");
        assert_eq!(release.version.major, 2);
        assert_eq!(release.version.build, Some(8));
        assert_eq!(release.version.text, "2.0.0-8", "显示用的文本要干净");
        assert!(release.prerelease);
    }

    #[test]
    fn normal_tag_still_wins() {
        let doc = json::parse(
            r#"{"tag_name":"v1.2.3","name":"NookSill 1.2.3","draft":false,"prerelease":false,
                "assets":[{"name":"NookSill-1.2.3-win64.exe","browser_download_url":"u"},
                          {"name":"NookSill-1.2.3-win64.exe.sha256","browser_download_url":"v"}]}"#,
        )
        .expect("parses");
        let release = parse_release(&doc).expect("release parses");
        assert_eq!(release.version.text, "1.2.3");
        assert_eq!(release.version.build, None);
    }

    #[test]
    fn empty_release_list_reports_no_release() {
        assert!(pick_newest(&[]).is_none());
    }

    #[test]
    fn dev_builds_follow_the_dev_channel_only() {
        let rel = |tag: &str, pre: bool| Release {
            tag: tag.to_owned(),
            version: Version::parse(tag).unwrap(),
            notes: String::new(),
            exe: Asset {
                name: "a.exe".to_owned(),
                url: "u".to_owned(),
            },
            sha256: Asset {
                name: "a.exe.sha256".to_owned(),
                url: "v".to_owned(),
            },
            prerelease: pre,
        };
        let stable = Version::parse("2.0.0").unwrap();
        let dev = Version::parse("2.0.0-9-abc1234").unwrap();

        assert!(!is_offered(&rel("2.0.0", false), &stable), "同名不算更新");
        assert!(is_offered(&rel("2.0.1", false), &stable));
        assert!(
            !is_offered(&rel("2.0.1-9-abc1234", true), &stable),
            "正式版不收预发布"
        );
        assert!(
            is_offered(&rel("2.0.1-9-abc1234", true), &dev),
            "dev 收预发布"
        );
        assert!(is_offered(&rel("2.0.1", false), &dev), "dev 也收正式版");
    }

    #[test]
    fn rate_limit_error_is_readable() {
        let e = UpdateError::Net(net::NetError::Status(403));
        assert!(e.to_string().contains("限流"), "{e}");
    }

    #[test]
    fn mirror_prefix_is_a_plain_concat() {
        assert_eq!(
            mirror_url("https://gh-proxy.com/", "https://github.com/a/b.exe"),
            "https://gh-proxy.com/https://github.com/a/b.exe"
        );
        assert_eq!(
            mirror_url("", "https://github.com/a/b.exe"),
            "https://github.com/a/b.exe"
        );
    }

    #[test]
    fn checksum_file_is_parsed() {
        let text =
            "3f4bce7ebed48898ab1d60f9b31b88623d4d09ef95fe193bbd727b6e9f7714cb  NookSill.exe\n";
        assert_eq!(
            parse_sha256_file(text).as_deref(),
            Some("3f4bce7ebed48898ab1d60f9b31b88623d4d09ef95fe193bbd727b6e9f7714cb")
        );
        assert!(parse_sha256_file("not a hash").is_none());
    }

    #[test]
    fn sha256_matches_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
