// HTTP bridge server (runs inside the GUI main process). Mirrors
// ClaudePet/src/main/bridge-server.js. Claude Code's statusLine/hook CLI
// processes POST events here over 127.0.0.1 with a bearer token; the server
// discovers the port/token via `~/.claudepet/runtime.json`.

pub mod handlers;
pub mod permissions;
pub mod server;

pub use server::{start_bridge, BridgeHandle, BridgeRuntime};
