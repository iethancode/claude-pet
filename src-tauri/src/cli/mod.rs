// CLI dispatch. Subcommands run as short-lived processes invoked by Claude
// Code's statusLine / hooks, or manually by the user for install/doctor/pets.

pub mod bridge_client;
pub mod doctor;
pub mod hook;
pub mod install;
pub mod pets;
pub mod statusline;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "claude-pet",
    bin_name = "claude-pet",
    about = "ClaudePet — Claude Code desktop pet",
    version,
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<CliCommand>,
}

#[derive(Subcommand)]
pub enum CliCommand {
    /// Claude Code statusLine bridge — reads JSON from stdin, forwards to the pet
    Statusline,
    /// Claude Code hooks bridge — reads JSON from stdin, forwards to the pet
    Hook,
    /// Install Claude Code integration (statusLine + hooks)
    Install {
        /// `user` (~/.claude/settings.json) or `local` (<cwd>/.claude/settings.local.json)
        #[arg(long, default_value = "local")]
        scope: String,
        /// Save the existing statusLine as legacy for forwarding
        #[arg(long, default_value_t = false)]
        preserve_statusline: bool,
    },
    /// Remove Claude Code integration
    Uninstall {
        #[arg(long, default_value = "local")]
        scope: String,
    },
    /// Launch the desktop pet GUI
    Start,
    /// List available pets
    Pets,
    /// Show runtime diagnostics
    Doctor,
}

/// Dispatch a subcommand. Returns Ok on success.
pub fn dispatch(cmd: CliCommand) -> anyhow::Result<()> {
    match cmd {
        CliCommand::Pets => pets::run(),
        CliCommand::Doctor => doctor::run(),
        CliCommand::Start => start_gui(),
        CliCommand::Install { scope, preserve_statusline } => install::install(&scope, preserve_statusline),
        CliCommand::Uninstall { scope } => install::uninstall(&scope),
        CliCommand::Statusline => statusline::run(),
        CliCommand::Hook => hook::run(),
    }
}

/// `claude-pet start` — spawn the GUI detached (the binary itself with no args).
fn start_gui() -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    use std::process::Command;
    Command::new(&exe)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(())
}
