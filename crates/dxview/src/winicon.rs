//! The window's title-bar / taskbar icon on Windows: the exe's own icon resource (build.rs embeds it as id 1),
//! sent to the window with WM_SETICON once it exists (winit leaves the window class without one). PostMessage, never
//! SendMessage: this system runs on a worker thread while the window's thread waits for the frame, so a SendMessage
//! (which waits for the window thread to answer) deadlocks the game at startup.

use bevy::prelude::*;
use bevy::window::{PrimaryWindow, RawHandleWrapper};

#[cfg(windows)]
pub fn window_icon_system(windows: Query<&RawHandleWrapper, With<PrimaryWindow>>, mut done: Local<bool>) {
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, LoadImageW, PostMessageW, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTCOLOR, SM_CXICON, SM_CXSMICON, WM_SETICON,
    };
    if *done {
        return;
    }
    let Some(h) = windows.iter().next() else { return };
    let raw_window_handle::RawWindowHandle::Win32(w) = h.get_window_handle() else {
        *done = true;
        return;
    };
    let hwnd = w.hwnd.get() as windows_sys::Win32::Foundation::HWND;
    // SAFETY: plain Win32 calls on our own window and module; a null icon is simply not sent.
    unsafe {
        let module = GetModuleHandleW(std::ptr::null());
        for (kind, metric) in [(ICON_BIG, SM_CXICON), (ICON_SMALL, SM_CXSMICON)] {
            let size = GetSystemMetrics(metric);
            let icon = LoadImageW(module, 1 as _, IMAGE_ICON, size, size, LR_DEFAULTCOLOR);
            if !icon.is_null() {
                PostMessageW(hwnd, WM_SETICON, kind as usize, icon as isize);
            }
        }
    }
    *done = true;
}

#[cfg(not(windows))]
pub fn window_icon_system() {}
