// Library entry shared by the binary. Hosts the GUI runtime and CLI dispatch.

pub mod bridge;
pub mod cli;
pub mod commands;
pub mod domain;
pub mod fs;
pub mod lifecycle;
pub mod passthrough;
pub mod state;
pub mod tray;

use tauri::Manager;

use crate::cli::CliCommand;

/// Launch the Tauri GUI main process (desktop pet + bridge server).
pub fn run_gui() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // Second launch: focus existing instance / show pets.
            log::info!("[claude-pet] second instance detected");
            for win in app.webview_windows().values() {
                let _ = win.show();
                let _ = win.set_focus();
            }
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Boot cleanup, then start the bridge + prune timer + tray.
            lifecycle::boot_prune();

            // Destroy any stale webview windows from a previous dev-session
            // run. Those windows load from localhost:65173 (the Vite dev
            // server) and would show "connection refused" once the dev server
            // is gone. The release binary creates fresh windows via the
            // embedded tauri:// protocol.
            for (_label, win) in app.webview_windows() {
                let _ = win.destroy();
            }

            let handle = app.handle().clone();
            let bridge = bridge::start_bridge(handle);
            app.manage(bridge);
            lifecycle::start_prune_timer(app.handle().clone());
            tray::setup_tray(app.handle())?;
            // Click-through poll loop: runs on Tauri's async runtime. The loop
            // only does a cheap OS cursor query + window geometry read every
            // 60ms (no I/O), so it doesn't need the bridge's dedicated runtime.
            passthrough::start_poll_loop(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            // Drop passthrough state when a pet window closes.
            if let tauri::WindowEvent::Destroyed = event {
                passthrough::remove(window.label());
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::ping,
            commands::get_initial,
            commands::get_pets,
            commands::drag_window,
            commands::set_passthrough,
            commands::set_hit_rect,
            commands::hold_interactive,
            commands::release_interactive,
            commands::set_session_pet,
            commands::close_pet,
            commands::bridge_info,
            commands::respond_permission,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ClaudePet GUI");
}

/// Dispatch a CLI subcommand. Returns the process exit code.
pub fn run_cli(cmd: CliCommand) -> i32 {
    match cli::dispatch(cmd) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("claude-pet: {e:#}");
            1
        }
    }
}
