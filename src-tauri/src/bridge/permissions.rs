// Global pending-permission registry. Shared between the axum bridge thread
// (handlers awaiting oneshot receivers) and the Tauri command thread
// (renderer invoking respond_permission).
//
// Also tracks per-session "auto-yes" mode: once the user picks "auto-yes for
// this session", every subsequent PermissionRequest for that session is
// auto-allowed without showing a dialog. This state lives in the long-lived
// bridge process (NOT the short-lived `claude-pet hook` CLI process, whose
// in-memory set would be lost on every event).

use std::collections::HashSet;
use std::sync::OnceLock;

use parking_lot::Mutex;
use serde_json::Value;
use tokio::sync::oneshot;

/// A pending permission request waiting for the user's decision.
pub struct PendingPermission {
    pub session_id: String,
    pub request_id: String,
    pub tx: oneshot::Sender<Value>,
}

static PENDING: OnceLock<Mutex<Vec<PendingPermission>>> = OnceLock::new();
static AUTO_YES: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn pending() -> &'static Mutex<Vec<PendingPermission>> {
    PENDING.get_or_init(|| Mutex::new(Vec::new()))
}

fn auto_yes() -> &'static Mutex<HashSet<String>> {
    AUTO_YES.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Whether this session is in auto-yes mode (every permission auto-allowed).
pub fn is_auto_yes(session_id: &str) -> bool {
    auto_yes().lock().contains(session_id)
}

/// Mark a session as auto-yes (user picked "auto-yes for this session").
pub fn set_auto_yes(session_id: &str) {
    auto_yes().lock().insert(session_id.to_string());
}

/// Register a pending permission oneshot sender.
pub fn register_pending(session_id: String, request_id: String, tx: oneshot::Sender<Value>) {
    pending().lock().push(PendingPermission { session_id, request_id, tx });
}

/// Resolve a pending permission by finding and removing the matching sender
/// for `(session_id, request_id)`. Returns `true` if the sender was found and
/// the action was sent.
pub fn resolve_pending(session_id: &str, request_id: &str, action: &str) -> bool {
    let mut perms = pending().lock();
    let idx = perms.iter().position(|p| p.session_id == session_id && p.request_id == request_id);
    match idx {
        Some(i) => {
            let perm = perms.remove(i);
            let response = serde_json::json!({ "action": action });
            perm.tx.send(response).is_ok()
        }
        None => false,
    }
}

/// Remove all pending permissions for a given session (used on disconnect/clear
/// /SessionEnd). Also clears auto-yes mode so a reused session id starts fresh.
pub fn clear_for_session(session_id: &str) {
    pending().lock().retain(|p| p.session_id != session_id);
    auto_yes().lock().remove(session_id);
}

