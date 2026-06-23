// Filesystem helpers — atomic JSON write + corruption isolation.
// Mirrors ClaudePet/src/shared/json-file.js.

pub mod json_file;

pub use json_file::{ensure_dir, read_json, read_json_strict, write_json};
