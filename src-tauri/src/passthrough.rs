// Click-through (cursor passthrough) manager.
//
// Problem: Tauri's `set_ignore_cursor_events(true)` makes the whole window
// click-through, but unlike Electron's `{forward:true}` it gives the window no
// way to receive the hover events needed to turn it back off. So a naively
// enabled passthrough makes the pet un-interactable.
//
// Solution: poll the OS-level cursor position on a short interval and toggle
// `set_ignore_cursor_events` per window based on whether the cursor is inside
// that window's interactive hit rectangle:
//   - cursor inside hit rect  → passthrough OFF (window receives clicks)
//   - cursor outside hit rect → passthrough ON  (clicks fall through to desktop)
//
// Each pet window registers its hit rect (in physical screen coords) via
// `update_hit_rect`. The rect covers the visible solids — the status bar and
// the sprite stage — so the large transparent margins stay click-through while
// the pet itself and its panel remain interactive. A window can be force-held
// interactive (e.g. while its context menu is open or while dragging) via
// `hold_interactive`.

use std::collections::HashMap;
use std::time::Duration;

use parking_lot::Mutex;
use tauri::{AppHandle, Manager};

use crate::domain::cursor::cursor_position;

/// Registered hit rect + force-interactive hold count for a window label.
#[derive(Clone, Default)]
struct WindowState {
    /// Hit rectangle in physical screen coordinates (origin = top-left of the
    /// window's outer frame). `None` = not registered yet (treated as fully
    /// passthrough).
    hit_rect: Option<(i32, i32, i32, i32)>, // x, y, w, h (screen coords)
    /// When > 0 the window is held interactive regardless of cursor position.
    hold: u32,
}

static STATES: parking_lot::Mutex<Option<HashMap<String, WindowState>>> = parking_lot::Mutex::new(None);

fn states() -> &'static Mutex<Option<HashMap<String, WindowState>>> {
    &STATES
}

/// Start the global cursor-poll loop. Runs forever on Tauri's async runtime;
/// polls every ~60ms and reconciles each registered window's passthrough state.
pub fn start_poll_loop(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            reconcile(&app);
            tokio::time::sleep(Duration::from_millis(60)).await;
        }
    });
}

/// Update a window's interactive hit rectangle. `x/y/w/h` are in physical
/// pixels, relative to the window's outer top-left (the renderer converts its
/// CSS-px rect using the window's scale factor + outer position).
pub fn update_hit_rect(label: &str, x: i32, y: i32, w: i32, h: i32) {
    let mut guard = states().lock();
    let map = guard.get_or_insert_with(HashMap::new);
    let entry = map.entry(label.to_string()).or_default();
    entry.hit_rect = Some((x, y, w, h));
}

/// Drop a window's registration (on close).
pub fn remove(label: &str) {
    if let Some(map) = states().lock().as_mut() {
        map.remove(label);
    }
}

/// Increment the force-interactive hold for a window (e.g. context menu open,
/// dragging). Returns the new hold count.
pub fn hold_interactive(label: &str) -> u32 {
    let mut guard = states().lock();
    let map = guard.get_or_insert_with(HashMap::new);
    let entry = map.entry(label.to_string()).or_default();
    entry.hold = entry.hold.saturating_add(1);
    entry.hold
}

/// Decrement the force-interactive hold.
pub fn release_interactive(label: &str) {
    let mut guard = states().lock();
    if let Some(map) = guard.as_mut() {
        if let Some(entry) = map.get_mut(label) {
            entry.hold = entry.hold.saturating_sub(1);
        }
    }
}

/// One reconciliation pass: for each registered window, decide whether
/// passthrough should be on or off and apply it.
fn reconcile(app: &AppHandle) {
    let cursor = cursor_position();
    let snapshot: Vec<(String, WindowState)> = {
        let guard = states().lock();
        match guard.as_ref() {
            Some(m) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            None => return,
        }
    };

    for (label, st) in snapshot {
        let Some(win) = app.get_webview_window(&label) else {
            // Window gone — drop its state.
            if let Some(map) = states().lock().as_mut() {
                map.remove(&label);
            }
            continue;
        };

        // Held interactive (menu open / dragging) → always interactive.
        let want_interactive = if st.hold > 0 {
            true
        } else {
            match (cursor, win.outer_position().ok(), st.hit_rect) {
                // Missing cursor / geometry / hit rect → default to passthrough
                // (the transparent window shouldn't block the desktop until the
                // renderer tells us where the solids are).
                (Some(cursor), Some(pos), Some((hx, hy, hw, hh))) => {
                    // hit rect is relative to outer top-left → screen coords.
                    let sx = pos.x + hx;
                    let sy = pos.y + hy;
                    cursor.x >= sx && cursor.x <= sx + hw && cursor.y >= sy && cursor.y <= sy + hh
                }
                _ => false,
            }
        };

        // set_ignore_cursor_events(true) = passthrough ON. We want ignore =
        // !want_interactive. Ignore errors (window may be mid-teardown).
        let _ = win.set_ignore_cursor_events(!want_interactive);
    }
}
