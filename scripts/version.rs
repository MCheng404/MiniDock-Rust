// Resolve the version string stamped into binaries and Windows resources.
//
// Included by build scripts (`include!("../../scripts/version.rs")`), so the
// `env!("MINIDOCK_VERSION")` constant, the VERSIONINFO resource and the
// artifact name can never drift apart.
//
// Rules, in order:
//
// | condition | result |
// |---|---|
// | `MINIDOCK_VERSION` set | used verbatim (CI passes the tag / run number) |
// | `PROFILE == release` | `X.Y.Z` — the stable version |
// | otherwise | `X.Y.Z-<build>-<sha>` — stable prefix, build number, commit |
//
// The build number comes from `MINIDOCK_BUILD`, else `git rev-list --count`
// (monotonic across commits), else `0`. The short commit comes from
// `GITHUB_SHA` in CI, else `git rev-parse --short`, else `nogit`.

#[derive(Debug)]
pub struct Version {
    /// Full string, e.g. `0.9.25` or `0.9.25-dev.412`.
    pub text: String,
    /// `major.minor.patch.build`, each clamped to 16 bits (VERSIONINFO limit).
    pub quad: [u16; 4],
}

/// `true` when the build script is running for a release profile.
fn is_release() -> bool {
    std::env::var("PROFILE").is_ok_and(|p| p == "release")
}

fn build_number() -> u64 {
    if let Ok(raw) = std::env::var("MINIDOCK_BUILD") {
        if let Ok(n) = raw.trim().parse() {
            return n;
        }
    }
    let out = std::process::Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .output();
    if let Ok(out) = out {
        if out.status.success() {
            if let Ok(n) = String::from_utf8_lossy(&out.stdout).trim().parse() {
                return n;
            }
        }
    }
    0
}

/// `major.minor.patch` of a version string, ignoring any suffix.
fn triple(text: &str) -> [u16; 3] {
    let core = text.split(['-', '+']).next().unwrap_or(text);
    let mut it = core.split('.').map(|p| p.trim().parse::<u16>().unwrap_or(0));
    [
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
    ]
}

/// Build number is the first dash-separated group after the stable prefix.
fn build_from_suffix(text: &str) -> Option<u64> {
    let (_, rest) = text.split_once('-')?;
    rest.split('-').next()?.parse().ok()
}

/// Short commit hash: `<7 hex>` from `GITHUB_SHA`, else `git rev-parse --short`.
fn short_sha() -> String {
    if let Ok(sha) = std::env::var("GITHUB_SHA") {
        let sha = sha.trim();
        if sha.len() >= 7 {
            return sha[..7].to_owned();
        }
    }
    if let Ok(out) = std::process::Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .output()
    {
        if out.status.success() {
            let sha = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            if !sha.is_empty() {
                return sha;
            }
        }
    }
    "nogit".to_owned()
}

pub fn resolve(pkg_version: &str) -> Version {
    if let Ok(explicit) = std::env::var("MINIDOCK_VERSION") {
        let explicit = explicit.trim().to_owned();
        if !explicit.is_empty() {
            let [a, b, c] = triple(&explicit);
            let d = build_from_suffix(&explicit).unwrap_or(0) as u16;
            return Version {
                text: explicit,
                quad: [a, b, c, d],
            };
        }
    }

    let [a, b, c] = triple(pkg_version);
    if is_release() {
        return Version {
            text: pkg_version.to_owned(),
            quad: [a, b, c, 0],
        };
    }

    // `1.0.0-12-4055a6c`: 稳定前缀 + 第几次构建 + 具体提交号。
    let n = build_number();
    Version {
        text: format!("{pkg_version}-{n}-{}", short_sha()),
        quad: [a, b, c, (n & 0xffff) as u16],
    }
}

/// `major<<48 | minor<<32 | patch<<16 | build`, the layout `winresource` wants.
pub fn packed(v: &Version) -> u64 {
    let [a, b, c, d] = v.quad;
    (u64::from(a) << 48) | (u64::from(b) << 32) | (u64::from(c) << 16) | u64::from(d)
}
