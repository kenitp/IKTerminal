//! Open an http(s) URL in the system browser.

pub fn open(url: &str) -> Result<(), String> {
    if !is_http(url) {
        return Err("URL が不正です".to_owned());
    }
    #[cfg(windows)]
    {
        open_windows(url)
    }
    #[cfg(not(windows))]
    {
        open_unix(url)
    }
}

fn is_http(url: &str) -> bool {
    let bytes = url.as_bytes();
    let scheme = if bytes.len() > 8 && bytes[..8].eq_ignore_ascii_case(b"https://") {
        8
    } else if bytes.len() > 7 && bytes[..7].eq_ignore_ascii_case(b"http://") {
        7
    } else {
        return false;
    };
    bytes[scheme..]
        .iter()
        .all(|byte| (0x21..0x7f).contains(byte))
}

#[cfg(windows)]
fn open_windows(url: &str) -> Result<(), String> {
    let op = wide("open");
    let file = wide(url);
    let code = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            op.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        )
    };
    if code > 32 {
        Ok(())
    } else {
        Err("ブラウザを開けません".to_owned())
    }
}

#[cfg(windows)]
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
#[link(name = "shell32")]
unsafe extern "system" {
    fn ShellExecuteW(
        hwnd: *mut std::ffi::c_void,
        operation: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show: i32,
    ) -> isize;
}

#[cfg(not(windows))]
fn open_unix(url: &str) -> Result<(), String> {
    use std::process::{Command, Stdio};

    Command::new("xdg-open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("ブラウザを開けません: {error}"))
}
