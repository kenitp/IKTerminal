//! Update a release build from the latest GitHub Release on exit.
//!
//! The check and download run off the UI thread. The file is kept only when its
//! SHA-256 matches the digest GitHub published for that asset.

#![cfg_attr(debug_assertions, allow(dead_code))]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
#[cfg(not(debug_assertions))]
use std::thread;

const LATEST: &str = "https://api.github.com/repos/kenitp/IKTerminal/releases/latest";
const CURRENT: &str = env!("CARGO_PKG_VERSION");
const JSON_LIMIT: u64 = 2 * 1024 * 1024;
const ASSET_LIMIT: u64 = 80 * 1024 * 1024;

#[derive(Clone)]
pub struct StagedUpdate {
    pub version: String,
    pub path: PathBuf,
}

pub fn start(wake: impl Fn() + Send + 'static) -> Receiver<StagedUpdate> {
    let (tx, rx) = mpsc::channel();
    #[cfg(debug_assertions)]
    drop((wake, tx));
    #[cfg(not(debug_assertions))]
    {
        let _ = thread::Builder::new()
            .name("ikterm-update".to_owned())
            .spawn(move || {
                if let Some(staged) = fetch()
                    && tx.send(staged).is_ok()
                {
                    wake();
                }
            });
    }
    rx
}

/// Windows starts the installer after this process exits. Linux replaces the running binary.
pub fn apply(staged: &StagedUpdate) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        apply_windows(&staged.path)
    }
    #[cfg(not(windows))]
    {
        apply_linux(&staged.path)
    }
}

fn fetch() -> Option<StagedUpdate> {
    let body = http_get(LATEST, Some("application/vnd.github+json"), JSON_LIMIT)?;
    let json = String::from_utf8(body).ok()?;
    let release = parse_release(&json)?;
    if !is_newer(&release.version, CURRENT) {
        return None;
    }
    let name = asset_name(&release.version);
    let asset = release.assets.iter().find(|asset| asset.name == name)?;
    let bytes = http_get(&asset.url, None, ASSET_LIMIT)?;
    if sha256(&bytes) != asset.sha256 {
        return None;
    }
    let path = stage(&release.version, &bytes).ok()?;
    Some(StagedUpdate {
        version: release.version,
        path,
    })
}

fn stage(version: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("IkTerminal-update-{version}"));
    std::fs::create_dir_all(&dir)?;
    #[cfg(windows)]
    let path = dir.join(asset_name(version));
    #[cfg(not(windows))]
    let path = dir.join("ikterminal");
    #[cfg(windows)]
    let payload = bytes;
    #[cfg(not(windows))]
    let payload = &extract_ikterminal(bytes)?;
    let partial = path.with_extension("partial");
    {
        let mut file = std::fs::File::create(&partial)?;
        file.write_all(payload)?;
        file.sync_all()?;
    }
    let _ = std::fs::remove_file(&path);
    std::fs::rename(&partial, &path)?;
    Ok(path)
}

#[cfg(windows)]
fn apply_windows(installer: &Path) -> std::io::Result<()> {
    let installer = installer
        .canonicalize()
        .unwrap_or_else(|_| installer.to_path_buf());
    let elevate = needs_elevation();
    let verb = if elevate { " -Verb RunAs" } else { "" };
    let command = format!(
        "Wait-Process -Id {} -ErrorAction SilentlyContinue; Start-Process -FilePath {} -ArgumentList '/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART'{verb}",
        std::process::id(),
        ps_quote(&installer.display().to_string()),
    );
    std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-WindowStyle",
            "Hidden",
            "-Command",
            &command,
        ])
        .spawn()?;
    Ok(())
}

#[cfg(windows)]
fn needs_elevation() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let Some(dir) = exe.parent() else {
        return false;
    };
    let probe = dir.join(format!(".ikterminal-update-{}", std::process::id()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = std::fs::remove_file(probe);
            false
        }
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => true,
        Err(_) => false,
    }
}

#[cfg(windows)]
fn ps_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

#[cfg(not(windows))]
fn apply_linux(binary: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let exe = std::env::current_exe()?;
    let dir = exe
        .parent()
        .ok_or_else(|| std::io::Error::other("インストール先が見つかりません"))?;
    let tmp = dir.join(".ikterminal-new");
    std::fs::copy(binary, &tmp)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    std::fs::rename(&tmp, &exe)?;
    Ok(())
}

fn asset_name(version: &str) -> String {
    if cfg!(windows) {
        format!("IkTerminal-{version}-setup.exe")
    } else {
        format!(
            "IkTerminal-{version}-linux-{}.tar.gz",
            std::env::consts::ARCH
        )
    }
}

struct Release {
    version: String,
    assets: Vec<Asset>,
}

struct Asset {
    name: String,
    url: String,
    sha256: [u8; 32],
}

fn parse_release(json: &str) -> Option<Release> {
    let version = depth1_string(json, "tag_name")?;
    let version = version.strip_prefix('v').unwrap_or(&version).to_owned();
    parse_version(&version)?;
    let assets_key = json.find("\"assets\"")?;
    let bracket = json[assets_key..].find('[')? + assets_key;
    let array = bracket_body(&json[bracket..], '[', ']')?;
    let mut assets = Vec::new();
    for object in objects(array) {
        let Some(name) = depth1_string(object, "name") else {
            continue;
        };
        let Some(url) = depth1_string(object, "browser_download_url") else {
            continue;
        };
        let Some(digest) = depth1_string(object, "digest") else {
            continue;
        };
        let Some(sha256) = parse_sha256(&digest) else {
            continue;
        };
        assets.push(Asset { name, url, sha256 });
    }
    Some(Release { version, assets })
}

fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(latest), Some(current)) => latest > current,
        _ => false,
    }
}

fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let text = text.strip_prefix('v').unwrap_or(text);
    let mut parts = text.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((major, minor, patch))
}

fn parse_sha256(digest: &str) -> Option<[u8; 32]> {
    let hex = digest.strip_prefix("sha256:")?;
    if hex.len() != 64 || !hex.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&hash);
    out
}

/// Value of a string field on the object that directly contains it.
fn depth1_string(json: &str, key: &str) -> Option<String> {
    let bytes = json.as_bytes();
    let mut i = 0;
    let mut depth: usize = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let (text, next) = json_string(json, i)?;
                if depth == 1 && text == key {
                    let rest = json[next..].trim_start();
                    let rest = rest.strip_prefix(':')?.trim_start();
                    if !rest.starts_with('"') {
                        return None;
                    }
                    let (value, _) = json_string(rest, 0)?;
                    return Some(value);
                }
                i = next;
            }
            b'{' | b'[' => {
                depth += 1;
                i += 1;
            }
            b'}' | b']' => {
                depth = depth.saturating_sub(1);
                i += 1;
            }
            _ => i += 1,
        }
    }
    None
}

fn json_string(json: &str, start: usize) -> Option<(String, usize)> {
    let bytes = json.as_bytes();
    if bytes.get(start) != Some(&b'"') {
        return None;
    }
    let mut out = String::new();
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return Some((out, i + 1)),
            b'\\' => {
                i += 1;
                match bytes.get(i)? {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let hex = json.get(i + 1..i + 5)?;
                        let code = u32::from_str_radix(hex, 16).ok()?;
                        out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                        i += 4;
                    }
                    _ => return None,
                }
                i += 1;
            }
            byte => {
                out.push(byte as char);
                i += 1;
            }
        }
    }
    None
}

fn bracket_body(json: &str, open: char, close: char) -> Option<&str> {
    let mut depth = 0;
    let mut start = None;
    let mut in_string = false;
    let mut escape = false;
    for (i, ch) in json.char_indices() {
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            c if c == open => {
                if depth == 0 {
                    start = Some(i + c.len_utf8());
                }
                depth += 1;
            }
            c if c == close => {
                if depth == 0 {
                    continue;
                }
                depth -= 1;
                if depth == 0 {
                    return Some(&json[start?..i]);
                }
            }
            _ => {}
        }
    }
    None
}

fn objects(array: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut depth = 0;
    let mut start = None;
    let mut in_string = false;
    let mut escape = false;
    for (i, ch) in array.char_indices() {
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            '}' => {
                if depth == 0 {
                    continue;
                }
                depth -= 1;
                if depth == 0
                    && let Some(from) = start.take()
                {
                    found.push(&array[from..=i]);
                }
            }
            _ => {}
        }
    }
    found
}

fn extract_ikterminal(gzip: &[u8]) -> std::io::Result<Vec<u8>> {
    use flate2::read::GzDecoder;
    let mut plain = Vec::new();
    GzDecoder::new(gzip).read_to_end(&mut plain)?;
    let mut offset = 0;
    while offset + 512 <= plain.len() {
        let header = &plain[offset..offset + 512];
        if header.iter().all(|byte| *byte == 0) {
            break;
        }
        let name = header_name(header);
        let size = header_size(header)?;
        let kind = header[156];
        offset += 512;
        let end = offset + size;
        if end > plain.len() {
            return Err(std::io::Error::other("アーカイブが壊れています"));
        }
        let data = &plain[offset..end];
        offset = end.div_ceil(512) * 512;
        if (kind == b'0' || kind == 0) && (name == "ikterminal" || name.ends_with("/ikterminal")) {
            return Ok(data.to_vec());
        }
    }
    Err(std::io::Error::other("配布物に実行ファイルがありません"))
}

fn header_name(header: &[u8]) -> String {
    let end = header[..100]
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(100);
    String::from_utf8_lossy(&header[..end]).into_owned()
}

fn header_size(header: &[u8]) -> std::io::Result<usize> {
    let text = header[124..136]
        .iter()
        .take_while(|byte| byte.is_ascii_digit() || **byte == b' ')
        .copied()
        .collect::<Vec<_>>();
    let text = String::from_utf8_lossy(&text);
    usize::from_str_radix(text.trim(), 8).map_err(|_| std::io::Error::other("サイズが不正です"))
}

fn http_get(url: &str, accept: Option<&str>, limit: u64) -> Option<Vec<u8>> {
    #[cfg(windows)]
    {
        http_get_windows(url, accept, limit)
    }
    #[cfg(not(windows))]
    {
        http_get_unix(url, accept, limit)
    }
}

#[cfg(windows)]
fn http_get_windows(url: &str, accept: Option<&str>, limit: u64) -> Option<Vec<u8>> {
    let parts = split_url(url)?;
    let agent = wide("IkTerminal");
    let verb = wide("GET");
    let host = wide(&parts.host);
    let path = wide(&parts.path);
    let session =
        Internet(unsafe { WinHttpOpen(agent.as_ptr(), 0, std::ptr::null(), std::ptr::null(), 0) });
    if session.0 == 0 {
        return None;
    }
    unsafe { WinHttpSetTimeouts(session.0, 10_000, 10_000, 15_000, 60_000) };
    let connect = Internet(unsafe { WinHttpConnect(session.0, host.as_ptr(), parts.port, 0) });
    if connect.0 == 0 {
        return None;
    }
    let flags = if parts.https { 0x0080_0000u32 } else { 0 };
    let request = Internet(unsafe {
        WinHttpOpenRequest(
            connect.0,
            verb.as_ptr(),
            path.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            flags,
        )
    });
    if request.0 == 0 {
        return None;
    }
    let mut policy: u32 = 2;
    unsafe {
        WinHttpSetOption(
            request.0,
            88,
            &mut policy as *mut u32 as *mut std::ffi::c_void,
            4,
        );
    }
    if let Some(accept) = accept {
        let header = wide(&format!("Accept: {accept}\r\n"));
        unsafe { WinHttpAddRequestHeaders(request.0, header.as_ptr(), u32::MAX, 0x2000_0000) };
    }
    let sent = unsafe {
        WinHttpSendRequest(
            request.0,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            0,
            0,
            0,
        )
    };
    if sent == 0 || unsafe { WinHttpReceiveResponse(request.0, std::ptr::null_mut()) } == 0 {
        return None;
    }
    let mut status = 0u32;
    let mut status_len = 4u32;
    let mut index = 0u32;
    let queried = unsafe {
        WinHttpQueryHeaders(
            request.0,
            19 | 0x2000_0000,
            std::ptr::null(),
            &mut status as *mut u32 as *mut std::ffi::c_void,
            &mut status_len,
            &mut index,
        )
    };
    if queried == 0 || status != 200 {
        return None;
    }
    let mut body = Vec::new();
    loop {
        let mut available = 0u32;
        if unsafe { WinHttpQueryDataAvailable(request.0, &mut available) } == 0 {
            return None;
        }
        if available == 0 {
            break;
        }
        if body.len() as u64 + u64::from(available) > limit {
            return None;
        }
        let mut chunk = vec![0u8; available as usize];
        let mut read = 0u32;
        if unsafe { WinHttpReadData(request.0, chunk.as_mut_ptr(), available, &mut read) } == 0 {
            return None;
        }
        chunk.truncate(read as usize);
        body.extend_from_slice(&chunk);
    }
    Some(body)
}

#[cfg(windows)]
struct Internet(isize);

#[cfg(windows)]
impl Drop for Internet {
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

#[cfg(windows)]
fn split_url(url: &str) -> Option<UrlParts> {
    let (https, rest) = if let Some(rest) = url.strip_prefix("https://") {
        (true, rest)
    } else {
        (false, url.strip_prefix("http://")?)
    };
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match hostport.split_once(':') {
        Some((host, port)) => (host, port.parse().ok()?),
        None => (hostport, if https { 443 } else { 80 }),
    };
    if host.is_empty() {
        return None;
    }
    Some(UrlParts {
        host: host.to_owned(),
        port,
        path: path.to_owned(),
        https,
    })
}

#[cfg(windows)]
struct UrlParts {
    host: String,
    port: u16,
    path: String,
    https: bool,
}

#[cfg(windows)]
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
#[link(name = "winhttp")]
unsafe extern "system" {
    fn WinHttpOpen(
        agent: *const u16,
        access: u32,
        proxy: *const u16,
        bypass: *const u16,
        flags: u32,
    ) -> isize;
    fn WinHttpSetTimeouts(
        session: isize,
        resolve: i32,
        connect: i32,
        send: i32,
        receive: i32,
    ) -> i32;
    fn WinHttpConnect(session: isize, host: *const u16, port: u16, reserved: u32) -> isize;
    fn WinHttpOpenRequest(
        connect: isize,
        verb: *const u16,
        path: *const u16,
        version: *const u16,
        referrer: *const u16,
        accept: *const u16,
        flags: u32,
    ) -> isize;
    fn WinHttpSetOption(handle: isize, option: u32, buffer: *mut std::ffi::c_void, len: u32)
    -> i32;
    fn WinHttpAddRequestHeaders(
        request: isize,
        headers: *const u16,
        len: u32,
        modifiers: u32,
    ) -> i32;
    fn WinHttpSendRequest(
        request: isize,
        headers: *const u16,
        headers_len: u32,
        optional: *mut std::ffi::c_void,
        optional_len: u32,
        total: u32,
        context: usize,
    ) -> i32;
    fn WinHttpReceiveResponse(request: isize, reserved: *mut std::ffi::c_void) -> i32;
    fn WinHttpQueryHeaders(
        request: isize,
        info: u32,
        name: *const u16,
        buffer: *mut std::ffi::c_void,
        len: *mut u32,
        index: *mut u32,
    ) -> i32;
    fn WinHttpQueryDataAvailable(request: isize, available: *mut u32) -> i32;
    fn WinHttpReadData(request: isize, buffer: *mut u8, to_read: u32, read: *mut u32) -> i32;
    fn WinHttpCloseHandle(handle: isize) -> i32;
}

#[cfg(not(windows))]
fn http_get_unix(url: &str, accept: Option<&str>, limit: u64) -> Option<Vec<u8>> {
    let agent = ureq::AgentBuilder::new()
        .https_only(true)
        .timeout(std::time::Duration::from_secs(30))
        .redirects(8)
        .build();
    let request = agent.get(url).set("User-Agent", "IkTerminal");
    let request = if let Some(accept) = accept {
        request.set("Accept", accept)
    } else {
        request
    };
    let response = request.call().ok()?;
    if response.status() != 200 {
        return None;
    }
    let mut body = Vec::new();
    response
        .into_reader()
        .take(limit + 1)
        .read_to_end(&mut body)
        .ok()?;
    (body.len() as u64 <= limit).then_some(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_version_is_a_greater_triple() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v0.1.1", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("1.0", "0.1.0"));
    }

    #[test]
    fn release_json_selects_the_platform_asset() {
        let json = r#"{"tag_name":"v0.2.0","assets":[{"name":"notes.txt","browser_download_url":"https://example.test/notes.txt"},{"name":"IkTerminal-0.2.0-setup.exe","browser_download_url":"https://example.test/setup.exe","digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","uploader":{"name":"ignored"}},{"name":"IkTerminal-0.2.0-linux-x86_64.tar.gz","browser_download_url":"https://example.test/linux.tar.gz","digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}]}"#;
        let release = parse_release(json).unwrap();
        assert_eq!(release.version, "0.2.0");
        assert_eq!(release.assets.len(), 2);
        assert_eq!(release.assets[0].name, "IkTerminal-0.2.0-setup.exe");
        assert!(parse_release(r#"{"tag_name":"nope","assets":[]}"#).is_none());
    }

    #[test]
    fn archive_contains_the_binary() {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        let payload = b"#!/bin/ikterminal\n";
        let mut tar = Vec::new();
        tar.extend_from_slice(&tar_header("ikterminal", payload.len()));
        tar.extend_from_slice(payload);
        tar.resize(tar.len().div_ceil(512) * 512, 0);
        tar.extend_from_slice(&[0u8; 1024]);
        let mut gzip = GzEncoder::new(Vec::new(), Compression::fast());
        gzip.write_all(&tar).unwrap();
        let compressed = gzip.finish().unwrap();
        assert_eq!(extract_ikterminal(&compressed).unwrap(), payload);
    }

    fn tar_header(name: &str, size: usize) -> [u8; 512] {
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        let mode = b"0000755\0";
        header[100..108].copy_from_slice(mode);
        header[108..116].copy_from_slice(b"0000000\0");
        header[116..124].copy_from_slice(b"0000000\0");
        let size_text = format!("{size:011o}");
        header[124..135].copy_from_slice(size_text.as_bytes());
        header[136..148].copy_from_slice(b"            ");
        header[156] = b'0';
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        let sum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
        let sum_text = format!("{sum:06o}\0 ");
        header[148..156].copy_from_slice(sum_text.as_bytes());
        header
    }
}
