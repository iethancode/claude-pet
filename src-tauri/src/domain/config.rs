// Config — `~/.claudepet/config.json`. Mirrors ClaudePet/src/shared/config.js.
//
// Holds the selected pet (global default + per-session overrides), window
// positions, panel visibility, legacy statusLine forwarding and install
// backups. Display prefs (theme/scale/opacity/always-on-top) were removed
// along with the settings center — the pet window is always dark, scale 0.48,
// always-on-top.

use serde_json::{json, Value};

use super::paths::config_path;
use crate::fs::json_file::{read_json, write_json};

/// Load config merged over defaults (deep merge).
pub fn load_config() -> Value {
    merge_deep(default_config(), read_json(&config_path(), json!({})))
}

/// Deep-merge `patch` over the current config on disk and persist atomically.
pub fn save_config(patch: &Value) -> Value {
    let current = load_config();
    let merged = merge_deep(current, patch.clone());
    let _ = write_json(&config_path(), &merged);
    merged
}

pub fn default_config() -> Value {
    json!({
        "selectedPet": "clawd",
        "positions": {},
        "selectedPets": {},
        "panelVisibility": {},
        "legacyStatusLine": null,
        "installBackups": {}
    })
}

/// Deep merge `base` with `over` (arrays replaced, objects merged recursively).
pub fn merge_deep(mut base: Value, over: Value) -> Value {
    match (&mut base, over) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                let merged = match a.remove(&k) {
                    Some(existing) => merge_deep(existing, v),
                    None => v,
                };
                a.insert(k, merged);
            }
            Value::Object(std::mem::take(a))
        }
        (_, over) => over,
    }
}
