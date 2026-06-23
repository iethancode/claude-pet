// Atomic JSON read/write with Windows file-lock retry and corruption isolation.
// Port of ClaudePet/src/shared/json-file.js.

use serde_json::Value;
use std::path::{Path, PathBuf};

/// Ensure a directory exists (mkdir -p).
pub fn ensure_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
}

/// Read & parse JSON. On ENOENT returns `fallback`. On parse failure, isolates
/// the corrupt file to `<file>.corrupt-<timestamp>` and returns `fallback`.
pub fn read_json(path: &Path, fallback: Value) -> Value {
    let Ok(text) = std::fs::read_to_string(path) else {
        return fallback;
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(v) => v,
        Err(_) => {
            // Isolate corrupt file so the main process can still boot.
            let ts = chrono::Local::now().format("%Y-%m-%dT%H-%M-%S").to_string();
            let corrupt = PathBuf::from(format!("{}.corrupt-{ts}", path.display()));
            let _ = std::fs::rename(path, &corrupt);
            fallback
        }
    }
}

/// Strict variant for files we must NOT silently overwrite when corrupt
/// (e.g. Claude Code's settings.json — clobbering it would wipe the user's
/// env/model/permissions). Returns:
///   - `Ok(value)`     — parsed cleanly
///   - `Ok(None)`      — file does not exist (caller decides default)
///   - `Err(message)`  — file exists but is not valid JSON; the corrupt file
///                       is left in place (NOT isolated) so the user can fix
///                       it. The message names the file + parse error.
pub fn read_json_strict(path: &Path) -> Result<Option<Value>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {}", path.display(), e)),
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(v) => Ok(Some(v)),
        Err(e) => Err(format!(
            "{} is not valid JSON and was left unchanged: {}. Fix it (or restore from a .claudepet-backup) before re-running install.",
            path.display(),
            e
        )),
    }
}

/// Atomically write JSON: write to `<file>.tmp` then rename. On Windows,
/// rename can race with concurrent readers/writers — retry a handful of times
/// on EPERM/EACCES/EBUSY/ENOTEMPTY.
pub fn write_json(path: &Path, value: &Value) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    let body = serde_json::to_vec_pretty(value).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));

    let mut attempts = 0;
    loop {
        match write_atomic(&tmp, path, &body) {
            Ok(()) => return Ok(()),
            Err(e) if attempts < 5 && is_transient(&e) => {
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(30 * attempts as u64));
            }
            Err(e) => return Err(e),
        }
    }
}

fn write_atomic(tmp: &Path, dest: &Path, body: &[u8]) -> std::io::Result<()> {
    std::fs::write(tmp, body)?;
    std::fs::rename(tmp, dest)
}

fn is_transient(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::PermissionDenied
            | std::io::ErrorKind::ResourceBusy
            | std::io::ErrorKind::WouldBlock
    ) || e.raw_os_error().map(|c| matches!(c, 5 | 13 | 32 | 39)).unwrap_or(false)
    // 5=ACCESS_DENIED on some, 13=EACCES, 32=EBUSY-ish, 39=ENOTEMPTY (win)
}
