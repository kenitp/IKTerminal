//! OS window frame.
//!
//! The window is created without decorations. winit keeps `WS_CAPTION` and hides the
//! caption from `WM_NCCALCSIZE`, but DWM on current Windows still paints a title bar
//! while that bit is set. The bit is cleared after startup. winit writes it back when
//! it refreshes window styles, so it is cleared again whenever it reappears.

pub fn install(cc: &eframe::CreationContext<'_>) {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};

        if let Ok(handle) = cc.window_handle()
            && let RawWindowHandle::Win32(win) = handle.as_raw()
        {
            clear_caption(win.hwnd.get());
        }
    }
    #[cfg(not(windows))]
    let _ = cc;
}

/// Clears `WS_CAPTION` on every winit window of this thread, including ones split off later.
pub fn enforce() {
    #[cfg(windows)]
    clear_thread_captions();
}

#[cfg(windows)]
fn clear_thread_captions() {
    unsafe extern "system" {
        fn EnumThreadWindows(
            thread_id: u32,
            callback: unsafe extern "system" fn(isize, isize) -> i32,
            param: isize,
        ) -> i32;
        fn GetCurrentThreadId() -> u32;
        fn GetClassNameW(hwnd: isize, class: *mut u16, max: i32) -> i32;
    }

    unsafe extern "system" fn each(hwnd: isize, _: isize) -> i32 {
        let mut buf = [0u16; 64];
        let len = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
        if len > 0 {
            let name = String::from_utf16_lossy(&buf[..len as usize]);
            if name == "Window Class" {
                clear_caption(hwnd);
            }
        }
        1
    }

    unsafe {
        EnumThreadWindows(GetCurrentThreadId(), each, 0);
    }
}

#[cfg(windows)]
fn clear_caption(hwnd: isize) {
    const GWL_STYLE: i32 = -16;
    const WS_CAPTION: i32 = 0x00C0_0000;
    const SWP_NOSIZE: u32 = 0x0001;
    const SWP_NOMOVE: u32 = 0x0002;
    const SWP_NOZORDER: u32 = 0x0004;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const SWP_FRAMECHANGED: u32 = 0x0020;

    unsafe extern "system" {
        fn GetWindowLongW(hwnd: isize, index: i32) -> i32;
        fn SetWindowLongW(hwnd: isize, index: i32, new_long: i32) -> i32;
        fn SetWindowPos(
            hwnd: isize,
            after: isize,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> i32;
    }

    unsafe {
        let style = GetWindowLongW(hwnd, GWL_STYLE);
        if style & WS_CAPTION == 0 {
            return;
        }
        SetWindowLongW(hwnd, GWL_STYLE, style & !WS_CAPTION);
        SetWindowPos(
            hwnd,
            0,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}
