// `claude-pet hook` — Claude Code hooks bridge.
//
// Reads the hook JSON from stdin, derives an incremental status, persists it,
// and forwards the event to the GUI. PermissionRequest blocks for up to 295s
// waiting for the user's decision via the renderer dialog.

use std::io::{Read, Write};

use serde_json::{json, Value};

use crate::cli::bridge_client::{
    build_event, request_permission_decision_with_launch, send_event_with_launch, send_permission_clear,
};
use crate::state::runtime_state::{resolve_session_id, save_session_state};
use crate::state::status::{build_session_meta_from_hook, status_from_hook};

pub fn run() -> anyhow::Result<()> {
    let raw = read_stdin_json()?;
    let event = raw.get("hook_event_name").and_then(|v| v.as_str()).unwrap_or("Hook");
    let session_id = raw
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let resolved = resolve_session_id(&session_id);

    if event != "PermissionRequest" {
        // statusLine is never fired in IDE `--no-chrome` headless mode, so the
        // pet's session/context/tokens/model/cwd would otherwise stay empty.
        // Read the transcript ONCE: its metadata feeds both the session meta
        // (usage/model/context) and the status detail (latest assistant text),
        // avoiding two overlapping tail reads on this hot path.
        let transcript_path = raw.get("transcript_path").and_then(|v| v.as_str()).unwrap_or("");
        let meta = if !transcript_path.is_empty() {
            crate::domain::transcript::read_transcript_meta(std::path::Path::new(transcript_path))
        } else {
            None
        };
        let assistant_text = meta.as_ref().and_then(|m| m.assistant_text.as_deref());
        let status = status_from_hook(&raw, assistant_text);

        let mut patch = build_session_meta_from_hook(&raw, meta.as_ref());
        if let Some(obj) = patch.as_object_mut() {
            obj.insert("status".into(), status.clone());
            obj.insert("pendingPermission".into(), Value::Null);
        }
        save_session_state(&resolved, &patch);
        send_permission_clear(&json!({ "sessionId": resolved }));
        let evt = build_hook_event(&resolved, &raw, &status);
        send_event_with_launch(&evt);
        return Ok(());
    }

    // PermissionRequest: the bridge holds auto-yes state in its long-lived
    // process. If this session is auto-yes, the bridge's permission-request
    // handler returns {action:"allow"} immediately without showing a dialog,
    // so we still go through the normal flow below — the short-circuit happens
    // server-side. No transcript read needed for permission requests.
    let status = status_from_hook(&raw, None);

    // Build the pending permission and request the user's decision.
    let request_id = uuid_v4();
    let pending = build_pending_permission(&raw, &request_id);

    save_session_state(&resolved, &json!({ "status": status, "pendingPermission": pending }));
    let evt = build_event("permission-request", &resolved, &raw, &status);
    send_event_with_launch(&evt);

    // Block on the bridge server's response (up to 295s).
    let decision = request_permission_decision_with_launch(&json!({
        "type": "permission-request",
        "sessionId": resolved,
        "requestId": request_id,
        "raw": raw,
        "status": status,
        "pendingPermission": pending,
    }));

    let action = decision.and_then(|d| d.get("action").and_then(|v| v.as_str()).map(|s| s.to_string()));

    match action.as_deref() {
        Some("auto_yes_session") | Some("auto_yes") => {
            // Bridge already registered this session as auto-yes (via
            // respond_permission). Auto-allow this request too.
            if let Some(o) = build_permission_hook_output(&raw, "allow") {
                stdout_json(&o);
            }
        }
        Some("allow_session") => {
            // Allow this one and write the suggested permissions into the
            // Claude Code session scope so the same tool/pattern won't prompt
            // again this session. Uses Claude Code's native updatedPermissions
            // mechanism (permission_suggestions → destination: "session").
            if let Some(o) = build_permission_hook_output(&raw, "allow_session") {
                stdout_json(&o);
            }
        }
        Some("allow") => {
            if let Some(o) = build_permission_hook_output(&raw, "allow") {
                stdout_json(&o);
            }
        }
        Some("deny") => {
            if let Some(o) = build_permission_hook_output(&raw, "deny") {
                stdout_json(&o);
            }
        }
        _ => {
            // Timeout or error — clear pending permission.
            save_session_state(&resolved, &json!({ "pendingPermission": Value::Null }));
        }
    }

    Ok(())
}

fn build_hook_event(session_id: &str, raw: &Value, status: &Value) -> Value {
    let mut e = build_event("hook", session_id, raw, status);
    if let Some(obj) = e.as_object_mut() {
        obj.insert(
            "receivedAt".into(),
            json!(chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false)),
        );
    }
    e
}

/// Build the pending permission object for the renderer. Mirrors
/// ClaudePet/src/shared/permission-response.js.
fn build_pending_permission(raw: &Value, request_id: &str) -> Value {
    let tool_name = raw.get("tool_name").and_then(|v| v.as_str()).unwrap_or("tool");
    let tool_input = raw.get("tool_input").cloned().unwrap_or(json!({}));
    let desc_text = tool_input.get("command").and_then(|v| v.as_str())
        .or_else(|| tool_input.get("file_path").and_then(|v| v.as_str()))
        .or_else(|| tool_input.get("url").and_then(|v| v.as_str()))
        .or_else(|| tool_input.get("pattern").and_then(|v| v.as_str()))
        .map(|s| s.chars().take(200).collect::<String>())
        .unwrap_or_default();

    // canAutoApprove: whether Claude Code offered session-scope suggestions for
    // this tool. When true the "allow this type for the session" button is shown.
    let can_auto_approve = session_permission_updates(raw).len() > 0;

    let mut m = serde_json::Map::new();
    m.insert("requestId".into(), json!(request_id));
    m.insert("tool".into(), json!(tool_name));
    m.insert("description".into(), json!(desc_text));
    m.insert("sessionId".into(), json!(raw.get("session_id").and_then(|v| v.as_str()).unwrap_or("")));
    m.insert("canAutoApprove".into(), json!(can_auto_approve));
    if let Some(scope) = raw.get("scope") {
        m.insert("scope".into(), scope.clone());
    }
    Value::Object(m)
}

/// Session-scope permission suggestions Claude Code attaches to a
/// PermissionRequest (`permission_suggestions`). Each is cloned with
/// `destination: "session"` so we can hand them back via the hook's
/// `updatedPermissions` to let Claude Code remember them for the session.
/// Mirrors sessionPermissionUpdates in permission-response.js.
fn session_permission_updates(raw: &Value) -> Vec<Value> {
    let suggestions = match raw.get("permission_suggestions") {
        Some(Value::Array(a)) => a,
        _ => return Vec::new(),
    };
    suggestions
        .iter()
        .filter_map(|s| {
            let mut cloned = s.clone();
            if let Some(obj) = cloned.as_object_mut() {
                obj.insert("destination".into(), json!("session"));
            }
            Some(cloned)
        })
        .collect()
}

/// Build the hook JSON output for Claude Code to read the decision. Mirrors
/// buildPermissionHookOutput: emits the `hookSpecificOutput` shape Claude Code
/// expects from a PermissionRequest hook, with `behavior` (allow/deny) and, for
/// `allow_session`, the `updatedPermissions` array (session-scope suggestions).
fn build_permission_hook_output(raw: &Value, action: &str) -> Option<Value> {
    let behavior = if action == "deny" { "deny" } else { "allow" };
    let mut decision = serde_json::Map::new();
    decision.insert("behavior".into(), json!(behavior));
    if action == "allow_session" {
        let updates = session_permission_updates(raw);
        if !updates.is_empty() {
            decision.insert("updatedPermissions".into(), Value::Array(updates));
        }
    }
    Some(json!({
        "hookSpecificOutput": {
            "hookEventName": "PermissionRequest",
            "decision": decision,
        }
    }))
}

fn stdout_json(v: &Value) {
    let s = serde_json::to_string(v).unwrap_or_default();
    let _ = std::io::stdout().write_all(s.as_bytes());
    let _ = std::io::stdout().write_all(b"\n");
}

fn read_stdin_json() -> anyhow::Result<Value> {
    let mut text = String::new();
    let _ = std::io::stdin().read_to_string(&mut text);
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    Ok(serde_json::from_str(&text).unwrap_or(json!({})))
}

fn uuid_v4() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    // Set version 4 and variant bits.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    )
}
