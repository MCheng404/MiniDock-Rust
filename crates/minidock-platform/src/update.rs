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
pub const REPO: &str = "MCheng404/MiniDock-Rust";

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
const UA: &str = "MiniDock-Updater";

#[derive(Debug)]
pub enum UpdateError {
    Net(net::NetError),
    Json,
    Http(String),
    /// The release has no `.exe` asset — packaging changed under us.
    NoAsset,
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
            Self::Net(e) => write!(f, "网络错误：{e}"),
            Self::Json => write!(f, "GitHub 返回的内容无法解析"),
            Self::Http(what) => write!(f, "{what}"),
            Self::NoAsset => write!(f, "该版本没有可用的 exe 资产"),
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

/// Ask GitHub for the newest published release.
pub fn latest_release() -> Result<Release, UpdateError> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let headers = format!(
        "Accept: application/vnd.github+json\r\nUser-Agent: {UA}\r\n\
         X-GitHub-Api-Version: 2022-11-28\r\n"
    );
    let res = net::get(&url, &headers, TIMEOUT_MS)?;
    let text = String::from_utf8_lossy(&res.body);
    let doc = json::parse(&text).map_err(|_| UpdateError::Json)?;

    let tag = doc
        .get("tag_name")
        .and_then(Json::as_str)
        .ok_or(UpdateError::Json)?
        .to_owned();
    let version = Version::parse(&tag).ok_or(UpdateError::Json)?;
    let notes = doc
        .get("body")
        .and_then(Json::as_str)
        .unwrap_or_default()
        .to_owned();

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
    })
}

/// Round-trip time for a tiny request to `url`, in milliseconds.
///
/// This measures DNS + TLS + HTTP, which is what the user actually waits for;
/// an ICMP ping would tell us nothing about a reverse proxy in front of a CDN.
#[must_use]
pub fn probe(url: &str) -> Option<u128> {
    let start = Instant::now();
    let headers = format!("User-Agent: {UA}\r\n");
    net::get(url, &headers, PROBE_TIMEOUT_MS).ok()?;
    Some(start.elapsed().as_millis())
}

/// DoH (Cloudflare) A-record lookup. `None` when DoH itself is unreachable.
pub fn resolve_doh(host: &str) -> Option<Vec<String>> {
    let url = format!("https://cloudflare-dns.com/dns-query?name={host}&type=A");
    let res = net::get(
        &url,
        "Accept: application/dns-json
",
        PROBE_TIMEOUT_MS,
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
    let res = net::get(&url, headers.as_str(), TIMEOUT_MS)?;
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
    let mut path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("minidock-dock.exe"));
    path.set_file_name("MiniDock.update.exe");
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
        .unwrap_or("minidock-dock.exe")
        .to_owned();
    let script = std::env::temp_dir().join("minidock-update.cmd");
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
            "3f4bce7ebed48898ab1d60f9b31b88623d4d09ef95fe193bbd727b6e9f7714cb  MiniDock.exe\n";
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
