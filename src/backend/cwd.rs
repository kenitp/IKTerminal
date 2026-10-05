//! Current directory of a local shell process.

use std::path::PathBuf;

/// Directory the process is in. `None` when the process cannot be read.
pub fn of_process(pid: u32) -> Option<PathBuf> {
    if pid == 0 {
        return None;
    }
    read(pid)
        .map(tidy)
        .filter(|path| !path.as_os_str().is_empty())
}

#[cfg(target_os = "linux")]
fn read(pid: u32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

#[cfg(all(unix, not(target_os = "linux")))]
fn read(_pid: u32) -> Option<PathBuf> {
    None
}

#[cfg(windows)]
fn read(pid: u32) -> Option<PathBuf> {
    windows::read(pid)
}

/// Drops a trailing separator. A root (`C:\`, `/`) stays as it is.
fn tidy(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    let trimmed = text.trim_end_matches(['\\', '/']);
    if trimmed.is_empty() || is_drive_root(trimmed) || trimmed.len() == text.len() {
        path
    } else {
        PathBuf::from(trimmed)
    }
}

fn is_drive_root(trimmed: &str) -> bool {
    let bytes = trimmed.as_bytes();
    bytes.len() == 2 && bytes[1] == b':'
}

#[cfg(windows)]
mod windows {
    use std::ffi::{OsString, c_void};
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;

    const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
    const PROCESS_VM_READ: u32 = 0x0010;
    const PROCESS_BASIC_INFORMATION: u32 = 0;
    const MAX_PATH_BYTES: usize = 32 * 1024;

    const PEB_PARAMETERS: usize = if cfg!(target_pointer_width = "64") {
        0x20
    } else {
        0x10
    };
    const PARAMETERS_DIRECTORY: usize = if cfg!(target_pointer_width = "64") {
        0x38
    } else {
        0x24
    };

    #[repr(C)]
    struct ProcessBasicInformation {
        _reserved1: *mut c_void,
        peb: *mut c_void,
        _reserved2: [*mut c_void; 2],
        _unique_process_id: usize,
        _reserved3: *mut c_void,
    }

    #[derive(Clone, Copy)]
    #[repr(C)]
    struct UnicodeString {
        length: u16,
        _maximum_length: u16,
        buffer: *const u16,
    }

    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn CloseHandle(handle: isize) -> i32;
        fn ReadProcessMemory(
            process: *mut c_void,
            address: *const c_void,
            buffer: *mut c_void,
            size: usize,
            read: *mut usize,
        ) -> i32;
    }

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtQueryInformationProcess(
            process: *mut c_void,
            class: u32,
            information: *mut c_void,
            length: u32,
            returned: *mut u32,
        ) -> i32;
    }

    struct Process(*mut c_void);

    impl Drop for Process {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0 as isize);
            }
        }
    }

    pub fn read(pid: u32) -> Option<PathBuf> {
        let process = open(pid)?;
        let info = query_basic(process.0)?;
        if info.peb.is_null() {
            return None;
        }
        let params = read_ptr(process.0, offset(info.peb, PEB_PARAMETERS))?;
        let directory: UnicodeString = read_value(process.0, offset(params, PARAMETERS_DIRECTORY))?;
        let bytes = usize::from(directory.length);
        if directory.buffer.is_null() || bytes == 0 || bytes > MAX_PATH_BYTES || bytes % 2 != 0 {
            return None;
        }
        let mut units = vec![0u16; bytes / 2];
        let raw = unsafe {
            std::slice::from_raw_parts_mut(units.as_mut_ptr().cast::<u8>(), units.len() * 2)
        };
        if !read_memory(process.0, directory.buffer.cast(), raw) {
            return None;
        }
        let end = units
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(units.len());
        Some(PathBuf::from(OsString::from_wide(&units[..end])))
    }

    fn open(pid: u32) -> Option<Process> {
        let handle = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) };
        if handle.is_null() {
            None
        } else {
            Some(Process(handle))
        }
    }

    fn query_basic(process: *mut c_void) -> Option<ProcessBasicInformation> {
        let mut info = std::mem::MaybeUninit::<ProcessBasicInformation>::uninit();
        let mut returned = 0u32;
        let status = unsafe {
            NtQueryInformationProcess(
                process,
                PROCESS_BASIC_INFORMATION,
                info.as_mut_ptr().cast(),
                u32::try_from(size_of::<ProcessBasicInformation>()).unwrap_or(u32::MAX),
                &mut returned,
            )
        };
        if status < 0 {
            None
        } else {
            Some(unsafe { info.assume_init() })
        }
    }

    fn read_ptr(process: *mut c_void, address: *const c_void) -> Option<*mut c_void> {
        let pointer: *mut c_void = read_value(process, address)?;
        if pointer.is_null() {
            None
        } else {
            Some(pointer)
        }
    }

    fn read_value<T: Copy>(process: *mut c_void, address: *const c_void) -> Option<T> {
        let mut value = std::mem::MaybeUninit::<T>::uninit();
        let raw = unsafe {
            std::slice::from_raw_parts_mut(value.as_mut_ptr().cast::<u8>(), size_of::<T>())
        };
        if read_memory(process, address, raw) {
            Some(unsafe { value.assume_init() })
        } else {
            None
        }
    }

    fn read_memory(process: *mut c_void, address: *const c_void, buffer: &mut [u8]) -> bool {
        if address.is_null() || buffer.is_empty() {
            return false;
        }
        let mut read = 0usize;
        unsafe {
            ReadProcessMemory(
                process,
                address,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut read,
            ) != 0
                && read == buffer.len()
        }
    }

    fn offset(base: *const c_void, bytes: usize) -> *const c_void {
        base.cast::<u8>().wrapping_add(bytes).cast()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::tidy;

    #[test]
    fn tidy_keeps_roots_and_drops_a_trailing_separator() {
        assert_eq!(tidy(PathBuf::from("/")), PathBuf::from("/"));
        assert_eq!(tidy(PathBuf::from(r"C:\")), PathBuf::from(r"C:\"));
        assert_eq!(
            tidy(PathBuf::from(r"C:\work\proj\")),
            PathBuf::from(r"C:\work\proj")
        );
        assert_eq!(
            tidy(PathBuf::from("/work/proj/")),
            PathBuf::from("/work/proj")
        );
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn reads_this_process_directory() {
        let got = super::of_process(std::process::id()).expect("process directory");
        let current = std::env::current_dir().expect("current directory");
        assert_eq!(
            std::fs::canonicalize(&got).unwrap_or(got),
            std::fs::canonicalize(&current).unwrap_or(current)
        );
    }
}
