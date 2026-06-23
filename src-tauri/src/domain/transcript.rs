// Transcript parsing. Mirrors ClaudePet/src/shared/transcript.js.
//
// Reads the tail of a Claude Code `.jsonl` transcript to extract cumulative
// token usage (input/output/cache-creation/cache-read) and the latest
// assistant text. Tail-only reads keep large transcripts cheap.

use serde::Serialize;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

const USAGE_MAX_BYTES: u64 = 2 * 1024 * 1024;
/// Tail size for the latest-assistant-text search. The most recent assistant
/// record is always near the file's end, so a small tail suffices and keeps
/// this hot path cheap (it runs on every hook event).
const ASSISTANT_MAX_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Default, Serialize)]
pub struct TranscriptUsage {
    pub input: u64,
    pub output: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
}

/// Transcript-derived session metadata: cumulative usage plus the latest
/// assistant record's model and context size (input + cache_read tokens — an
/// approximation of the current context window occupancy). Used to populate
/// the pet's session/tokens/context fields when statusLine is not fired (IDE
/// `--no-chrome` headless mode).
#[derive(Debug, Clone, Default, Serialize)]
pub struct TranscriptMeta {
    pub usage: TranscriptUsage,
    pub model: Option<String>,
    pub context_tokens: u64,
    pub cwd: Option<String>,
    /// Latest assistant message text (for the pet's status detail bubble).
    pub assistant_text: Option<String>,
    /// Token counts from the LAST assistant record only (not cumulative).
    /// Mirrors Claude Code's `context_window.current_usage` semantics. Used for
    /// the pet's "current" context fields so they don't show session-wide totals.
    pub last_input: u64,
    pub last_output: u64,
}

/// Read the latest assistant message text from the transcript tail (≤64KB).
pub fn read_latest_assistant_text(path: &Path) -> Option<String> {
    let lines = read_tail_lines(path, ASSISTANT_MAX_BYTES)?;
    for line in lines.into_iter().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if !is_assistant_record(&v) {
            continue;
        }
        if let Some(text) = extract_assistant_text(&v) {
            if !text.trim().is_empty() {
                return Some(text);
            }
        }
    }
    None
}

/// Read session metadata from the transcript tail in a single pass: the latest
/// assistant record's model and context size (input + cache_read tokens), the
/// most recent `cwd`, and cumulative usage. Returns None only when the
/// transcript can't be read at all.
///
/// `input`/`output`/`cache_creation` are summed across records (real token
/// spend). `cache_read` is NOT summed — every API call re-reads the full
/// context from cache, so summing would wildly overcount. Instead it holds the
/// latest record's cache_read (≈ current context size loaded from cache).
pub fn read_transcript_meta(path: &Path) -> Option<TranscriptMeta> {
    let lines = read_tail_lines(path, USAGE_MAX_BYTES)?;
    let mut meta = TranscriptMeta::default();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        // Track the most recent cwd seen on any record (user/assistant carry it).
        if let Some(cwd) = v.get("cwd").and_then(|c| c.as_str()) {
            if !cwd.is_empty() {
                meta.cwd = Some(cwd.to_string());
            }
        }
        // The latest assistant record wins for model + context_tokens + cache_read + text.
        if is_assistant_record(&v) {
            if let Some(model) = v
                .get("message")
                .and_then(|m| m.get("model"))
                .and_then(|m| m.as_str())
            {
                meta.model = Some(model.to_string());
            }
            // Latest assistant text (non-empty) for the status detail bubble.
            if let Some(text) = extract_assistant_text(&v) {
                if !text.trim().is_empty() {
                    meta.assistant_text = Some(text);
                }
            }
            if let Some(usage) = v.get("message").and_then(|m| m.get("usage")) {
                let input = usage.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
                let output = usage.get("output_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
                let cache_creation = usage
                    .get("cache_creation_input_tokens")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0);
                let cache_read = usage
                    .get("cache_read_input_tokens")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0);
                // Skip synthetic/empty records (all-zero usage, e.g. aborted
                // responses) — they would clobber the last real record's
                // context_tokens/cache_read with zeros.
                let nonzero = input + output + cache_creation + cache_read > 0;
                // Accumulate real spend; cache_read is latest-only (see doc comment).
                meta.usage.input += input;
                meta.usage.output += output;
                meta.usage.cache_creation += cache_creation;
                if nonzero {
                    meta.usage.cache_read = cache_read;
                    // Last record's raw counts (non-cumulative) for "current" fields.
                    meta.last_input = input;
                    meta.last_output = output;
                    meta.context_tokens = input + cache_read;
                }
            }
        }
    }
    Some(meta)
}

/// Read up to `max_bytes` from the file tail, split into lines. If we read
/// from the middle of the file, the first (possibly truncated) line is dropped.
fn read_tail_lines(path: &Path, max_bytes: u64) -> Option<Vec<String>> {
    let mut file = std::fs::File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    let start = if size > max_bytes { size - max_bytes } else { 0 };
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut reader = BufReader::new(file);
    // When reading from the middle of the file, the first line is a partial
    // record — drop it.
    if start > 0 {
        let mut buf = Vec::new();
        let _ = reader.read_until(b'\n', &mut buf);
    }
    let mut lines: Vec<String> = Vec::new();
    for line in reader.lines() {
        match line {
            Ok(l) => lines.push(l),
            Err(_) => break,
        }
    }
    Some(lines)
}

fn is_assistant_record(v: &serde_json::Value) -> bool {
    v.get("type").and_then(|t| t.as_str()) == Some("assistant")
        || v.get("message").and_then(|m| m.get("role")).and_then(|r| r.as_str()) == Some("assistant")
}

fn extract_assistant_text(v: &serde_json::Value) -> Option<String> {
    let message = v.get("message").or(Some(v))?;
    let content = message.get("content")?;
    if let Some(s) = content.as_str() {
        return Some(s.to_string());
    }
    if let Some(arr) = content.as_array() {
        let mut parts = Vec::new();
        for block in arr {
            if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                    parts.push(t.to_string());
                }
            }
        }
        if !parts.is_empty() {
            return Some(parts.join("\n"));
        }
    }
    None
}
