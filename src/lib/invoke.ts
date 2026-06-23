import { invoke } from "@tauri-apps/api/core";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { AppConfig, InitialPayload } from "./types";

export function getInitial(): Promise<InitialPayload> {
  return invoke<InitialPayload>("get_initial");
}

/** Switch the pet for the calling session's window (per-session). */
export function setSessionPet(petId: string): Promise<AppConfig> {
  return invoke<AppConfig>("set_session_pet", { petId });
}

/** Close the calling session's pet window and drop its state. Fire-and-forget:
 *  the window is destroyed, so the promise rarely resolves. */
export function closePet(): void {
  invoke("close_pet").catch(() => {});
}

/** Respond to a pending Claude Code permission request. `action` is one of
 *  "allow" | "deny" | "allow_session" (remember this tool/pattern for the
 *  session via Claude Code's updatedPermissions) | "auto_yes_session" (allow
 *  this + auto-allow every remaining tool in the session). Resolves the
 *  oneshot the bridge handler is blocking on. */
export function respondPermission(
  sessionId: string,
  requestId: string,
  action: "allow" | "deny" | "allow_session" | "auto_yes_session",
): Promise<void> {
  return invoke("respond_permission", { sessionId, requestId, action });
}

/** Register the interactive hit rectangle (CSS px, relative to window
 *  top-left) for the cursor-poll click-through loop. The Rust side converts
 *  to physical px and enables click-through outside this rect. */
export function setHitRect(x: number, y: number, w: number, h: number): Promise<void> {
  return invoke("set_hit_rect", { x, y, w, h });
}

/** Force the window to stay interactive (e.g. while a context menu is open or
 *  while dragging) so the poll loop doesn't click-through under the cursor. */
export function holdInteractive(): Promise<void> {
  return invoke("hold_interactive");
}

/** Release a hold acquired with `holdInteractive`. */
export function releaseInteractive(): Promise<void> {
  return invoke("release_interactive");
}

/** Convert a local absolute path to a URL the webview can load. */
export function assetUrl(absPath: string): string {
  return convertFileSrc(absPath);
}

