// Global cursor position query for click-through hit-testing.
//
// On Windows we read the OS-level cursor position (screen coords) via
// GetCursorPos. This works even when our window has cursor events ignored
// (set_ignore_cursor_events true), which is exactly the situation we need to
// poll in: the window can't receive pointer events while transparent, so we
// can't rely on the renderer to track the mouse — we must ask the OS.

/// A point in physical screen coordinates (pixels, origin top-left of the
/// virtual screen).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenPoint {
    pub x: i32,
    pub y: i32,
}

/// Current cursor position in physical screen coordinates, or `None` if it
/// could not be read.
pub fn cursor_position() -> Option<ScreenPoint> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
        use windows_sys::Win32::Foundation::POINT;
        let mut pt = POINT { x: 0, y: 0 };
        unsafe {
            if GetCursorPos(&mut pt) != 0 {
                return Some(ScreenPoint { x: pt.x, y: pt.y });
            }
        }
        None
    }
    #[cfg(not(windows))]
    {
        None
    }
}
