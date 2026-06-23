// Bridge shared state + server bootstrap.

use std::net::TcpListener;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::domain::config::load_config;
use crate::domain::paths::runtime_path;
use crate::fs::json_file::{ensure_dir, write_json};
use crate::state::runtime_state::{
    find_rebind_candidate, load_session_state, rebind_session, remove_session,
    save_session_state, save_session_state_raw, DEFAULT_SESSION_ID,
};

/// Shared bridge state.
pub struct AppState {
    pub app: AppHandle,
}

/// What we persist to runtime.json for CLI discovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeRuntime {
    pub pid: u32,
    pub port: u16,
    pub token: String,
    #[serde(rename = "startedAt")]
    pub started_at: String,
}

/// Handle to a running bridge: its port/token + a way to shut it down.
pub struct BridgeHandle {
    pub runtime: BridgeRuntime,
    pub shutdown: tokio::sync::watch::Sender<bool>,
}

/// Start the bridge on a random 127.0.0.1 port. Writes runtime.json so CLI
/// processes can discover {port, token}. Runs the axum server in a background
/// tokio task tied to the app's lifetime.
pub fn start_bridge(app: AppHandle) -> BridgeHandle {
    // Bind on std first just to claim a port and write runtime.json before the
    // server is actually accepting (so CLI discovery is fast). The real tokio
    // listener is bound inside the runtime below.
    let probe = TcpListener::bind("127.0.0.1:0").expect("bind bridge port");
    let port = probe.local_addr().expect("local addr").port();
    drop(probe);
    let token = random_token();
    let pid = std::process::id();
    let started_at = chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false);

    let runtime = BridgeRuntime { pid, port, token: token.clone(), started_at };
    let _ = ensure_dir(&runtime_path().parent().unwrap().to_path_buf());
    let _ = write_json(&runtime_path(), &json!(runtime));

    let state = Arc::new(AppState { app: app.clone() });

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let app_clone = app.clone();

    // Dedicated multi-thread tokio runtime: Tauri's async_runtime may not arm
    // an I/O driver for spawned tasks, so accept() would never wake. Owning the
    // runtime here is the reliable fix.
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                log::error!("[claude-pet] bridge: tokio build failed: {e}");
                return;
            }
        };
        rt.block_on(async move {
            let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                Ok(l) => l,
                Err(e) => {
                    log::error!("[claude-pet] bridge: bind failed: {e}");
                    return;
                }
            };
            let router = crate::bridge::handlers::build_router(state, token.clone());
            log::info!("[claude-pet] bridge serving on port {port}");
            match axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let mut rx = shutdown_rx;
                    let _ = rx.wait_for(|v| *v).await;
                })
                .await
            {
                Ok(()) => log::info!("[claude-pet] bridge exited"),
                Err(e) => log::error!("[claude-pet] bridge error: {e}"),
            }
            let _ = std::fs::remove_file(runtime_path());
            let _ = app_clone;
        });
    });
    log::info!("[claude-pet] bridge started on port {port}");

    BridgeHandle { runtime, shutdown: shutdown_tx }
}

fn random_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 24];
    rand::rng().fill_bytes(&mut bytes);
    hex(&bytes)
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

// --- handlers used by the router, exposed here for the handlers module ---

const SESSION_REBIND_WINDOW_MS: u64 = 5 * 60 * 1000;

/// Apply an incoming event: persist state with the correct merge priority,
/// handle session rebind (`/clear`) and SessionEnd, then emit to the window.
pub fn apply_event(app: &AppHandle, event: &Value) {
    let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let raw_session_id = event
        .get("sessionId")
        .and_then(|v| v.as_str())
        .unwrap_or(DEFAULT_SESSION_ID)
        .to_string();

    // SessionEnd hook → close the window and drop the session.
    if event_type == "hook" {
        let hook_event = event.get("raw").and_then(|r| r.get("hook_event_name")).and_then(|v| v.as_str()).unwrap_or("");
        if hook_event == "SessionEnd" {
            // Drop any in-flight permission oneshots + auto-yes state so a
            // reused session id starts clean.
            crate::bridge::permissions::clear_for_session(&raw_session_id);
            close_pet_window(app, &raw_session_id);
            remove_session(&raw_session_id);
            return;
        }
    }

    // Rebind: a fresh session_id in the same cwd within the rebind window
    // takes over the existing session's window/state (Claude Code `/clear`).
    let cwd = event
        .pointer("/state/session/cwd")
        .or_else(|| event.pointer("/raw/cwd"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let session_id = if !cwd.is_empty()
        && raw_session_id != DEFAULT_SESSION_ID
        && app.get_webview_window(&raw_session_id).is_none()
    {
        if let Some(old_id) = find_rebind_candidate(&raw_session_id, cwd, SESSION_REBIND_WINDOW_MS) {
            rebind_window(app, &old_id, &raw_session_id);
            rebind_session(&old_id, &raw_session_id);
            raw_session_id.clone()
        } else {
            raw_session_id.clone()
        }
    } else {
        raw_session_id.clone()
    };

    match event_type {
        "statusline" => {
            if let Some(state) = event.get("state") {
                let saved = apply_statusline_event(&session_id, state);
                emit_update(app, &session_id, &saved);
            }
        }
        "hook" => {
            if let Some(status) = event.get("status") {
                let saved = apply_hook_event(&session_id, status);
                notify_if_attention(app, &session_id, status);
                emit_update(app, &session_id, &saved);
            }
        }
        _ => {}
    }
}

/// statusline: merge event.state over current, but preserve a recent (<30s)
/// hook-driven status so the bar doesn't flip back to "idle" between hooks.
fn apply_statusline_event(session_id: &str, incoming_state: &Value) -> Value {
    let current = load_session_state(session_id);
    let incoming_status = incoming_state.get("status");

    let merged_status = if is_recent_active_status(current.get("status")) {
        let active = current.get("status").cloned().unwrap_or(json!({}));
        if let Some(inc) = incoming_status {
            if inc.get("detail").is_some() {
                let mut m = active.clone();
                if let (Some(dst), Some(src)) = (m.as_object_mut(), inc.as_object()) {
                    if let Some(d) = src.get("detail") {
                        dst.insert("detail".into(), d.clone());
                    }
                }
                m
            } else {
                active
            }
        } else {
            active
        }
    } else {
        incoming_status.cloned().unwrap_or_else(|| current.get("status").cloned().unwrap_or(json!({})))
    };

    // Merge incoming state over current (shallow, like the original), then
    // force the resolved status and keep activeSubagent.
    let mut next = merge_shallow(current, incoming_state);
    if let Some(obj) = next.as_object_mut() {
        obj.insert("status".into(), merged_status);
    }
    save_session_state_raw(session_id, &next)
}

/// hook: replace status, append history, manage activeSubagent.
fn apply_hook_event(session_id: &str, status: &Value) -> Value {
    let current = load_session_state(session_id);
    let mut next = current.clone();

    // activeSubagent lifecycle.
    let kind = status.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    let active_subagent = if kind == "subagent-running" {
        json!({
            "type": status.get("subagentType").and_then(|v| v.as_str()).unwrap_or("agent"),
            "since": status.get("updatedAt").and_then(|v| v.as_str()).unwrap_or(""),
        })
    } else if kind == "subagent-complete" || status.get("subagentEnded").is_some() {
        Value::Null
    } else {
        next.get("activeSubagent").cloned().unwrap_or(Value::Null)
    };

    if let Some(obj) = next.as_object_mut() {
        obj.insert("status".into(), status.clone());
        obj.insert("activeSubagent".into(), active_subagent);
    }
    // history append + status replace + timestamps handled by save_session_state.
    let patch = json!({ "status": status, "activeSubagent": next.get("activeSubagent").cloned().unwrap_or(Value::Null) });
    save_session_state(session_id, &patch)
}

fn is_recent_active_status(status: Option<&Value>) -> bool {
    let Some(s) = status else { return false };
    let kind = s.get("kind").and_then(|v| v.as_str()).unwrap_or("idle");
    if kind == "idle" {
        return false;
    }
    let updated = s
        .get("updatedAt")
        .and_then(|v| v.as_str())
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(0);
    let now = chrono::Local::now().timestamp_millis();
    now - updated < 30_000
}

fn merge_shallow(mut base: Value, over: &Value) -> Value {
    if let (Some(dst), Some(src)) = (base.as_object_mut(), over.as_object()) {
        for (k, v) in src {
            dst.insert(k.clone(), v.clone());
        }
    }
    base
}

/// Send a system notification if the hook status has `attention: true` and
/// enough time has passed since the last notification for this session.
fn notify_if_attention(app: &AppHandle, session_id: &str, status: &Value) {
    let attention = status.get("attention").and_then(|v| v.as_bool()).unwrap_or(false);
    if !attention {
        return;
    }
    // Check if we recently notified for this session to avoid spam.
    let label = status.get("label").and_then(|v| v.as_str()).unwrap_or("Claude needs attention");
    let detail = status.get("detail").and_then(|v| v.as_str()).unwrap_or("");
    let _ = app.emit_to(session_id, "claudepet:attention", json!({ "sessionId": session_id, "label": label, "detail": detail }));
    // Use Tauri notification plugin.
    if let Err(e) = tauri_plugin_notification::NotificationExt::notification(app)
        .builder()
        .title(label.to_string())
        .body(detail.to_string())
        .show()
    {
        log::warn!("notification failed: {e}");
    }
}

/// Emit a `claudepet:update` payload to the pet window for this session.
pub fn emit_update(app: &AppHandle, session_id: &str, state: &Value) {
    let config = load_config();
    let payload = json!({
        "sessionId": session_id,
        "state": state,
        "config": config,
    });
    // Target the pet window whose label == session id; fall back to default.
    let label = if session_id.is_empty() { DEFAULT_SESSION_ID } else { session_id };
    ensure_pet_window(app, label);
    if app.get_webview_window(label).is_some() {
        let _ = app.emit_to(label, "claudepet:update", payload);
    }
}

/// Create the pet window for a session on demand (one window per session).
/// No-op if it already exists. Mirrors createPetWindow in main.js.
pub fn ensure_pet_window(app: &AppHandle, session_id: &str) {
    if app.get_webview_window(session_id).is_some() {
        return;
    }

    let url = format!("index.html?view=pet&session={}", urlencoding(session_id));
    let mut builder = WebviewWindowBuilder::new(app, session_id.to_string(), WebviewUrl::App(url.into()))
        .title("ClaudePet")
        .inner_size(438.0, 338.0)
        .decorations(false)
        .transparent(true)
        .resizable(true)
        .fullscreen(false)
        .skip_taskbar(true)
        .shadow(false)
        .visible(false)
        .always_on_top(true)
        .focused(false);

    // Position: bottom-right of the primary monitor, staggered per session.
    if let Ok(Some(monitor)) = app.primary_monitor() {
        let work = monitor.size();
        let scale = monitor.scale_factor();
        let offset = 36.0 * (count_pet_windows(app) as f64);
        let x = work.width as f64 / scale - 460.0 + offset;
        let y = work.height as f64 / scale - 360.0 + offset;
        builder = builder.position(x, y);
    }

    match builder.build() {
        Ok(win) => {
            // Show without stealing focus.
            let _ = win.show();
        }
        Err(e) => {
            log::warn!("failed to create pet window for {session_id}: {e}");
        }
    }
}

fn count_pet_windows(app: &AppHandle) -> usize {
    app.webview_windows()
        .iter()
        .filter(|(label, _)| label.as_str() != DEFAULT_SESSION_ID)
        .count()
}

/// Migrate a pet window from `old_id` to `new_id` (relabel). Tauri can't
/// relabel an existing window, so we close the old one and let ensure_pet_window
/// recreate it under the new id on the next emit. State migration is done
/// separately in runtime_state::rebind_session.
fn rebind_window(app: &AppHandle, old_id: &str, new_id: &str) {
    if old_id == new_id {
        return;
    }
    if let Some(win) = app.get_webview_window(old_id) {
        // Remember position so the new window reappears in the same spot.
        if let Ok(pos) = win.outer_position() {
            let mut config = load_config();
            if let Some(positions) = config.get_mut("positions").and_then(|v| v.as_object_mut()) {
                positions.insert(new_id.to_string(), json!([pos.x, pos.y]));
            }
            let _ = crate::domain::config::save_config(&json!({ "positions": config.get("positions").cloned().unwrap_or(json!({})) }));
        }
        let _ = win.close();
    }
}

/// Close and destroy a session's pet window (SessionEnd / prune).
pub fn close_pet_window(app: &AppHandle, session_id: &str) {
    if let Some(win) = app.get_webview_window(session_id) {
        let _ = win.destroy();
    }
}

fn urlencoding(s: &str) -> String {
    s.replace('%', "%25").replace('&', "%26").replace('=', "%3D").replace('?', "%3F")
}
