// CLI-side HTTP client. Mirrors ClaudePet/src/shared/bridge-client.js.
//
// Reads runtime.json to discover {port, token}, then POSTs events / permission
// requests with a bearer token. If the GUI isn't running, launches it (spawn
// the same binary with no args) and retries once.

use std::time::Duration;

use serde_json::{json, Value};

use crate::domain::paths::{runtime_path, current_exe_string};

#[derive(Debug)]
struct Runtime {
    port: u16,
    token: String,
}

fn read_runtime() -> Option<Runtime> {
    let text = std::fs::read_to_string(runtime_path()).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    Some(Runtime {
        port: v.get("port")?.as_u64()? as u16,
        token: v.get("token")?.as_str()?.to_string(),
    })
}

fn base_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// Fire-and-forget POST to /event. Auto-launches the GUI on connection failure.
pub fn send_event_with_launch(event: &Value) {
    if send_event(event, Duration::from_millis(500)).is_ok() {
        return;
    }
    if std::env::var("CLAUDEPET_NO_AUTO_LAUNCH").is_ok() {
        return;
    }
    if launch_app() {
        std::thread::sleep(Duration::from_millis(650));
        let _ = send_event(event, Duration::from_millis(700));
    }
}

fn send_event(event: &Value, timeout: Duration) -> Result<(), ()> {
    let Some(rt) = read_runtime() else { return Err(()); };
    let url = format!("{}/event", base_url(rt.port));
    let client = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|_| ())?;
    let resp = client
        .post(&url)
        .bearer_auth(&rt.token)
        .json(event)
        .send()
        .map_err(|_| ())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(())
    }
}

/// POST to /permission-clear (best-effort).
pub fn send_permission_clear(payload: &Value) {
    let Some(rt) = read_runtime() else { return };
    let url = format!("{}/permission-clear", base_url(rt.port));
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
        .ok();
    let Some(client) = client else { return };
    let _ = client.post(&url).bearer_auth(&rt.token).json(payload).send();
}

/// POST to /permission-request and block for the user's decision (≤295s).
/// Auto-launches the GUI if needed.
pub fn request_permission_decision_with_launch(payload: &Value) -> Option<Value> {
    if read_runtime().is_some() {
        return request_permission_decision_inner(payload);
    }
    if std::env::var("CLAUDEPET_NO_AUTO_LAUNCH").is_err() && launch_app() {
        std::thread::sleep(std::time::Duration::from_millis(650));
        request_permission_decision_inner(payload)
    } else {
        None
    }
}

/// POST to /permission-request and block for the user's decision (≤295s).
pub fn request_permission_decision(payload: &Value) -> Option<Value> {
    request_permission_decision_inner(payload)
}

fn request_permission_decision_inner(payload: &Value) -> Option<Value> {
    let rt = read_runtime()?;
    let url = format!("{}/permission-request", base_url(rt.port));
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_millis(295_000))
        .build()
        .ok()?;
    let resp = client
        .post(&url)
        .bearer_auth(&rt.token)
        .json(payload)
        .send()
        .ok()?;
    resp.json::<Value>().ok()
}

/// Launch the GUI binary detached (the same exe with no args).
pub fn launch_app() -> bool {
    let exe = std::env::current_exe().unwrap_or_default();
    let _ = current_exe_string(); // keep linker honest
    std::process::Command::new(&exe)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
}

/// Build the event payload the statusline/hook commands send.
pub fn build_event(event_type: &str, session_id: &str, raw: &Value, state: &Value) -> Value {
    json!({
        "type": event_type,
        "sessionId": session_id,
        "raw": raw,
        "state": state,
        "receivedAt": chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
    })
}
