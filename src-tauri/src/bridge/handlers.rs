// axum route handlers for the bridge server.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tokio::sync::oneshot;
use tokio::time::timeout;

use super::permissions;
use super::server::{apply_event, emit_update, AppState};
use crate::state::runtime_state::{list_sessions, load_session_state, save_session_state, DEFAULT_SESSION_ID};

/// Build the authenticated router.
pub fn build_router(state: Arc<AppState>, token: String) -> Router {
    Router::new()
        .route("/state", get(get_state))
        .route("/event", post(post_event))
        .route("/permission-request", post(post_permission_request))
        .route("/permission-clear", post(post_permission_clear))
        .layer(axum::middleware::from_fn_with_state(token, auth_middleware))
        .with_state(state)
}

async fn auth_middleware(
    axum::extract::State(token): axum::extract::State<String>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, StatusCode> {
    let auth = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if auth == format!("Bearer {token}") {
        Ok(next.run(req).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

async fn get_state(State(_state): State<Arc<AppState>>) -> impl IntoResponse {
    let sessions: serde_json::Map<String, Value> = list_sessions()
        .into_iter()
        .map(|(id, s)| (id, s))
        .collect();
    let config = crate::domain::config::load_config();
    Json(json!({ "sessions": sessions, "config": config }))
}

async fn post_event(State(state): State<Arc<AppState>>, Json(event): Json<Value>) -> impl IntoResponse {
    apply_event(&state.app, &event);
    StatusCode::NO_CONTENT
}

/// Permission request handler — blocks the CLI HTTP request for up to 295s
/// while waiting for the user's decision via the renderer dialog.
///
/// Flow:
/// 1. Create a oneshot channel, store sender in AppState
/// 2. Show the pending permission status on the pet window + notification
/// 3. Await the oneshot receiver (or timeout after 295s)
/// 4. Return `{action}` to the CLI
async fn post_permission_request(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let session_id = payload
        .get("sessionId")
        .and_then(|v| v.as_str())
        .unwrap_or(DEFAULT_SESSION_ID)
        .to_string();
    let request_id = payload
        .get("requestId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let status = payload.get("status").cloned().unwrap_or(json!({}));
    let pending = payload.get("pendingPermission").cloned().unwrap_or(json!({}));

    // Auto-yes short-circuit: if the user previously picked "auto-yes for this
    // session", allow immediately without showing a dialog or registering a
    // pending request. This state lives in the long-lived bridge process, so it
    // survives across the short-lived `claude-pet hook` CLI invocations.
    if permissions::is_auto_yes(&session_id) {
        return Json(json!({ "action": "allow" }));
    }

    // Save the pending permission state and show it on the pet window.
    save_session_state(&session_id, &json!({ "status": status, "pendingPermission": pending }));
    let saved = load_session_state(&session_id);
    emit_update(&state.app, &session_id, &saved);

    // Create the oneshot channel and register the sender globally.
    let (tx, rx) = oneshot::channel::<Value>();
    permissions::register_pending(session_id.clone(), request_id.clone(), tx);

    // Await the user's decision with a 295s timeout.
    match timeout(std::time::Duration::from_secs(295), rx).await {
        Ok(Ok(response)) => Json(response),
        Ok(Err(_)) | Err(_) => {
            // Timed out or channel closed — return empty action (CLI will auto-deny).
            Json(json!({ "action": "" }))
        }
    }
}

/// Permission clear: remove the pending permission state from the session.
async fn post_permission_clear(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let session_id = payload
        .get("sessionId")
        .and_then(|v| v.as_str())
        .unwrap_or(DEFAULT_SESSION_ID)
        .to_string();
    save_session_state(&session_id, &json!({ "pendingPermission": Value::Null }));
    emit_update(&state.app, &session_id, &load_session_state(&session_id));
    Json(json!({ "ok": true }))
}
