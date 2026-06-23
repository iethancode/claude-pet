// `claude-pet install` / `uninstall` — Claude Code integration.
// Mirrors ClaudePet/src/shared/install.js.
//
// Writes the binary path into Claude Code's settings.json as the statusLine
// command and appends a `claude-pet hook` entry to each of 14 hook events.
// Backs up the original file and remembers the legacy statusLine for forwarding.

use serde_json::{json, Map, Value};
use std::path::PathBuf;

use crate::domain::config::{load_config, save_config};
use crate::domain::paths::{build_exe_command, claude_home};
use crate::fs::json_file::{read_json, read_json_strict, write_json};

/// The 14 hook events we integrate with. Value is the matcher (null = no
/// matcher, "" = empty matcher). Mirrors HOOK_EVENTS in install.js.
fn hook_events() -> Vec<(&'static str, Option<&'static str>)> {
    vec![
        ("UserPromptSubmit", None),
        ("PermissionRequest", Some("")),
        ("Notification", Some("permission_prompt|idle_prompt|elicitation_dialog")),
        ("PreToolUse", Some("")),
        ("PostToolUse", Some("")),
        ("PostToolUseFailure", Some("")),
        ("PostToolBatch", None),
        ("SubagentStart", Some("")),
        ("SubagentStop", Some("")),
        ("TaskCreated", None),
        ("TaskCompleted", None),
        ("Stop", None),
        ("StopFailure", Some("")),
        ("PreCompact", Some("")),
        ("PostCompact", Some("")),
    ]
}

fn settings_path_for_scope(scope: &str, cwd: &PathBuf) -> PathBuf {
    match scope {
        "user" => claude_home().join("settings.json"),
        "project" => cwd.join(".claude").join("settings.json"),
        _ => cwd.join(".claude").join("settings.local.json"),
    }
}

fn is_claudepet_command(command: &str) -> bool {
    command.contains("claude-pet") || command.contains("claudepet")
}

fn command_hook(command: &str, event: &str) -> Value {
    if event == "PermissionRequest" {
        json!({ "type": "command", "command": command, "async": false, "timeout": 300 })
    } else {
        json!({ "type": "command", "command": command, "async": true, "timeout": 5 })
    }
}

fn has_ccpet_hook(entry: &Value) -> bool {
    entry
        .get("hooks")
        .and_then(|h| h.as_array())
        .map(|arr| {
            arr.iter().any(|h| {
                h.get("type").and_then(|v| v.as_str()) == Some("command")
                    && h.get("command").and_then(|v| v.as_str()).map(is_claudepet_command).unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn build_hook_entry(event: &str, matcher: Option<&str>, command: &str) -> Value {
    let mut entry = Map::new();
    entry.insert("hooks".into(), json!([command_hook(command, event)]));
    if let Some(m) = matcher {
        entry.insert("matcher".into(), json!(m));
    }
    Value::Object(entry)
}

fn normalize_ccpet_hook(hook: &Value, command: &str, event: &str) -> Value {
    if hook.get("type").and_then(|v| v.as_str()) != Some("command")
        || !hook.get("command").and_then(|v| v.as_str()).map(is_claudepet_command).unwrap_or(false)
    {
        return hook.clone();
    }
    let mut merged = hook.clone();
    if let (Some(dst), Some(src)) = (merged.as_object_mut(), command_hook(command, event).as_object()) {
        for (k, v) in src {
            dst.insert(k.clone(), v.clone());
        }
    }
    merged
}

fn merge_hooks(settings: &Value, hook_command: &str) -> Value {
    let mut hooks = settings.get("hooks").cloned().unwrap_or(json!({}));
    let hooks_obj = hooks.as_object_mut().expect("hooks is object");
    for (event, matcher) in hook_events() {
        let existing = hooks_obj.get(event).and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let mut normalized: Vec<Value> = existing
            .iter()
            .map(|entry| {
                if entry.get("hooks").and_then(|v| v.as_array()).is_some() {
                    let mut e = entry.clone();
                    if let Some(arr) = e.get_mut("hooks").and_then(|v| v.as_array_mut()) {
                        let new_arr: Vec<Value> = arr
                            .iter()
                            .map(|h| normalize_ccpet_hook(h, hook_command, event))
                            .collect();
                        *arr = new_arr;
                    }
                    e
                } else {
                    entry.clone()
                }
            })
            .collect();
        if !normalized.iter().any(has_ccpet_hook) {
            normalized.push(build_hook_entry(event, matcher, hook_command));
        }
        hooks_obj.insert(event.to_string(), Value::Array(normalized));
    }
    hooks
}

fn remove_ccpet_hooks(settings: &Value) -> Value {
    let mut hooks = settings.get("hooks").cloned().unwrap_or(json!({}));
    let hooks_obj = hooks.as_object_mut().expect("hooks is object");
    let events: Vec<String> = hooks_obj.keys().cloned().collect();
    for event in events {
        if let Some(arr) = hooks_obj.get_mut(&event).and_then(|v| v.as_array_mut()) {
            let mut filtered: Vec<Value> = Vec::new();
            for entry in arr.drain(..) {
                let mut e = entry;
                if let Some(inner) = e.get_mut("hooks").and_then(|v| v.as_array_mut()) {
                    inner.retain(|h| {
                        h.get("command").and_then(|v| v.as_str()).map(|c| !is_claudepet_command(c)).unwrap_or(true)
                    });
                    if !inner.is_empty() {
                        filtered.push(e);
                    }
                } else {
                    filtered.push(e);
                }
            }
            if filtered.is_empty() {
                hooks_obj.remove(&event);
            } else {
                hooks_obj.insert(event, Value::Array(filtered));
            }
        }
    }
    hooks
}

fn timestamp() -> String {
    chrono::Local::now()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
        .replace([':', '.'], "-")
}

fn backup_settings(file: &PathBuf, scope: &str) -> Option<PathBuf> {
    if !file.exists() {
        return None;
    }
    let backup = PathBuf::from(format!("{}.claudepet-backup-{}", file.display(), timestamp()));
    let _ = std::fs::copy(file, &backup);
    let mut config = load_config();
    let backups = config
        .get("installBackups")
        .cloned()
        .unwrap_or(json!({}));
    let mut backups_obj = backups.as_object().cloned().unwrap_or_default();
    backups_obj.insert(scope.to_string(), json!(backup.display().to_string()));
    if let Some(obj) = config.as_object_mut() {
        obj.insert("installBackups".into(), Value::Object(backups_obj));
    }
    let _ = save_config(&config);
    Some(backup)
}

fn effective_legacy_statusline(scope: &str, target_settings: &Value) -> Option<Value> {
    if let Some(sl) = target_settings.get("statusLine") {
        if let Some(cmd) = sl.get("command").and_then(|v| v.as_str()) {
            if !is_claudepet_command(cmd) {
                return Some(sl.clone());
            }
        }
    }
    if scope != "user" {
        let user_settings = read_json(&claude_home().join("settings.json"), json!({}));
        if let Some(sl) = user_settings.get("statusLine") {
            if let Some(cmd) = sl.get("command").and_then(|v| v.as_str()) {
                if !is_claudepet_command(cmd) {
                    return Some(sl.clone());
                }
            }
        }
    }
    None
}

pub fn install(scope: &str, preserve_statusline: bool) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let settings_file = settings_path_for_scope(scope, &cwd);
    if let Some(parent) = settings_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let settings = read_json_strict(&settings_file).map_err(anyhow::Error::msg)?.unwrap_or_else(|| json!({}));
    let _backup = backup_settings(&settings_file, scope);

    let statusline_command = build_exe_command(&["statusline"]);
    let hook_command = build_exe_command(&["hook"]);

    if preserve_statusline {
        if let Some(legacy) = effective_legacy_statusline(scope, &settings) {
            let _ = save_config(&json!({ "legacyStatusLine": legacy }));
        }
    }

    let mut next = settings.clone();
    if let Some(obj) = next.as_object_mut() {
        let mut sl = obj
            .get("statusLine")
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        sl.insert("type".into(), json!("command"));
        sl.insert("command".into(), json!(statusline_command.clone()));
        let refresh = sl
            .get("refreshInterval")
            .and_then(|v| v.as_u64())
            .unwrap_or(2);
        sl.insert("refreshInterval".into(), json!(refresh));
        obj.insert("statusLine".into(), Value::Object(sl));
        obj.insert("hooks".into(), merge_hooks(&settings, &hook_command));
    }
    write_json(&settings_file, &next)?;

    println!("claudepet installed for {scope}");
    println!("settings: {}", settings_file.display());
    if let Some(b) = _backup {
        println!("backup: {}", b.display());
    }
    println!("statusLine: {statusline_command}");
    Ok(())
}

pub fn uninstall(scope: &str) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let settings_file = settings_path_for_scope(scope, &cwd);
    let settings = read_json_strict(&settings_file).map_err(anyhow::Error::msg)?.unwrap_or_else(|| json!({}));
    let config = load_config();
    let mut next = settings.clone();

    if let Some(obj) = next.as_object_mut() {
        // Restore legacy statusLine on user scope, else drop ours.
        if let Some(sl) = obj.get("statusLine").cloned() {
            if let Some(cmd) = sl.get("command").and_then(|v| v.as_str()) {
                if is_claudepet_command(cmd) {
                    if scope == "user" {
                        if let Some(legacy) = config.get("legacyStatusLine").cloned() {
                            obj.insert("statusLine".into(), legacy);
                        } else {
                            obj.remove("statusLine");
                        }
                    } else {
                        obj.remove("statusLine");
                    }
                }
            }
        }
    }
    // Compute hooks outside the mutable borrow of `next`.
    let cleaned = remove_ccpet_hooks(&next);
    if let Some(obj) = next.as_object_mut() {
        if cleaned.as_object().map(|m| m.is_empty()).unwrap_or(true) {
            obj.remove("hooks");
        } else {
            obj.insert("hooks".into(), cleaned);
        }
    }
    write_json(&settings_file, &next)?;
    println!("claudepet uninstalled for {scope}");
    println!("settings: {}", settings_file.display());
    Ok(())
}
