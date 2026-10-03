//! Single-instance handoff.
//!
//! The first process listens. A later process sends one line and exits, and the
//! running process opens a tab. Windows uses a per-session named pipe. Linux uses
//! a unix socket under the runtime directory.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub enum Role {
    Forwarded,
    Primary(Receiver<Request>),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Request {
    Home,
    Dir(PathBuf),
    Notice(String),
}

static WAKE: Mutex<Option<Arc<dyn Fn() + Send + Sync>>> = Mutex::new(None);

pub fn bind_wake(wake: impl Fn() + Send + Sync + 'static) {
    if let Ok(mut slot) = WAKE.lock() {
        *slot = Some(Arc::new(wake));
    }
}

pub fn attach(launch: &Result<Option<PathBuf>, String>) -> Role {
    let request = match launch {
        Ok(Some(path)) => Request::Dir(path.clone()),
        Ok(None) => Request::Home,
        Err(message) => Request::Notice(message.clone()),
    };
    claim(request)
}

fn wake() {
    let wake = WAKE.lock().ok().and_then(|slot| slot.clone());
    if let Some(wake) = wake {
        wake();
    }
}

fn encode(request: &Request) -> Vec<u8> {
    match request {
        Request::Home => b"home\n".to_vec(),
        Request::Dir(path) => format!("dir {}\n", path.display()).into_bytes(),
        Request::Notice(message) => {
            let message = message.replace(['\n', '\r'], " ");
            format!("err {message}\n").into_bytes()
        }
    }
}

fn decode(line: &str) -> Option<Request> {
    if line == "home" {
        return Some(Request::Home);
    }
    if let Some(path) = line.strip_prefix("dir ")
        && !path.is_empty()
    {
        return Some(Request::Dir(PathBuf::from(path)));
    }
    if let Some(message) = line.strip_prefix("err ")
        && !message.is_empty()
    {
        return Some(Request::Notice(message.to_owned()));
    }
    None
}

fn read_request(reader: &mut impl BufRead) -> Option<Request> {
    let mut line = String::new();
    if reader.read_line(&mut line).ok()? == 0 {
        return None;
    }
    decode(line.trim_end_matches(['\r', '\n']))
}

fn idle() -> Receiver<Request> {
    mpsc::channel().1
}

fn forward_loop(mut submit: impl FnMut() -> bool) -> bool {
    for _ in 0..20 {
        if submit() {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    false
}

#[cfg(windows)]
fn claim(request: Request) -> Role {
    let name = pipe_name();
    if submit_windows(&name, &request) {
        return Role::Forwarded;
    }
    if !own_mutex() {
        let forwarded = forward_loop(|| submit_windows(&name, &request));
        return if forwarded {
            Role::Forwarded
        } else {
            Role::Primary(idle())
        };
    }
    let (tx, rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel();
    let started = thread::Builder::new()
        .name("ikterm-instance".to_owned())
        .spawn(move || windows_server(name, tx, ready_tx));
    if started.is_err() {
        return Role::Primary(idle());
    }
    let _ = ready_rx.recv_timeout(Duration::from_secs(1));
    Role::Primary(rx)
}

#[cfg(windows)]
fn windows_server(name: String, tx: Sender<Request>, ready: Sender<()>) {
    let Some(mut handle) = create_pipe(&name) else {
        let _ = ready.send(());
        return;
    };
    let _ = ready.send(());
    loop {
        if !wait_client(handle) {
            close_handle(handle);
            handle = match create_pipe(&name) {
                Some(handle) => handle,
                None => break,
            };
            continue;
        }
        let received = {
            use std::os::windows::io::FromRawHandle;
            let mut file =
                unsafe { std::fs::File::from_raw_handle(handle as *mut std::ffi::c_void) };
            let received = read_request(&mut BufReader::new(&mut file));
            drop(file);
            received
        };
        if let Some(request) = received {
            let closed = tx.send(request).is_err();
            wake();
            if closed {
                break;
            }
        }
        handle = match create_pipe(&name) {
            Some(handle) => handle,
            None => break,
        };
    }
}

#[cfg(unix)]
fn claim(request: Request) -> Role {
    let Ok(path) = socket_path() else {
        return Role::Primary(idle());
    };
    match handoff(&path, &request) {
        Handoff::Sent => return Role::Forwarded,
        Handoff::Missing => {
            let _ = std::fs::remove_file(&path);
        }
        Handoff::Failed => {
            let forwarded = forward_loop(|| handoff(&path, &request) == Handoff::Sent);
            return if forwarded {
                Role::Forwarded
            } else {
                Role::Primary(idle())
            };
        }
    }
    match std::os::unix::net::UnixListener::bind(&path) {
        Ok(listener) => {
            let _ = std::fs::set_permissions(&path, unix_mode(0o600));
            let (tx, rx) = mpsc::channel();
            let started = thread::Builder::new()
                .name("ikterm-instance".to_owned())
                .spawn(move || unix_server(listener, tx));
            if started.is_err() {
                return Role::Primary(idle());
            }
            Role::Primary(rx)
        }
        Err(_) => {
            if handoff(&path, &request) == Handoff::Sent {
                Role::Forwarded
            } else {
                Role::Primary(idle())
            }
        }
    }
}

#[cfg(unix)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum Handoff {
    Sent,
    Missing,
    Failed,
}

#[cfg(unix)]
fn handoff(path: &std::path::Path, request: &Request) -> Handoff {
    let mut stream = match std::os::unix::net::UnixStream::connect(path) {
        Ok(stream) => stream,
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                || error.kind() == std::io::ErrorKind::ConnectionRefused =>
        {
            return Handoff::Missing;
        }
        Err(_) => return Handoff::Failed,
    };
    let _ = stream.set_write_timeout(Some(Duration::from_millis(400)));
    if stream.write_all(&encode(request)).is_ok() && stream.flush().is_ok() {
        Handoff::Sent
    } else {
        Handoff::Failed
    }
}

#[cfg(unix)]
fn unix_server(listener: std::os::unix::net::UnixListener, tx: Sender<Request>) {
    for stream in listener.incoming().flatten() {
        let mut stream = stream;
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        if let Some(request) = read_request(&mut BufReader::new(&mut stream)) {
            let closed = tx.send(request).is_err();
            wake();
            if closed {
                break;
            }
        }
    }
}

#[cfg(unix)]
fn socket_path() -> std::io::Result<PathBuf> {
    let dir = match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) => PathBuf::from(dir).join("ikterminal"),
        None => std::env::temp_dir().join(format!("ikterminal-{}", user_id())),
    };
    std::fs::create_dir_all(&dir)?;
    let _ = std::fs::set_permissions(&dir, unix_mode(0o700));
    Ok(dir.join("instance.sock"))
}

#[cfg(unix)]
fn unix_mode(mode: u32) -> std::fs::Permissions {
    use std::os::unix::fs::PermissionsExt;
    std::fs::Permissions::from_mode(mode)
}

#[cfg(unix)]
fn user_id() -> u32 {
    unsafe extern "C" {
        fn getuid() -> u32;
    }
    unsafe { getuid() }
}

#[cfg(windows)]
fn pipe_name() -> String {
    format!(r"\\.\pipe\IkTerminal-{}-{}", session_id(), owner_token())
}

#[cfg(windows)]
fn mutex_name() -> String {
    format!(r"Local\IkTerminal-{}-{}", session_id(), owner_token())
}

#[cfg(windows)]
fn owner_token() -> String {
    std::env::var("USERNAME")
        .unwrap_or_else(|_| "user".to_owned())
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

#[cfg(windows)]
fn session_id() -> u32 {
    unsafe extern "system" {
        fn GetCurrentProcessId() -> u32;
        fn ProcessIdToSessionId(pid: u32, session: *mut u32) -> i32;
    }
    let mut session = 0u32;
    let ok = unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) };
    if ok == 0 { 0 } else { session }
}

#[cfg(windows)]
fn own_mutex() -> bool {
    use std::sync::OnceLock;

    static OWNER: OnceLock<isize> = OnceLock::new();

    unsafe extern "system" {
        fn CreateMutexW(attrs: *const std::ffi::c_void, initial: i32, name: *const u16) -> isize;
        fn GetLastError() -> u32;
    }
    const ERROR_ALREADY_EXISTS: u32 = 183;

    let wide = wide(&mutex_name());
    let handle = unsafe { CreateMutexW(std::ptr::null(), 1, wide.as_ptr()) };
    if handle == 0 {
        return false;
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        close_handle(handle);
        return false;
    }
    if OWNER.set(handle).is_err() {
        close_handle(handle);
        return false;
    }
    OWNER.get().is_some()
}

#[cfg(windows)]
fn submit_windows(name: &str, request: &Request) -> bool {
    let Some(handle) = open_pipe(name) else {
        return false;
    };
    use std::os::windows::io::FromRawHandle;
    let mut file = unsafe { std::fs::File::from_raw_handle(handle as *mut std::ffi::c_void) };
    file.write_all(&encode(request)).is_ok() && file.flush().is_ok()
}

#[cfg(windows)]
fn create_pipe(name: &str) -> Option<isize> {
    unsafe extern "system" {
        fn CreateNamedPipeW(
            name: *const u16,
            open_mode: u32,
            pipe_mode: u32,
            max_instances: u32,
            out_buffer: u32,
            in_buffer: u32,
            timeout: u32,
            security: *const std::ffi::c_void,
        ) -> isize;
    }
    const PIPE_ACCESS_DUPLEX: u32 = 0x0000_0003;
    const INVALID_HANDLE: isize = -1;

    let wide = wide(name);
    let handle = unsafe {
        CreateNamedPipeW(
            wide.as_ptr(),
            PIPE_ACCESS_DUPLEX,
            0,
            1,
            4096,
            4096,
            0,
            std::ptr::null(),
        )
    };
    if handle == 0 || handle == INVALID_HANDLE {
        None
    } else {
        Some(handle)
    }
}

#[cfg(windows)]
fn wait_client(handle: isize) -> bool {
    unsafe extern "system" {
        fn ConnectNamedPipe(pipe: isize, overlapped: *mut std::ffi::c_void) -> i32;
        fn GetLastError() -> u32;
    }
    const ERROR_PIPE_CONNECTED: u32 = 535;
    let connected = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) };
    connected != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED
}

#[cfg(windows)]
fn open_pipe(name: &str) -> Option<isize> {
    unsafe extern "system" {
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *const std::ffi::c_void,
            disposition: u32,
            flags: u32,
            template: isize,
        ) -> isize;
    }
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const OPEN_EXISTING: u32 = 3;
    const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
    const INVALID_HANDLE: isize = -1;

    let wide = wide(name);
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            0,
        )
    };
    if handle == 0 || handle == INVALID_HANDLE {
        None
    } else {
        Some(handle)
    }
}

#[cfg(windows)]
fn close_handle(handle: isize) {
    unsafe extern "system" {
        fn CloseHandle(handle: isize) -> i32;
    }
    unsafe { CloseHandle(handle) };
}

#[cfg(windows)]
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_line_roundtrip() {
        assert_eq!(decode("home"), Some(Request::Home));
        assert_eq!(
            decode("dir C:/work/app"),
            Some(Request::Dir(PathBuf::from("C:/work/app")))
        );
        assert_eq!(
            decode("err missing folder"),
            Some(Request::Notice("missing folder".to_owned()))
        );
        assert_eq!(decode("nope"), None);
        assert_eq!(decode("dir "), None);
        let line = String::from_utf8(encode(&Request::Home)).unwrap();
        assert_eq!(decode(line.trim_end()), Some(Request::Home));
    }
}
