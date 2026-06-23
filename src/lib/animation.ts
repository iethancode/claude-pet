// Animation selection. Mirrors renderer.js#animationFor.

const WORKING_KINDS = new Set([
  "thinking",
  "running-tool",
  "tool-complete",
  "tool-batch",
  "subagent-running",
  "subagent-complete",
  "task-created",
  "task-completed",
  "activity",
  "compacting",
  "compacted",
]);

/** Decide which animation to play given the session status (and drag state). */
export function animationFor(status: { kind: string; animation: string } | undefined, dragging: boolean): string {
  if (dragging) return "run";
  const anim = status?.animation;
  if (anim && anim !== "idle") return anim;
  if (status && WORKING_KINDS.has(status.kind)) return "thinking";
  return "idle";
}
