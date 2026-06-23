// Shared types mirroring the Rust domain models.

export interface Anchor {
  x: number;
  y: number;
}

export interface AnimationDef {
  frames: number[];
  fps: number;
  looped: boolean;
}

export interface PetManifest {
  id: string;
  displayName: string;
  description: string;
  kind: string;
  spritesheetPath: string;
  spritesheetFile: string;
  imageWidth: number;
  imageHeight: number;
  frameWidth: number;
  frameHeight: number;
  columns: number;
  rows: number;
  defaultScale: number;
  anchor: Anchor;
  animations: Record<string, AnimationDef>;
}

export interface GitInfo {
  isRepo: boolean;
  branch: string;
  dirty: number;
  staged: number;
  untracked: number;
  ahead: number;
  behind: number;
}

export interface StatusState {
  kind: string;
  label: string;
  detail: string;
  severity: "info" | "warning" | "error" | string;
  attention: boolean;
  animation: string;
  updatedAt: string;
}

export interface SessionState {
  source?: string;
  updatedAt?: string;
  session: {
    id: string;
    name?: string;
    cwd: string;
    projectDir?: string;
    cwdName: string;
    transcriptPath?: string;
    version?: string;
    model?: { display_name?: string; id?: string; [k: string]: unknown };
    [k: string]: unknown;
  };
  cost: {
    totalCostUsd: number;
    totalDurationMs: number;
    totalApiDurationMs: number;
    linesAdded: number;
    linesRemoved: number;
  };
  context: {
    usedPercentage: number | null;
    remainingPercentage: number | null;
    size: number;
    totalInput: number;
    totalOutput: number;
    current: { input: number; output: number; cacheCreation: number; cacheRead: number };
    exceeds200k: boolean;
  };
  tokens: {
    liveInput: number;
    liveOutput: number;
    sessionInput: number | null;
    sessionOutput: number | null;
    sessionCacheCreation: number | null;
    sessionCacheRead: number | null;
  };
  git: GitInfo;
  status: StatusState;
  /** A pending Claude Code permission request awaiting the user's decision,
   *  or null when none is pending. Set by the hook CLI on PermissionRequest. */
  pendingPermission?: PendingPermission | null;
  [k: string]: unknown;
}

export interface PendingPermission {
  requestId: string;
  tool: string;
  description: string;
  sessionId?: string;
  /** True when Claude Code offered session-scope permission_suggestions —
   *  controls whether the "allow this type for the session" button is shown. */
  canAutoApprove?: boolean;
  scope?: string;
  [k: string]: unknown;
}

export interface AppConfig {
  selectedPet: string;
  positions: Record<string, [number, number]>;
  selectedPets: Record<string, string>;
  panelVisibility: Record<string, boolean>;
  legacyStatusLine: unknown;
  [k: string]: unknown;
}

export interface InitialPayload {
  sessionId: string;
  state: SessionState;
  config: AppConfig;
  pets: PetManifest[];
  appVersion: string;
  sessions: Record<string, SessionState>;
}

export interface UpdatePayload {
  sessionId: string;
  state: SessionState;
  config: AppConfig;
}
