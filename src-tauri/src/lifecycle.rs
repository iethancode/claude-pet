// App lifecycle: prune timer, session hydration on boot, single-instance.

use std::time::Duration;

use tauri::AppHandle;

use crate::bridge::server::close_pet_window;
use crate::state::runtime_state::{prune_stale_sessions, DEFAULT_SESSION_ID};

const SESSION_INACTIVITY_MS: u64 = 15 * 60 * 1000;
const PRUNE_INTERVAL_MS: u64 = 60 * 1000;

/// On boot: prune stale sessions left over from a previous run. We do NOT
/// auto-open windows for surviving sessions — those open on demand when the
/// next real CLI event arrives (avoids ghost windows on auto-start).
pub fn boot_prune() {
    let removed = prune_stale_sessions(SESSION_INACTIVITY_MS);
    if !removed.is_empty() {
        log::info!("[claude-pet] boot prune removed {} stale sessions", removed.len());
    }
}

/// Start the 60s prune timer. Stale sessions (>15min inactive) are dropped and
/// their windows destroyed.
pub fn start_prune_timer(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(PRUNE_INTERVAL_MS));
        loop {
            ticker.tick().await;
            let removed = prune_stale_sessions(SESSION_INACTIVITY_MS);
            for id in &removed {
                if id == DEFAULT_SESSION_ID {
                    continue;
                }
                close_pet_window(&app, id);
            }
            if !removed.is_empty() {
                log::info!("[claude-pet] pruned {} inactive sessions", removed.len());
            }
        }
    });
}

#[allow(dead_code)]
fn _unused(_app: &AppHandle) {}
