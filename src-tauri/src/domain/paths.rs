// Path resolution. Mirrors ClaudePet/src/shared/paths.js.
//
// All persistent data lives under `~/.claudepet/` (overridable via
// CLAUDEPET_HOME / CCPET_HOME). Claude Code's own dir is `~/.claude/`
// (overridable via CLAUDE_HOME).

use std::path::PathBuf;

/// Absolute path to the running binary's directory (used to locate `pet/` and
/// to write the binary path into Claude Code's settings.json on install).
pub fn app_root() -> PathBuf {
    // In a Tauri build the resource/pet dir is resolved relative to the exe.
    // Fall back to the current exe's parent.
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Directory holding bundled `pet/<id>/` folders. In dev this is the workspace
/// `pet/` dir; in a packaged build it lives next to the exe.
pub fn pet_dir() -> PathBuf {
    // 1. <exe_dir>/pet  (packaged portable build)
    let packaged = app_root().join("pet");
    if packaged.is_dir() {
        return packaged;
    }
    // 2. <exe_dir>/resources/pet  (nsis installer resource dir)
    let resource = app_root().join("resources").join("pet");
    if resource.is_dir() {
        return resource;
    }
    // 3. dev fallback: <workspace>/claude-pet/pet
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("pet");
    dev
}

/// `~/.claudepet` (or $CLAUDEPET_HOME / $CCPET_HOME).
pub fn app_home() -> PathBuf {
    std::env::var("CLAUDEPET_HOME")
        .or_else(|_| std::env::var("CCPET_HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| dirs::home_dir().unwrap_or_default().join(".claudepet"))
}

/// `~/.claude` (or $CLAUDE_HOME).
pub fn claude_home() -> PathBuf {
    std::env::var("CLAUDE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dirs::home_dir().unwrap_or_default().join(".claude"))
}

pub fn runtime_path() -> PathBuf {
    app_home().join("runtime.json")
}

pub fn config_path() -> PathBuf {
    app_home().join("config.json")
}

pub fn state_path() -> PathBuf {
    app_home().join("state.json")
}

/// Path to this binary, quoted for shell embedding into Claude Code settings.
pub fn current_exe_string() -> String {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("claude-pet"));
    quote_arg(&exe.to_string_lossy())
}

/// Build a shell command string `<exe> <args...>` for embedding into settings.json.
pub fn build_exe_command(args: &[&str]) -> String {
    let mut parts = vec![current_exe_string()];
    for a in args {
        parts.push(quote_arg(a));
    }
    parts.join(" ")
}

/// Quote a single argument for the current platform's shell.
pub fn quote_arg(value: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}
