// Tauri commands exposed to the renderer (replaces Electron's ipcMain.handle).

use serde_json::{json, Value};
use tauri::State;
use tauri_plugin_notification::NotificationExt;

use crate::bridge::BridgeHandle;
use crate::domain::config::load_config;
use crate::domain::pets::list_pets;
use crate::state::runtime_state::{list_sessions, load_session_state, remove_session, save_session_state};

#[tauri::command]
pub fn ping() -> String {
    "claude-pet alive".to_string()
}

/// Initial payload a window requests on load: its session state, config, pets,
/// app version, and the list of active sessions.
#[tauri::command]
pub fn get_initial(app: tauri::AppHandle, window: tauri::Window) -> Value {
    let session_id = window.label().to_string();
    let state = load_session_state(&session_id);
    let config = load_config();
    let pets = list_pets();
    let sessions: serde_json::Map<String, Value> = list_sessions()
        .into_iter()
        .map(|(id, s)| (id, s))
        .collect();
    let version = app.package_info().version.to_string();
    json!({
        "sessionId": session_id,
        "state": state,
        "config": config,
        "pets": pets,
        "appVersion": version,
        "sessions": sessions,
    })
}

/// List available pet manifests (for the picker).
#[tauri::command]
pub fn get_pets() -> Vec<Value> {
    list_pets().into_iter().map(|p| json!(p)).collect()
}

/// Drag the calling window by the given delta (cursor passthrough means the
/// renderer tracks the pointer and we move the window server-side).
#[tauri::command]
pub fn drag_window(window: tauri::Window, dx: i32, dy: i32) -> Result<(), String> {
    let pos = window.outer_position().map_err(|e| e.to_string())?;
    window
        .set_position(tauri::Position::Physical(tauri::PhysicalPosition {
            x: pos.x + dx,
            y: pos.y + dy,
        }))
        .map_err(|e| e.to_string())
}

/// Toggle click-through (cursor events ignored) on the calling window.
/// Kept for ad-hoc use; the cursor-poll loop in `passthrough` is the primary
/// mechanism and will override this on its next tick.
#[tauri::command]
pub fn set_passthrough(window: tauri::Window, ignore: bool) -> Result<(), String> {
    window
        .set_ignore_cursor_events(ignore)
        .map_err(|e| e.to_string())
}

/// Register the interactive hit rectangle for the calling window. The renderer
/// measures the union of its solid elements (status bar + sprite + panel) in
/// CSS pixels, converts to physical pixels via the scale factor, and reports
/// it here relative to the window's top-left. The poll loop enables
/// click-through only outside this rect.
#[tauri::command]
pub fn set_hit_rect(window: tauri::Window, x: f64, y: f64, w: f64, h: f64) -> Result<(), String> {
    let scale = window.scale_factor().unwrap_or(1.0);
    let phys = |v: f64| -> i32 { (v * scale).round() as i32 };
    crate::passthrough::update_hit_rect(window.label(), phys(x), phys(y), phys(w), phys(h));
    Ok(())
}

/// Force the calling window to stay interactive (ignore the poll loop's
/// click-through decision) — used while a context menu is open or while
/// dragging, so the window doesn't go click-through under the cursor.
#[tauri::command]
pub fn hold_interactive(window: tauri::Window) -> Result<(), String> {
    crate::passthrough::hold_interactive(window.label());
    Ok(())
}

/// Release a hold acquired with `hold_interactive`.
#[tauri::command]
pub fn release_interactive(window: tauri::Window) -> Result<(), String> {
    crate::passthrough::release_interactive(window.label());
    Ok(())
}

/// Close the calling session's pet window and drop its persisted state.
/// Mirrors closePetWindow(sessionId, { dropSession: true }) in main.js.
#[tauri::command]
pub fn close_pet(app: tauri::AppHandle, window: tauri::Window) -> Result<(), String> {
    let session_id = window.label().to_string();
    remove_session(&session_id);
    crate::bridge::server::close_pet_window(&app, &session_id);
    Ok(())
}

/// Show a system notification (used for attention states). Phase 3 hooks this
/// to permission/attention events; exposed now for the renderer to call.
#[tauri::command]
pub fn notify(app: tauri::AppHandle, title: String, body: String) -> Result<(), String> {
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|e| e.to_string())
}

/// Switch the pet for the calling session's window. Persists
/// `selectedPets[session_id] = pet_id` and broadcasts the new config so the
/// window re-renders with the chosen pet. Mirrors setSessionPet in main.js.
#[tauri::command]
pub fn set_session_pet(app: tauri::AppHandle, window: tauri::Window, pet_id: String) -> Result<Value, String> {
    use tauri::Emitter;
    let session_id = window.label().to_string();
    if session_id.is_empty() || pet_id.is_empty() {
        return Err("missing session or pet id".into());
    }
    let known = list_pets().iter().any(|p| p.id == pet_id);
    if !known {
        return Err("unknown pet id".into());
    }
    let mut selected_pets = load_config()
        .get("selectedPets")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    selected_pets.insert(session_id.clone(), Value::String(pet_id));
    let config = crate::domain::config::save_config(&json!({ "selectedPets": selected_pets }));
    let _ = app.emit_to(&session_id, "claudepet:config-changed", json!({ "config": config }));
    Ok(config)
}
#[tauri::command]
pub fn bridge_info(state: State<'_, BridgeHandle>) -> Value {
    json!({
        "port": state.runtime.port,
        "pid": state.runtime.pid,
        "startedAt": state.runtime.started_at,
    })
}

/// Respond to a pending permission request from the renderer. Resolves the
/// oneshot the axum handler is waiting on, unblocking the CLI.
#[tauri::command]
pub fn respond_permission(
    session_id: String,
    request_id: String,
    action: String,
) -> Result<(), String> {
    // "auto-yes for this session" registers the session so the bridge
    // auto-allows every subsequent PermissionRequest without a dialog. The
    // current request is still resolved as "allow".
    if action == "auto_yes_session" || action == "auto_yes" {
        crate::bridge::permissions::set_auto_yes(&session_id);
    }
    let ok = crate::bridge::permissions::resolve_pending(&session_id, &request_id, &action);
    if ok {
        // Clear the pending permission from session state.
        let _ = save_session_state(&session_id, &serde_json::json!({ "pendingPermission": Value::Null }));
        Ok(())
    } else {
        Err("no matching pending permission found".into())
    }
}
