// `claude-pet doctor` — integration & runtime health check.
//
// Prints paths, then checks: is the GUI bridge running and reachable, are pets
// discoverable, and (most importantly) is the Claude Code integration actually
// written into ~/.claude/settings.json — statusLine + the 14 hook events — and
// is that file valid JSON? This is the command to run when "the pet doesn't
// show up".

use serde_json::Value;

use crate::domain::{paths, pets};
use crate::fs::json_file::read_json_strict;

/// The 14 hook events install registers. Kept in sync with install.rs.
const HOOK_EVENTS: &[&str] = &[
    "UserPromptSubmit",
    "PermissionRequest",
    "Notification",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "PostToolBatch",
    "SubagentStart",
    "SubagentStop",
    "TaskCreated",
    "TaskCompleted",
    "Stop",
    "StopFailure",
    "PreCompact",
    "PostCompact",
];

pub fn run() -> anyhow::Result<()> {
    let mut ok = true;

    println!("== paths ==");
    println!("claude-pet home: {}", paths::app_home().display());
    println!("Claude home:    {}", paths::claude_home().display());
    println!("config:         {}", paths::config_path().display());
    println!("state:          {}", paths::state_path().display());
    println!("runtime:        {}", paths::runtime_path().display());
    println!("pet dir:        {}", paths::pet_dir().display());

    println!("\n== runtime ==");
    match read_runtime() {
        Some(r) => {
            println!("  bridge: running (pid {}, port {})", r.pid, r.port);
            match probe_bridge(r.port, &r.token) {
                Ok(true) => println!("  bridge: reachable (/state responded)"),
                Ok(false) => {
                    println!("  bridge: NOT reachable on port {} (process up but no response)", r.port);
                    ok = false;
                }
                Err(_) => {
                    println!("  bridge: reachable check failed");
                    ok = false;
                }
            }
        }
        None => {
            println!("  bridge: NOT running (no runtime.json) — start the GUI with `claude-pet` or tray");
            ok = false;
        }
    }

    println!("\n== pets ==");
    let pet_list = pets::list_pets();
    let ids: Vec<_> = pet_list.iter().map(|p| p.id.clone()).collect();
    if ids.is_empty() {
        println!("  none found in {}", paths::pet_dir().display());
        ok = false;
    } else {
        println!("  {}", ids.join(", "));
    }

    println!("\n== Claude Code integration ==");
    let settings_file = paths::claude_home().join("settings.json");
    match read_json_strict(&settings_file) {
        Ok(Some(settings)) => {
            check_integration(&settings, &settings_file, &mut ok);
        }
        Ok(None) => {
            println!("  settings.json: not found at {}", settings_file.display());
            println!("  → run: claude-pet install --scope user");
            ok = false;
        }
        Err(e) => {
            println!("  settings.json: INVALID — {e}");
            ok = false;
        }
    }

    println!("\n== result ==");
    if ok {
        println!("  all good ✓");
    } else {
        println!("  problems found — see above");
    }
    Ok(())
}

fn check_integration(settings: &Value, file: &std::path::Path, ok: &mut bool) {
    println!("  settings.json: {} (valid JSON)", file.display());

    // statusLine
    let sl = settings.get("statusLine");
    let sl_ok = sl
        .and_then(|s| s.get("command"))
        .and_then(|c| c.as_str())
        .map(|c| c.contains("claude-pet") || c.contains("claudepet"))
        .unwrap_or(false);
    if sl_ok {
        println!("  statusLine: integrated ✓");
    } else {
        println!("  statusLine: NOT integrated (no claude-pet statusLine command)");
        *ok = false;
    }

    // hooks — count which of the 14 events have a claude-pet entry.
    let hooks = settings.get("hooks").and_then(|h| h.as_object());
    let mut present = 0;
    let mut missing: Vec<&str> = Vec::new();
    for ev in HOOK_EVENTS {
        if event_has_ccpet_entry(hooks, ev) {
            present += 1;
        } else {
            missing.push(ev);
        }
    }
    println!("  hooks: {}/{} events integrated", present, HOOK_EVENTS.len());
    if !missing.is_empty() {
        println!("  missing events: {}", missing.join(", "));
        *ok = false;
    }
}

/// Walk the hooks[<event>] array (which may be wrapped in matcher groups:
/// `[{ "matcher": ..., "hooks": [{ "type":"command", "command":"..." }] }]`)
/// and report whether any leaf command mentions claude-pet.
fn event_has_ccpet_entry(hooks: Option<&serde_json::Map<String, Value>>, event: &str) -> bool {
    let Some(arr) = hooks.and_then(|h| h.get(event)).and_then(|v| v.as_array()) else {
        return false;
    };
    for entry in arr {
        // Direct leaf: { type, command }.
        if let Some(cmd) = entry.get("command").and_then(|v| v.as_str()) {
            if cmd.contains("claude-pet") || cmd.contains("claudepet") {
                return true;
            }
        }
        // Wrapped in a matcher group: { matcher, hooks: [...] }.
        if let Some(inner) = entry.get("hooks").and_then(|v| v.as_array()) {
            for h in inner {
                if let Some(cmd) = h.get("command").and_then(|v| v.as_str()) {
                    if cmd.contains("claude-pet") || cmd.contains("claudepet") {
                        return true;
                    }
                }
            }
        }
    }
    false
}

#[derive(serde::Deserialize)]
struct Runtime {
    pid: u32,
    port: u16,
    token: String,
}

fn read_runtime() -> Option<Runtime> {
    let text = std::fs::read_to_string(paths::runtime_path()).ok()?;
    serde_json::from_str(&text).ok()
}

/// Best-effort reachability probe of the bridge's /state endpoint. Uses a
/// blocking reqwest with a short timeout. Returns Ok(true) on HTTP 200.
fn probe_bridge(port: u16, token: &str) -> Result<bool, String> {
    let url = format!("http://127.0.0.1:{port}/state");
    let resp = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_millis(800))
        .build()
        .map_err(|e| e.to_string())?
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .map_err(|e| e.to_string())?;
    Ok(resp.status().as_u16() == 200)
}
