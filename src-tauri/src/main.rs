// ClaudePet — Claude Code desktop pet (Rust + Tauri rewrite).
//
// Single binary serves both the GUI (Tauri) and the CLI (statusline/hook/
// install/doctor). The entrypoint parses the command line: a known subcommand
// runs the CLI path (no GUI), otherwise the Tauri GUI main process launches.
//
// Windows dual-mode:
//   - Release binary uses `windows_subsystem = "windows"` so double-clicking
//     the exe doesn't spawn a CMD window.
//   - CLI subcommands attach to the parent console on the fly so `stdout`
//     works when invoked from a terminal or by Claude Code hooks.

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use clap::Parser;
use claude_pet_lib::cli::{Cli, CliCommand};

fn main() {
    let cli = Cli::parse();

    match cli.command {
        None => {
            // GUI mode — no console needed (windows_subsystem hides it).
            claude_pet_lib::run_gui();
        }
        Some(cmd) => {
            // CLI mode — attach to the parent console so stdout reaches the
            // terminal (or Claude Code's hook machinery).
            #[cfg(target_os = "windows")]
            attach_parent_console();
            let code = claude_pet_lib::run_cli(cmd);
            std::process::exit(code);
        }
    }
}

/// Attach to the parent process's console for CLI stdout/stderr output.
/// This is only needed when the binary is compiled with
/// `windows_subsystem = "windows"` because Windows won't automatically
/// connect the stdio handles to the calling terminal.
#[cfg(target_os = "windows")]
fn attach_parent_console() {
    use windows_sys::Win32::System::Console::AttachConsole;
    // ATTACH_PARENT_PROCESS = 0xFFFFFFFF
    unsafe {
        AttachConsole(0xFFFFFFFF);
    }
}

#[allow(dead_code)]
fn _type_check(_c: CliCommand) {}
