// Per-session state persistence. Mirrors ClaudePet/src/shared/runtime-state.js.
//
// state.json holds `{ "sessions": { "<sessionId>": <SessionState> } }`. Reads
// deep-merge over a default template so missing fields are filled. Writes are
// atomic (see fs::json_file). Phase 1 covers load/save/resolve/list; rebind
// and prune (phase 2) live here too.

use serde_json::{json, Value};
use std::path::PathBuf;

use crate::domain::config::merge_deep;
use crate::domain::paths::state_path;
use crate::fs::json_file::{read_json, write_json};

pub const DEFAULT_SESSION_ID: &str = "__default__";

pub fn default_session_state() -> Value {
    json!({
        "session": {},
        "cost": {},
        "context": {},
        "tokens": {},
        "git": { "isRepo": false },
        "rateLimits": null,
        "status": {
            "kind": "idle",
            "label": "Claude Code 已就绪",
            "detail": "",
            "severity": "info",
            "attention": false,
            "animation": "idle",
            "updatedAt": ""
        },
        "pendingPermission": null,
        "permissionAutoYes": false,
        "tasks": {},
        "history": [],
        "activeSubagent": null,
        "updatedAt": "",
        "lastEventAt": ""
    })
}

/// Normalize a session id: empty → `__default__`.
pub fn resolve_session_id(value: &str) -> String {
    if value.is_empty() {
        DEFAULT_SESSION_ID.to_string()
    } else {
        value.to_string()
    }
}

fn read_all() -> Value {
    let raw = read_json(&state_path(), json!({}));
    // Legacy flat shape (no `sessions` key) → migrate.
    if raw.get("sessions").is_none() && !raw.is_null() {
        let mut m = serde_json::Map::new();
        m.insert("sessions".into(), json!({ DEFAULT_SESSION_ID: raw }));
        return Value::Object(m);
    }
    raw
}

fn write_all(all: &Value) {
    let _ = write_json(&state_path(), all);
}

/// Load one session's state, merged over the default template.
pub fn load_session_state(session_id: &str) -> Value {
    let id = resolve_session_id(session_id);
    let all = read_all();
    let sessions = all.get("sessions").cloned().unwrap_or(json!({}));
    let current = sessions.get(&id).cloned().unwrap_or(json!({}));
    merge_deep(default_session_state(), current)
}

/// Merge a patch into one session's state and persist. The `status` field, if
/// present in the patch, replaces the current status (and appends to history).
pub fn save_session_state(session_id: &str, patch: &Value) -> Value {
    let id = resolve_session_id(session_id);
    let current = load_session_state(&id);
    let mut next = merge_deep(current, patch.clone());

    // status is replaced, not merged; history appends the new status.
    if let Some(status) = patch.get("status") {
        if let Some(obj) = next.as_object_mut() {
            obj.insert("status".into(), status.clone());
        }
        let history = append_history(next.get("history"), status);
        if let Some(obj) = next.as_object_mut() {
            obj.insert("history".into(), history);
        }
    }
    let now = now_iso();
    if let Some(obj) = next.as_object_mut() {
        obj.insert("updatedAt".into(), json!(now));
        obj.insert("lastEventAt".into(), json!(now));
    }

    let mut all = read_all();
    if all.get("sessions").is_none() {
        all = json!({ "sessions": {} });
    }
    if let Some(sessions) = all.get_mut("sessions").and_then(|v| v.as_object_mut()) {
        sessions.insert(id, next.clone());
    }
    write_all(&all);
    next
}

/// Write a fully-formed session state (no status/history merging). Used when
/// the caller has already resolved the status (e.g. statusline merge).
pub fn save_session_state_raw(session_id: &str, state: &Value) -> Value {
    let id = resolve_session_id(session_id);
    let now = now_iso();
    let mut next = state.clone();
    if let Some(obj) = next.as_object_mut() {
        obj.insert("updatedAt".into(), json!(now));
        obj.insert("lastEventAt".into(), json!(now));
    }
    let mut all = read_all();
    if all.get("sessions").is_none() {
        all = json!({ "sessions": {} });
    }
    if let Some(sessions) = all.get_mut("sessions").and_then(|v| v.as_object_mut()) {
        sessions.insert(id, next.clone());
    }
    write_all(&all);
    next
}

/// Keep at most 40 most-recent status entries.
fn append_history(history: Option<&Value>, status: &Value) -> Value {
    let mut arr: Vec<Value> = history
        .and_then(|h| h.as_array())
        .map(|a| a.iter().take(39).cloned().collect())
        .unwrap_or_default();
    arr.push(status.clone());
    Value::Array(arr)
}

pub fn list_sessions() -> Vec<(String, Value)> {
    let all = read_all();
    let sessions = all.get("sessions").cloned().unwrap_or(json!({}));
    let mut out = Vec::new();
    if let Some(obj) = sessions.as_object() {
        for (k, v) in obj {
            out.push((k.clone(), merge_deep(default_session_state(), v.clone())));
        }
    }
    out
}

pub fn remove_session(session_id: &str) {
    let id = resolve_session_id(session_id);
    let mut all = read_all();
    if let Some(sessions) = all.get_mut("sessions").and_then(|v| v.as_object_mut()) {
        sessions.remove(&id);
    }
    write_all(&all);
}

/// Remove sessions whose `lastEventAt` is older than `max_age_ms`. Returns the
/// removed session ids. Mirrors pruneStaleSessions in runtime-state.js.
pub fn prune_stale_sessions(max_age_ms: u64) -> Vec<String> {
    let now = now_millis();
    let cutoff = now.saturating_sub(max_age_ms as i64);
    let mut all = read_all();
    let sessions = match all.get_mut("sessions").and_then(|v| v.as_object_mut()) {
        Some(s) => s,
        None => return Vec::new(),
    };
    let mut removed = Vec::new();
    let ids: Vec<String> = sessions.keys().cloned().collect();
    for id in ids {
        if id == DEFAULT_SESSION_ID {
            continue;
        }
        let last = sessions
            .get(&id)
            .and_then(|s| s.get("lastEventAt"))
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.timestamp_millis())
            .unwrap_or(0);
        if last < cutoff {
            sessions.remove(&id);
            removed.push(id);
        }
    }
    if !removed.is_empty() {
        write_all(&all);
    }
    removed
}

/// Rebind a session id (used when `/clear` produces a fresh session_id in the
/// same cwd). Migrates the old session's state to the new id and removes the
/// old one. Mirrors rebindSession in main.js (state portion).
pub fn rebind_session(old_id: &str, new_id: &str) {
    if old_id == new_id || old_id.is_empty() || new_id.is_empty() {
        return;
    }
    let mut all = read_all();
    let sessions = match all.get_mut("sessions").and_then(|v| v.as_object_mut()) {
        Some(s) => s,
        None => return,
    };
    if let Some(state) = sessions.remove(old_id) {
        sessions.insert(new_id.to_string(), state);
        write_all(&all);
    }
}

/// Find a rebind candidate: a non-default session with the same cwd, last
/// active within `rebind_window_ms`. Returns the most-recent match.
pub fn find_rebind_candidate(new_session_id: &str, cwd: &str, rebind_window_ms: u64) -> Option<String> {
    if new_session_id.is_empty() || cwd.is_empty() {
        return None;
    }
    let now = now_millis();
    let cutoff = now.saturating_sub(rebind_window_ms as i64);
    let all = read_all();
    let sessions = all.get("sessions")?.as_object()?;
    let mut best: Option<String> = None;
    let mut best_time = 0i64;
    for (id, state) in sessions {
        if id == new_session_id || id == DEFAULT_SESSION_ID {
            continue;
        }
        let state_cwd = state.get("session").and_then(|s| s.get("cwd")).and_then(|v| v.as_str()).unwrap_or("");
        if state_cwd != cwd {
            continue;
        }
        let last = state
            .get("lastEventAt")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.timestamp_millis())
            .unwrap_or(0);
        if last < cutoff {
            continue;
        }
        if last > best_time {
            best_time = last;
            best = Some(id.clone());
        }
    }
    best
}

fn now_millis() -> i64 {
    chrono::Local::now().timestamp_millis()
}

fn now_iso() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false)
}

#[allow(dead_code)]
pub fn state_file() -> PathBuf {
    state_path()
}
