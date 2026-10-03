//! OS window frame.
//!
//! The window is created without decorations. winit keeps `WS_CAPTION` and hides the
//! caption from `WM_NCCALCSIZE`, but DWM on current Windows still paints a title bar
//! while that bit is set. The bit is cleared after startup. winit writes it back when
//! it refreshes window styles, so it is cleared again whenever it reappears.

pub fn install(cc: &eframe::CreationContext<'_>) -> Option<isize> {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};

        if let Ok(handle) = cc.window_handle()
            && let RawWindowHandle::Win32(win) = handle.as_raw()
        {
            let hwnd = win.hwnd.get();
            clear_caption(hwnd);
            return Some(hwnd);
        }
        None
    }
    #[cfg(not(windows))]
    {
        let _ = cc;
        None
    }
}

pub fn enforce(hwnd: Option<isize>) {
    #[cfg(windows)]
    if let Some(hwnd) = hwnd {
        clear_caption(hwnd);
    }
    #[cfg(not(windows))]
    let _ = hwnd;
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
