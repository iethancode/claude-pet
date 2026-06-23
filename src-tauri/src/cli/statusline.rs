// `claude-pet statusline` — Claude Code statusLine bridge.
//
// Reads the statusLine JSON from stdin, builds a normalized session state,
// persists it, forwards it to the GUI bridge (auto-launching if needed),
// then prints a one-line fallback to stdout (Claude Code shows it in the bar).

use std::io::Read;

use serde_json::{json, Value};

use crate::cli::bridge_client::{send_event_with_launch, build_event};
use crate::domain::config::load_config;
use crate::state::runtime_state::{resolve_session_id, save_session_state};
use crate::state::status::{build_statusline_state, format_fallback_status_line};

pub fn run() -> anyhow::Result<()> {
    let raw = read_stdin_json()?;
    let state = build_statusline_state(&raw);
    let session_id = raw
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let resolved = resolve_session_id(&session_id);

    // Persist locally so the pet can hydrate even if the bridge is down.
    save_session_state(&resolved, &state);

    // Forward to the GUI (auto-launch on failure).
    let event = build_event("statusline", &resolved, &raw, &state);
    send_event_with_launch(&event);

    // Forward to a legacy statusLine if one was registered at install time.
    let config = load_config();
    let legacy = config
        .get("legacyStatusLine")
        .and_then(|v| v.get("command"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let legacy_output = if !legacy.is_empty() {
        run_legacy_statusline(legacy, &raw)
    } else {
        None
    };

    let line = legacy_output.unwrap_or_else(|| format_fallback_status_line(&state));
    print!("{line}\n");
    Ok(())
}

fn read_stdin_json() -> anyhow::Result<Value> {
    let mut text = String::new();
    let _ = std::io::stdin().read_to_string(&mut text);
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    Ok(serde_json::from_str(&text).unwrap_or(json!({})))
}

/// Run a legacy statusLine command, feeding it the same stdin we received.
/// Mirrors runLegacyStatusLine in cli.js.
fn run_legacy_statusline(command: &str, input: &Value) -> Option<String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let body = input.to_string();
    let mut child = Command::new(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(body.as_bytes());
    }
    let output = child.wait_with_output().ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        None
    }
}
