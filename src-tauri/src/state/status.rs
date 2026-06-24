// Status construction from Claude Code inputs. Mirrors ClaudePet/src/shared/state.js.
//
// `build_statusline_state` turns the JSON Claude Code pipes to the statusLine
// command into a normalized session snapshot (session/cost/context/tokens/git/
// status). `status_from_hook` (phase 2) turns hook payloads into incremental
// status updates.

use serde_json::{json, Value};
use std::path::Path;

use crate::domain::git::get_git_info;
use crate::domain::transcript::{read_latest_assistant_text, read_transcript_meta, TranscriptMeta};

fn basename(value: &str) -> String {
    Path::new(value)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn shorten_command(command: &str) -> String {
    let text = command.trim();
    if text.is_empty() {
        return String::new();
    }
    let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let stripped = regex_strip_cd(&collapsed);
    if stripped.chars().count() > 70 {
        let cut: String = stripped.chars().take(67).collect();
        format!("{cut}...")
    } else {
        stripped
    }
}

fn regex_strip_cd(s: &str) -> String {
    let lower = s.to_string();
    if let Some(rest) = lower.strip_prefix("cd ") {
        let mut iter = rest.splitn(2, char::is_whitespace);
        let _dir = iter.next();
        if let Some(rem) = iter.next() {
            let rem = rem.trim_start();
            for sep in ["&&", ";"] {
                if let Some(after) = rem.strip_prefix(sep) {
                    return after.trim_start().to_string();
                }
            }
        }
    }
    s.to_string()
}

fn shorten_path(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    let name = basename(value);
    if name.chars().count() > 60 {
        let cut: String = name.chars().take(57).collect();
        format!("{cut}...")
    } else {
        name
    }
}

fn shorten_host(url: &str) -> String {
    if let Ok(parsed) = url::Url::parse(url) {
        if let Some(host) = parsed.host_str() {
            return host.to_string();
        }
    }
    url.chars().take(60).collect()
}

fn shorten_text(value: &str, max: usize) -> String {
    let text = value.trim();
    if text.is_empty() {
        return String::new();
    }
    let first_line = text.split(['\r', '\n']).next().unwrap_or("").trim();
    let first_line = if first_line.is_empty() { text } else { first_line };
    let collapsed: String = first_line.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > max {
        let cut: String = collapsed.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}...")
    } else {
        collapsed
    }
}

fn shorten_multiline(value: &str, max: usize) -> String {
    let text = value.trim();
    if text.is_empty() {
        return String::new();
    }
    let normalized = text.replace("\r\n", "\n");
    let normalized: String = normalized
        .lines()
        .map(|l| {
            let trimmed = l.trim_end_matches([' ', '\t']);
            trimmed.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    if normalized.chars().count() > max {
        let cut: String = normalized.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}...")
    } else {
        normalized
    }
}

/// Describe a tool invocation for bubble display. Returns {tool, target, summary}.
pub fn describe_tool(input: &Value) -> Value {
    let raw_tool = input.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
    let tool_input = input.get("tool_input").cloned().unwrap_or(json!({}));
    let tool = if raw_tool.is_empty() { "tool" } else { raw_tool };

    let result = match raw_tool {
        "Bash" | "PowerShell" => {
            let command = tool_input.get("command").and_then(|v| v.as_str()).map(shorten_command).unwrap_or_default();
            json!({ "tool": raw_tool, "target": command, "summary": if command.is_empty() { "Running shell command".into() } else { format!("Running: {command}") } })
        }
        "Edit" | "MultiEdit" => {
            let target = tool_input.get("file_path").and_then(|v| v.as_str()).map(shorten_path).unwrap_or_default();
            json!({ "tool": raw_tool, "target": target, "summary": if target.is_empty() { "Editing file".into() } else { format!("Editing {target}") } })
        }
        "Write" => {
            let target = tool_input.get("file_path").and_then(|v| v.as_str()).map(shorten_path).unwrap_or_default();
            json!({ "tool": raw_tool, "target": target, "summary": if target.is_empty() { "Writing file".into() } else { format!("Writing {target}") } })
        }
        "Read" => {
            let target = tool_input.get("file_path").and_then(|v| v.as_str()).map(shorten_path).unwrap_or_default();
            json!({ "tool": raw_tool, "target": target, "summary": if target.is_empty() { "Reading file".into() } else { format!("Reading {target}") } })
        }
        "Grep" => {
            let pattern = tool_input.get("pattern").and_then(|v| v.as_str()).map(|s| shorten_text(s, 60)).unwrap_or_default();
            json!({ "tool": raw_tool, "target": pattern, "summary": if pattern.is_empty() { "Searching code".into() } else { format!("Searching for \"{pattern}\"") } })
        }
        "Glob" => {
            let pattern = tool_input.get("pattern").and_then(|v| v.as_str()).map(|s| shorten_text(s, 60)).unwrap_or_default();
            json!({ "tool": raw_tool, "target": pattern, "summary": if pattern.is_empty() { "Finding files".into() } else { format!("Finding {pattern}") } })
        }
        "Task" => {
            let target = tool_input.get("subagent_type").and_then(|v| v.as_str()).unwrap_or("subagent").to_string();
            json!({ "tool": raw_tool, "target": target, "summary": format!("Delegating to {target}") })
        }
        "WebFetch" => {
            let target = tool_input.get("url").and_then(|v| v.as_str()).map(shorten_host).unwrap_or_default();
            json!({ "tool": raw_tool, "target": target, "summary": if target.is_empty() { "Fetching URL".into() } else { format!("Fetching {target}") } })
        }
        "WebSearch" => {
            let target = tool_input.get("query").and_then(|v| v.as_str()).map(|s| shorten_text(s, 80)).unwrap_or_default();
            json!({ "tool": raw_tool, "target": target, "summary": if target.is_empty() { "Web search".into() } else { format!("Web search: {target}") } })
        }
        "TodoWrite" => json!({ "tool": raw_tool, "target": "", "summary": "Updating task list" }),
        "NotebookEdit" => {
            let target = tool_input.get("notebook_path").and_then(|v| v.as_str()).map(shorten_path).unwrap_or_default();
            json!({ "tool": raw_tool, "target": target, "summary": if target.is_empty() { "Editing notebook".into() } else { format!("Editing notebook {target}") } })
        }
        "" => json!({ "tool": "", "target": "", "summary": "" }),
        _ => {
            let raw = ["command", "file_path", "path", "url", "pattern"]
                .iter()
                .find_map(|k| tool_input.get(*k).and_then(|v| v.as_str()))
                .unwrap_or("");
            if raw.is_empty() {
                json!({ "tool": tool, "target": "", "summary": format!("Using {tool}") })
            } else if tool_input.get("url").and_then(|v| v.as_str()).is_some() {
                let t = shorten_host(raw);
                json!({ "tool": tool, "target": t, "summary": format!("{tool}: {t}") })
            } else if tool_input.get("command").and_then(|v| v.as_str()).is_some() {
                let t = shorten_command(raw);
                json!({ "tool": tool, "target": t, "summary": format!("{tool}: {t}") })
            } else {
                let t = shorten_path(raw);
                json!({ "tool": tool, "target": t, "summary": format!("{tool}: {t}") })
            }
        }
    };
    result
}

fn summarize_output(input: &Value) -> String {
    for key in [
        "last_assistant_message",
        "message",
        "output",
        "result",
        "tool_output",
        "tool_response",
        "error_details",
        "error",
    ] {
        if let Some(v) = input.get(key) {
            if let Some(s) = v.as_str() {
                let text = shorten_text(s, 180);
                if !text.is_empty() {
                    return text;
                }
            }
        }
    }
    String::new()
}

fn now_iso() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false)
}

/// Build a full session state snapshot from a statusLine input payload.
pub fn build_statusline_state(input: &Value) -> Value {
    let workspace = input.get("workspace").cloned().unwrap_or(json!({}));
    let cwd = workspace
        .get("current_dir")
        .and_then(|v| v.as_str())
        .or_else(|| input.get("cwd").and_then(|v| v.as_str()))
        .map(|s| s.to_string())
        .unwrap_or_else(|| std::env::current_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_default());
    let context = input.get("context_window").cloned().unwrap_or(json!({}));
    let current_usage = context.get("current_usage").cloned().unwrap_or(json!({}));
    let transcript_path = input.get("transcript_path").and_then(|v| v.as_str()).unwrap_or("");
    // Single tail read + parse: yields cumulative usage (input/output/
    // cache_creation summed, cache_read latest-only to avoid wild overcounting)
    // AND the latest assistant text — previously two separate overlapping reads.
    let transcript_meta = if !transcript_path.is_empty() {
        read_transcript_meta(Path::new(transcript_path))
    } else {
        None
    };
    let transcript_usage = transcript_meta.as_ref().map(|m| &m.usage);
    let live_assistant = transcript_meta
        .as_ref()
        .and_then(|m| m.assistant_text.as_ref())
        .map(|t| shorten_multiline(t, 600))
        .unwrap_or_default();

    let used_percentage = context.get("used_percentage").and_then(|v| v.as_f64());
    let remaining_percentage = used_percentage.map(|p| 100.0 - p);

    let model = input.get("model").cloned().unwrap_or(json!({}));
    let cost = input.get("cost").cloned().unwrap_or(json!({}));

    let session = json!({
        "id": input.get("session_id").and_then(|v| v.as_str()).unwrap_or(""),
        "name": input.get("session_name").and_then(|v| v.as_str()).unwrap_or(""),
        "cwd": cwd.clone(),
        "projectDir": workspace.get("project_dir").and_then(|v| v.as_str()).unwrap_or(&cwd),
        "cwdName": basename(&cwd),
        "transcriptPath": transcript_path,
        "version": input.get("version").and_then(|v| v.as_str()).unwrap_or(""),
        "model": model,
    });

    let tokens = if let Some(tu) = &transcript_usage {
        json!({
            "liveInput": context.get("total_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
            "liveOutput": context.get("total_output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
            "sessionInput": tu.input + tu.cache_creation + tu.cache_read,
            "sessionOutput": tu.output,
            "sessionCacheCreation": tu.cache_creation,
            "sessionCacheRead": tu.cache_read,
        })
    } else {
        json!({
            "liveInput": context.get("total_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
            "liveOutput": context.get("total_output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
            "sessionInput": 0,
            "sessionOutput": 0,
            "sessionCacheCreation": 0,
            "sessionCacheRead": 0,
        })
    };

    json!({
        "source": "statusline",
        "updatedAt": now_iso(),
        "session": session,
        "cost": {
            "totalCostUsd": cost.get("total_cost_usd").and_then(|v| v.as_f64()).unwrap_or(0.0),
            "totalDurationMs": cost.get("total_duration_ms").and_then(|v| v.as_u64()).unwrap_or(0),
            "totalApiDurationMs": cost.get("total_api_duration_ms").and_then(|v| v.as_u64()).unwrap_or(0),
            "linesAdded": cost.get("total_lines_added").and_then(|v| v.as_u64()).unwrap_or(0),
            "linesRemoved": cost.get("total_lines_removed").and_then(|v| v.as_u64()).unwrap_or(0),
        },
        "context": {
            "usedPercentage": used_percentage,
            "remainingPercentage": remaining_percentage,
            "size": context.get("context_window_size").and_then(|v| v.as_u64()).unwrap_or(0),
            "totalInput": context.get("total_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
            "totalOutput": context.get("total_output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
            "current": {
                "input": current_usage.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                "output": current_usage.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                "cacheCreation": current_usage.get("cache_creation_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                "cacheRead": current_usage.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
            },
            "exceeds200k": input.get("exceeds_200k_tokens").and_then(|v| v.as_bool()).unwrap_or(false),
        },
        "tokens": tokens,
        "git": get_git_info(Path::new(&cwd)),
        "rateLimits": input.get("rate_limits").cloned().unwrap_or(Value::Null),
        "status": {
            "kind": "idle",
            "label": "Claude Code is ready",
            "detail": live_assistant,
            "severity": "info",
            "attention": false,
            "animation": "idle",
            "updatedAt": now_iso(),
        }
    })
}

/// Approximate context window size used when statusLine is unavailable (IDE
/// `--no-chrome` headless mode). Claude Code only reports the precise
/// `context_window_size` via statusLine; 200k is the standard window.
const FALLBACK_CONTEXT_WINDOW: u64 = 200_000;

/// Build session/tokens/context metadata from a hook payload + transcript.
///
/// statusLine is never fired in IDE `--no-chrome` headless mode, so the pet's
/// session/context/tokens/model/cwd would otherwise stay empty. This derives
/// them from the hook payload (session_id, cwd, transcript_path) and the
/// transcript itself (cumulative usage, latest model, current context size).
/// The returned object merges cleanly into session state alongside `status`.
///
/// `meta` is the transcript metadata already parsed by the caller; the hook
/// CLI process reads the transcript once and shares it with `status_from_hook`
/// so this hot path does a single tail read per event.
pub fn build_session_meta_from_hook(raw: &Value, meta: Option<&TranscriptMeta>) -> Value {
    let session_id = raw.get("session_id").and_then(|v| v.as_str()).unwrap_or("");
    let transcript_path = raw.get("transcript_path").and_then(|v| v.as_str()).unwrap_or("");
    let raw_cwd = raw.get("cwd").and_then(|v| v.as_str()).unwrap_or("");

    // cwd: hook payload first, transcript's most-recent cwd as fallback.
    let cwd = if !raw_cwd.is_empty() {
        raw_cwd.to_string()
    } else {
        meta.and_then(|m| m.cwd.clone())
            .unwrap_or_else(|| {
                std::env::current_dir()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default()
            })
    };

    let model_value = meta
        .and_then(|m| m.model.clone())
        .map(|id| json!({ "id": id, "display_name": id }))
        .unwrap_or(json!({}));

    let usage = meta.map(|m| &m.usage);
    // sessionInput = cumulative real new tokens spent (input + cache_creation);
    // cache_read is the latest context re-read, shown separately as sessionCacheRead.
    let (s_in, s_out, s_cc, s_cr) = match usage {
        Some(u) => (u.input + u.cache_creation, u.output, u.cache_creation, u.cache_read),
        None => (0u64, 0u64, 0u64, 0u64),
    };
    // Last assistant record's raw counts (non-cumulative) — mirrors Claude
    // Code's `context_window.current_usage`. Used for "current" / "total" fields
    // so they reflect the most recent API call, not session-wide totals.
    let (last_in, last_out) = meta
        .as_ref()
        .map(|m| (m.last_input, m.last_output))
        .unwrap_or((0u64, 0u64));
    let context_tokens = meta.as_ref().map(|m| m.context_tokens).unwrap_or(0);
    // No clamp: show the real occupancy ratio. IDE mode can't read the true
    // context_window_size (statusLine is not fired), so 200k is an assumption —
    // long-context models will read >100%, which is more informative than a
    // flat 100% that never recovers after a compact. The HP bar caps visually.
    let used_percentage = (context_tokens as f64 / FALLBACK_CONTEXT_WINDOW as f64) * 100.0;

    json!({
        "session": {
            "id": session_id,
            "name": raw.get("session_name").and_then(|v| v.as_str()).unwrap_or(""),
            "cwd": cwd.clone(),
            "projectDir": cwd.clone(),
            "cwdName": basename(&cwd),
            "transcriptPath": transcript_path,
            "version": raw.get("version").and_then(|v| v.as_str()).unwrap_or(""),
            "model": model_value,
        },
        "tokens": {
            "liveInput": 0,
            "liveOutput": 0,
            "sessionInput": s_in,
            "sessionOutput": s_out,
            "sessionCacheCreation": s_cc,
            "sessionCacheRead": s_cr,
        },
        "git": get_git_info(Path::new(&cwd)),
        "context": {
            "usedPercentage": used_percentage,
            "remainingPercentage": 100.0 - used_percentage,
            "size": FALLBACK_CONTEXT_WINDOW,
            "totalInput": last_in,
            "totalOutput": last_out,
            "current": {
                "input": last_in,
                "output": last_out,
                "cacheCreation": s_cc,
                "cacheRead": s_cr,
            },
            "exceeds200k": context_tokens > FALLBACK_CONTEXT_WINDOW,
        },
    })
}

fn format_number(value: f64) -> String {
    if value.is_nan() {
        return "--".into();
    }
    format!("{}", value as u64)
}

/// One-line fallback for the statusLine stdout (when no legacy statusLine).
pub fn format_fallback_status_line(state: &Value) -> String {
    let model = state
        .pointer("/session/model/display_name")
        .and_then(|v| v.as_str())
        .or_else(|| state.pointer("/session/model/id").and_then(|v| v.as_str()))
        .unwrap_or("Claude");
    let used = state.pointer("/context/usedPercentage").and_then(|v| v.as_f64());
    let pct = match used {
        Some(p) => format!("{}%", p.round() as i64),
        None => "--".into(),
    };
    let git = state.pointer("/git");
    let git_str = match git {
        Some(g) if g.get("isRepo").and_then(|v| v.as_bool()).unwrap_or(false) => {
            let branch = g.get("branch").and_then(|v| v.as_str()).unwrap_or("");
            let dirty = if g.get("dirty").and_then(|v| v.as_u64()).unwrap_or(0) > 0 { "*" } else { "" };
            format!("{branch}{dirty}")
        }
        _ => "no-git".into(),
    };
    let cwd_name = state.pointer("/session/cwdName").and_then(|v| v.as_str()).unwrap_or("cwd");
    let session_in = state
        .pointer("/tokens/sessionInput")
        .and_then(|v| v.as_f64())
        .map(format_number)
        .unwrap_or_else(|| "--".into());
    let session_out = state
        .pointer("/tokens/sessionOutput")
        .and_then(|v| v.as_f64())
        .map(format_number)
        .unwrap_or_else(|| "--".into());
    format!("{model} | {cwd_name} | ctx {pct} | in {session_in} out {session_out} | {git_str}")
}

/// Build an incremental status object from a hook payload. Phase 2.
///
/// `assistant_text` is the latest assistant message text already parsed from
/// the transcript by the caller (the hook CLI process reads the transcript
/// once for `build_session_meta_from_hook` and reuses the text here, avoiding
/// a second tail read on this hot path). Pass `None` to fall back to reading
/// the transcript tail directly.
pub fn status_from_hook(input: &Value, assistant_text: Option<&str>) -> Value {
    let event = input.get("hook_event_name").and_then(|v| v.as_str()).unwrap_or("Hook");
    let now = now_iso();
    let mut m = serde_json::Map::new();
    m.insert("source".into(), json!("hook"));
    m.insert("event".into(), json!(event));
    m.insert("updatedAt".into(), json!(now));
    m.insert("attention".into(), json!(false));
    m.insert("severity".into(), json!("info"));
    m.insert("animation".into(), json!("thinking"));
    m.insert("label".into(), json!(event));
    m.insert("detail".into(), json!(""));

    let live_assistant = if event == "UserPromptSubmit" {
        String::new()
    } else {
        assistant_text
            .map(|t| shorten_multiline(t, 600))
            .or_else(|| {
                input
                    .get("transcript_path")
                    .and_then(|v| v.as_str())
                    .and_then(|p| read_latest_assistant_text(Path::new(p)))
                    .map(|t| shorten_multiline(&t, 600))
            })
            .unwrap_or_default()
    };

    let info = describe_tool(input);
    let tool_name = info.get("tool").and_then(|v| v.as_str()).unwrap_or("tool").to_string();
    let target = info.get("target").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let summary = info.get("summary").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let out = summarize_output(input);

    match event {
        "UserPromptSubmit" => {
            let detail = input.get("prompt").and_then(|v| v.as_str()).map(|s| s.chars().take(120).collect::<String>()).unwrap_or_default();
            set(&mut m, "kind", "thinking");
            set(&mut m, "label", "Reading prompt");
            m.insert("detail".into(), json!(detail));
            set(&mut m, "animation", "thinking");
        }
        "PermissionRequest" => {
            set(&mut m, "kind", "waiting-permission");
            set(&mut m, "label", "Awaiting confirmation");
            m.insert("detail".into(), json!(summary));
            m.insert("tool".into(), json!(tool_name));
            m.insert("target".into(), json!(target));
            m.insert("attention".into(), json!(true));
            set(&mut m, "severity", "warning");
            set(&mut m, "animation", "waiting");
        }
        "Notification" => {
            let ntype = input.get("notification_type").and_then(|v| v.as_str()).unwrap_or("");
            let kind = if ntype == "idle_prompt" { "waiting-input" } else { "notification" };
            let attention = matches!(ntype, "permission_prompt" | "idle_prompt" | "elicitation_dialog");
            let severity = if ntype == "permission_prompt" { "warning" } else { "info" };
            let label = input.get("title").and_then(|v| v.as_str())
                .unwrap_or(if ntype == "permission_prompt" { "Permission prompt" } else { "Claude needs attention" });
            set(&mut m, "kind", kind);
            set(&mut m, "label", label);
            m.insert("detail".into(), json!(input.get("message").and_then(|v| v.as_str()).unwrap_or(ntype)));
            m.insert("attention".into(), json!(attention));
            set(&mut m, "severity", severity);
            set(&mut m, "animation", "waiting");
        }
        "PreToolUse" => {
            set(&mut m, "kind", "running-tool");
            m.insert("label".into(), json!(format!("Running {tool_name}")));
            m.insert("detail".into(), json!(summary));
            m.insert("tool".into(), json!(tool_name));
            m.insert("target".into(), json!(target));
            set(&mut m, "animation", "tool");
        }
        "PostToolUse" => {
            set(&mut m, "kind", "tool-complete");
            m.insert("label".into(), json!(format!("{tool_name} finished")));
            m.insert("detail".into(), json!(if out.is_empty() { summary.clone() } else { out }));
            m.insert("tool".into(), json!(tool_name));
            m.insert("target".into(), json!(target));
            set(&mut m, "animation", "tool");
        }
        "PostToolUseFailure" => {
            set(&mut m, "kind", "tool-error");
            m.insert("label".into(), json!(format!("{tool_name} failed")));
            m.insert("detail".into(), json!(if out.is_empty() { summary.clone() } else { out }));
            m.insert("tool".into(), json!(tool_name));
            m.insert("target".into(), json!(target));
            set(&mut m, "severity", "error");
            set(&mut m, "animation", "error");
            m.insert("attention".into(), json!(true));
        }
        "PostToolBatch" => {
            set(&mut m, "kind", "tool-batch");
            set(&mut m, "label", "Tool batch updated");
            m.insert("detail".into(), json!(input.get("message").and_then(|v| v.as_str()).unwrap_or("")));
            set(&mut m, "animation", "tool");
        }
        "SubagentStart" => {
            let ttype = input.get("agent_type").or_else(|| input.get("subagent_type"))
                .or_else(|| input.pointer("/tool_input/subagent_type"))
                .or_else(|| input.get("agent_id"))
                .and_then(|v| v.as_str()).unwrap_or("agent").to_string();
            set(&mut m, "kind", "subagent-running");
            set(&mut m, "label", "Subagent running");
            m.insert("detail".into(), json!(ttype));
            set(&mut m, "animation", "tool");
            m.insert("subagentType".into(), json!(ttype));
        }
        "SubagentStop" => {
            let ttype = input.get("agent_type").or_else(|| input.get("subagent_type")).or_else(|| input.get("agent_id")).and_then(|v| v.as_str()).unwrap_or("").to_string();
            set(&mut m, "kind", "subagent-complete");
            set(&mut m, "label", "Subagent finished");
            m.insert("detail".into(), json!(if out.is_empty() { ttype.clone() } else { out }));
            set(&mut m, "animation", "tool");
            m.insert("subagentType".into(), json!(ttype));
            m.insert("subagentEnded".into(), json!(true));
        }
        "TaskCreated" => {
            set(&mut m, "kind", "task-created");
            set(&mut m, "label", "Task created");
            m.insert("detail".into(), json!(input.get("task_subject").or_else(|| input.get("task_id")).and_then(|v| v.as_str()).unwrap_or("")));
            set(&mut m, "animation", "tool");
        }
        "TaskCompleted" => {
            let detail = if out.is_empty() {
                input.get("task_subject").or_else(|| input.get("task_id")).and_then(|v| v.as_str()).unwrap_or("").to_string()
            } else {
                out
            };
            set(&mut m, "kind", "task-completed");
            set(&mut m, "label", "Task completed");
            m.insert("detail".into(), json!(detail));
            set(&mut m, "animation", "tool");
        }
        "Stop" => {
            set(&mut m, "kind", "completed");
            set(&mut m, "label", "Claude finished");
            m.insert("detail".into(), json!(out));
            set(&mut m, "animation", "success");
            m.insert("attention".into(), json!(true));
        }
        "StopFailure" => {
            set(&mut m, "kind", "error");
            set(&mut m, "label", "Claude stopped with an error");
            m.insert("detail".into(), json!(input.get("error_details").or_else(|| input.get("error")).and_then(|v| v.as_str()).unwrap_or("")));
            set(&mut m, "severity", "error");
            set(&mut m, "animation", "error");
            m.insert("attention".into(), json!(true));
        }
        "PreCompact" => {
            set(&mut m, "kind", "compacting");
            set(&mut m, "label", "Compacting context");
            m.insert("detail".into(), json!(input.get("trigger").and_then(|v| v.as_str()).unwrap_or("")));
            set(&mut m, "animation", "thinking");
        }
        "PostCompact" => {
            set(&mut m, "kind", "compacted");
            set(&mut m, "label", "Context compacted");
            let detail = input.get("compact_summary").and_then(|v| v.as_str())
                .map(|s| s.chars().take(180).collect::<String>())
                .unwrap_or_else(|| input.get("trigger").and_then(|v| v.as_str()).unwrap_or("").to_string());
            m.insert("detail".into(), json!(detail));
            set(&mut m, "animation", "thinking");
        }
        _ => {
            set(&mut m, "kind", "activity");
            set(&mut m, "label", event);
            m.insert("detail".into(), json!(summary));
            set(&mut m, "animation", "thinking");
        }
    };

    if !live_assistant.is_empty() {
        m.insert("detail".into(), json!(live_assistant));
    }
    Value::Object(m)
}

fn set(m: &mut serde_json::Map<String, Value>, key: &str, val: &str) {
    m.insert(key.into(), json!(val));
}
