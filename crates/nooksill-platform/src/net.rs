//! Minimal HTTP client on WinHTTP.
//!
//! Why not a crate: the app makes three kinds of request (GitHub API, a DoH
//! resolver, release assets) and already links Win32. WinHTTP gives the system
//! proxy, the OS trust store, chunked/redirect handling and HTTP/1.1 for free;
//! `reqwest` would add a TLS stack and ~1 MB to a 7 MB binary.
//!
//! Only `GET` is implemented — every request we make is a download.

use std::fmt;

use windows::core::PCWSTR;
use windows::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable,
    WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
    WinHttpSetTimeouts, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
    WINHTTP_OPEN_REQUEST_FLAGS, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
};

/// Requests are small; 16 MB is a hard stop so a hostile mirror cannot make us
/// allocate without bound.
const MAX_BODY: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub enum NetError {
    /// URL is not `http(s)://host[:port]/path`.
    BadUrl,
    /// WinHTTP refused an operation (DNS, TLS, connect, ...).
    Win32(&'static str),
    /// Server answered with a non-2xx status.
    Status(u32),
    TooLarge,
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadUrl => write!(f, "URL 格式不正确"),
            Self::Win32(what) => write!(f, "{what} 失败"),
            Self::Status(code) => write!(f, "HTTP {code}"),
            Self::TooLarge => write!(f, "响应超过 16 MB"),
        }
    }
}

impl std::error::Error for NetError {}

/// A response we care about: status plus the whole body.
#[derive(Debug)]
pub struct Response {
    pub status: u32,
    pub body: Vec<u8>,
}

/// `GET url`, following redirects (WinHTTP does that by default).
///
/// `headers` is the raw block WinHTTP expects, e.g.
/// `"Accept: application/json\r\nUser-Agent: NookSill\r\n"`.
pub fn get(url: &str, headers: &str, timeout_ms: i32) -> Result<Response, NetError> {
    let parts = split_url(url).ok_or(NetError::BadUrl)?;

    let agent = wide("NookSill/1.0");
    // SAFETY: all handles are checked for null and released by `Handle`.
    let session = unsafe {
        WinHttpOpen(
            PCWSTR(agent.as_ptr()),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        )
    };
    let session = Handle::new(session, "WinHttpOpen")?;
    unsafe {
        let _ = WinHttpSetTimeouts(
            session.raw(),
            timeout_ms,
            timeout_ms,
            timeout_ms,
            timeout_ms,
        );
    }

    let host = wide(parts.host);
    // SAFETY: host is a NUL-terminated wide string.
    let connect = unsafe { WinHttpConnect(session.raw(), PCWSTR(host.as_ptr()), parts.port, 0) };
    let connect = Handle::new(connect, "WinHttpConnect")?;

    let verb = wide("GET");
    let path = wide(parts.path);
    let flags: WINHTTP_OPEN_REQUEST_FLAGS = if parts.tls {
        WINHTTP_FLAG_SECURE
    } else {
        WINHTTP_OPEN_REQUEST_FLAGS(0)
    };
    // SAFETY: verb/path are NUL-terminated; no extra accept types.
    let request = unsafe {
        WinHttpOpenRequest(
            connect.raw(),
            PCWSTR(verb.as_ptr()),
            PCWSTR(path.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            flags,
        )
    };
    let request = Handle::new(request, "WinHttpOpenRequest")?;

    let header_block = wide(headers);
    let header_slice: Option<&[u16]> = if headers.is_empty() {
        None
    } else {
        Some(&header_block[..header_block.len() - 1])
    };
    // SAFETY: no body; lengths describe the header block.
    unsafe { WinHttpSendRequest(request.raw(), header_slice, None, 0, 0, 0) }
        .map_err(|_| NetError::Win32("WinHttpSendRequest"))?;
    // SAFETY: null reserved pointer is documented.
    unsafe { WinHttpReceiveResponse(request.raw(), std::ptr::null_mut()) }
        .map_err(|_| NetError::Win32("WinHttpReceiveResponse"))?;

    let status = status_code(request.raw())?;
    let body = read_body(request.raw())?;
    if !(200..300).contains(&status) {
        return Err(NetError::Status(status));
    }
    Ok(Response { status, body })
}

fn status_code(request: *mut core::ffi::c_void) -> Result<u32, NetError> {
    let mut code: u32 = 0;
    let mut len = std::mem::size_of::<u32>() as u32;
    // SAFETY: buffer is a u32, length is its size in bytes.
    unsafe {
        WinHttpQueryHeaders(
            request,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&raw mut code).cast()),
            &mut len,
            std::ptr::null_mut(),
        )
    }
    .map_err(|_| NetError::Win32("WinHttpQueryHeaders"))?;
    Ok(code)
}

fn read_body(request: *mut core::ffi::c_void) -> Result<Vec<u8>, NetError> {
    let mut body = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let mut available: u32 = 0;
        // SAFETY: out-pointer is a u32.
        unsafe { WinHttpQueryDataAvailable(request, &raw mut available) }
            .map_err(|_| NetError::Win32("WinHttpQueryDataAvailable"))?;
        if available == 0 {
            return Ok(body);
        }
        let want = (available as usize).min(chunk.len());
        let mut read: u32 = 0;
        // SAFETY: buffer has `want` writable bytes.
        unsafe {
            WinHttpReadData(
                request,
                chunk.as_mut_ptr().cast(),
                want as u32,
                &raw mut read,
            )
        }
        .map_err(|_| NetError::Win32("WinHttpReadData"))?;
        if read == 0 {
            return Ok(body);
        }
        body.extend_from_slice(&chunk[..read as usize]);
        if body.len() > MAX_BODY {
            return Err(NetError::TooLarge);
        }
    }
}

/// Closes a WinHTTP handle on drop, so early returns cannot leak.
struct Handle(*mut core::ffi::c_void);

impl Handle {
    fn new(h: *mut core::ffi::c_void, what: &'static str) -> Result<Self, NetError> {
        if h.is_null() {
            Err(NetError::Win32(what))
        } else {
            Ok(Self(h))
        }
    }

    fn raw(&self) -> *mut core::ffi::c_void {
        self.0
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: handle came from WinHttpOpen* and is closed exactly once.
        unsafe {
            let _ = WinHttpCloseHandle(self.0);
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The piece of a URL WinHTTP needs.
struct Parts<'a> {
    host: &'a str,
    port: u16,
    path: &'a str,
    tls: bool,
}

/// `https://host[:port]/path?query` → parts. Fragments are dropped.
fn split_url(url: &str) -> Option<Parts<'_>> {
    let (tls, rest) = url
        .strip_prefix("https://")
        .map(|r| (true, r))
        .or_else(|| url.strip_prefix("http://").map(|r| (false, r)))?;
    let rest = rest.split('#').next()?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    if authority.is_empty() {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => {
            (h, p.parse().ok()?)
        }
        _ => (authority, if tls { 443 } else { 80 }),
    };
    Some(Parts {
        host,
        port,
        path,
        tls,
    })
}

#[cfg(test)]
mod tests {
    use super::split_url;

    #[test]
    fn url_parts_are_split() {
        let p = split_url("https://gh-proxy.com/https://github.com/a/b.exe").unwrap();
        assert_eq!(p.host, "gh-proxy.com");
        assert_eq!(p.port, 443);
        assert!(p.tls);
        assert_eq!(p.path, "/https://github.com/a/b.exe");

        let p = split_url("http://127.0.0.1:8080/x?y=1").unwrap();
        assert_eq!(p.host, "127.0.0.1");
        assert_eq!(p.port, 8080);
        assert!(!p.tls);
        assert_eq!(p.path, "/x?y=1");

        assert!(split_url("ftp://x/y").is_none());
        assert!(split_url("https:///nohost").is_none());
    }
}
