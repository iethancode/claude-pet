import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { AppConfig, PetManifest, SessionState } from "../lib/types";
import { closePet, getInitial, holdInteractive, releaseInteractive, respondPermission, setHitRect, setSessionPet } from "../lib/invoke";
import { useUpdate } from "../hooks/useUpdate";
import { animationFor } from "../lib/animation";
import { formatNumber } from "../lib/format";
import PetCanvas from "../components/PetCanvas";

type Severity = "info" | "warning" | "error";

/** Render scale (previously `config.scale`, now hardcoded — the settings center
 *  was removed). Matches the original global default of 0.48. */
const PET_SCALE = 0.48;

function severityClass(sev: string | undefined): Severity {
  if (sev === "error") return "error";
  if (sev === "warning") return "warning";
  return "info";
}

function hpLevel(pct: number | null | undefined): "" | "warn" | "danger" {
  if (typeof pct !== "number" || !isFinite(pct)) return "";
  if (pct >= 90) return "danger";
  if (pct >= 70) return "warn";
  return "";
}

/**
 * Desktop pet window. Sprite anchored bottom-center; a compact status bar
 * floats above it (model + ctx% + status dot). Clicking the bar toggles the
 * full info panel. Right-clicking the pet opens a context menu to switch
 * appearance (per-session). One window per Claude Code session.
 */
export default function PetView() {
  const [sessionId, setSessionId] = useState("");
  const [state, setState] = useState<SessionState | null>(null);
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [pets, setPets] = useState<PetManifest[]>([]);
  const [panelOpen, setPanelOpen] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [permBusy, setPermBusy] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const stackRef = useRef<HTMLDivElement>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const dragOrigin = useRef<{ x: number; y: number } | null>(null);

  // Report the interactive region (the union of the status-bar/panel stack and
  // the sprite stage) to the Rust poll loop as a hit rectangle in CSS px
  // relative to the window. Outside this rect the window is click-through;
  // inside it the window receives pointer events. The context menu is a
  // floating overlay handled separately via holdInteractive.
  const reportHitRect = () => {
    const a = stackRef.current?.getBoundingClientRect();
    const b = stageRef.current?.getBoundingClientRect();
    const rects = [a, b].filter((r): r is DOMRect => !!r);
    if (rects.length === 0) return;
    const x1 = Math.min(...rects.map((r) => r.left));
    const y1 = Math.min(...rects.map((r) => r.top));
    const x2 = Math.max(...rects.map((r) => r.right));
    const y2 = Math.max(...rects.map((r) => r.bottom));
    void setHitRect(x1, y1, x2 - x1, y2 - y1);
  };

  useEffect(() => {
    getInitial()
      .then((p) => {
        setSessionId(p.sessionId);
        setState(p.state);
        setConfig(p.config);
        setPets(p.pets ?? []);
      })
      .catch((e) => console.error("[claude-pet] get_initial failed", e));
  }, []);

  const update = useUpdate(sessionId, "pet");
  useEffect(() => {
    if (update) {
      setState(update.state);
      if (update.config) setConfig(update.config);
      // Permission resolved/cleared on the server side — re-enable buttons.
      if (!update.state?.pendingPermission) setPermBusy(null);
    }
  }, [update]);

  // Live config updates (per-session pet switch broadcasts config-changed).
  useEffect(() => {
    let unlist: (() => void) | undefined;
    listen<{ config: AppConfig }>("claudepet:config-changed", (e) => {
      setConfig(e.payload.config);
    }).then((fn) => { unlist = fn; });
    return () => { unlist?.(); };
  }, []);

  const pet = useMemo(() => {
    const id = config?.selectedPets?.[sessionId] || config?.selectedPet || "clawd";
    return pets.find((p) => p.id === id) ?? pets[0];
  }, [pets, config, sessionId]);

  const status = state?.status;
  const animation = animationFor(status, dragging);
  const sev = severityClass(status?.severity);
  const hp = hpLevel(state?.context.usedPercentage ?? null);

  // Drag the window. Held interactive while dragging so the poll loop doesn't
  // click-through under the cursor mid-drag.
  async function onPointerDown(e: React.PointerEvent) {
    if (e.button !== 0) return;
    setDragging(true);
    dragOrigin.current = { x: e.screenX, y: e.screenY };
    void holdInteractive();
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      await getCurrentWindow().startDragging();
    } catch (err) {
      console.warn("[claude-pet] startDragging failed", err);
    } finally {
      setDragging(false);
      void releaseInteractive();
    }
  }

  // Click-through via cursor-position polling: the Rust poll loop enables
  // click-through outside the hit rect reported below, and disables it inside.
  // This replaces the old set_ignore_cursor_events approach (which had no
  // {forward:true} equivalent and left transparent areas blocking the desktop).
  // Report the hit rect on mount + whenever layout changes (ResizeObserver)
  // + whenever state that changes layout changes (panel/menu/drag/perm).
  useEffect(() => {
    reportHitRect();
    const ro = new ResizeObserver(() => reportHitRect());
    if (stackRef.current) ro.observe(stackRef.current);
    if (stageRef.current) ro.observe(stageRef.current);
    // Also re-report on window resize (scale factor / size changes).
    const onResize = () => reportHitRect();
    window.addEventListener("resize", onResize);
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", onResize);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [panelOpen, menu, dragging, state?.pendingPermission, state?.status, pet]);

  // Hold interactive while the context menu is open (so the poll loop keeps
  // the window interactive for the menu even if the cursor drifts off the
  // hit rect).
  useEffect(() => {
    if (!menu) return;
    void holdInteractive();
    return () => { void releaseInteractive(); };
  }, [menu]);

  // Close the context menu on outside click / Escape / window blur.
  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    window.addEventListener("pointerdown", close);
    window.addEventListener("blur", close);
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") setMenu(null); };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("blur", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [menu]);

  // Clamp the menu inside the window so it isn't clipped at the edges
  // (the pet window is small — 438×338 — so an un-clamped menu at the click
  // point overflows the bottom/right and gets cut off). Mirrors
  // showPetContextMenu's rect-based clamping in the original renderer.
  useLayoutEffect(() => {
    if (!menu || !menuRef.current) return;
    const rect = menuRef.current.getBoundingClientRect();
    const margin = 8;
    const maxLeft = Math.max(margin, window.innerWidth - rect.width - margin);
    const maxTop = Math.max(margin, window.innerHeight - rect.height - margin);
    const x = Math.min(Math.max(margin, menu.x), maxLeft);
    const y = Math.min(Math.max(margin, menu.y), maxTop);
    if (x !== menu.x || y !== menu.y) {
      setMenu({ x, y });
    }
  }, [menu]);

  async function switchPet(petId: string) {
    setMenu(null);
    try {
      await setSessionPet(petId);
    } catch (e) {
      console.error("[claude-pet] set_session_pet failed", e);
    }
  }

  // Respond to a pending Claude Code permission request. The bridge handler is
  // blocking on a oneshot for up to 295s; this resolves it. Buttons disable
  // until the next state update clears pendingPermission.
  async function answerPermission(action: "allow" | "deny" | "allow_session" | "auto_yes_session") {
    const perm = state?.pendingPermission;
    if (!perm || !sessionId) return;
    setPermBusy(action);
    try {
      await respondPermission(sessionId, perm.requestId, action);
    } catch (e) {
      console.error("[claude-pet] respond_permission failed", e);
      setPermBusy(null);
    }
  }

  if (!state || !pet) {
    return <div className="pet-root" />;
  }

  const model = (state.session.model as any)?.display_name || (state.session.model as any)?.id || "Claude";
  const cwdName = state.session.cwdName || "当前目录";
  const usedPct = state.context.usedPercentage;
  // IDE mode approximates context% (statusLine isn't fired, window size assumed
  // 200k), so it can exceed 100% on long-context models. Show "100%+" on
  // overflow rather than a misleading precise figure; the HP bar fills full.
  const pctText =
    typeof usedPct === "number" && isFinite(usedPct)
      ? usedPct >= 100
        ? "100%+"
        : `${Math.round(usedPct)}%`
      : "--";
  const git = state.git;
  const gitStr = git?.isRepo
    ? `${git.branch}${git.dirty ? "*" : ""}${git.ahead ? ` ↑${git.ahead}` : ""}${git.behind ? ` ↓${git.behind}` : ""}`
    : "无 git";
  const barClass = ["pet-bar", hp, panelOpen ? "open" : ""].filter(Boolean).join(" ");
  const selectedPetId = config?.selectedPets?.[sessionId] || config?.selectedPet || pet.id;
  const pending = state.pendingPermission;
  // Auto-expand the panel while a permission request is pending so the user
  // always sees it, even if the bar was collapsed.
  const showPanel = panelOpen || !!pending;

  return (
    <div className="pet-root" data-theme="dark">
      {pending && (
        <div className="permission-overlay">
          <div className="permission-card" role="alertdialog" aria-label="权限请求">
            <div className="permission-head">
              <span className="permission-icon">🔑</span>
              <span className="permission-title">权限请求</span>
            </div>
            <div className="permission-tool">{pending.tool}</div>
            {pending.description ? <div className="permission-desc">{pending.description}</div> : null}
            <div className="permission-actions">
              <button
                className="permission-btn allow"
                disabled={!!permBusy}
                onClick={() => answerPermission("allow")}
              >允许</button>
              <button
                className="permission-btn deny"
                disabled={!!permBusy}
                onClick={() => answerPermission("deny")}
              >拒绝</button>
              {pending.canAutoApprove && (
                <button
                  className="permission-btn session"
                  disabled={!!permBusy}
                  title="允许本次，并把此类工具加入本会话白名单（不再询问）"
                  onClick={() => answerPermission("allow_session")}
                >本会话允许此类</button>
              )}
              <button
                className="permission-btn auto"
                disabled={!!permBusy}
                title="允许本次，且本会话后续所有工具自动允许"
                onClick={() => answerPermission("auto_yes_session")}
              >本会话全部自动</button>
            </div>
            {permBusy ? <div className="permission-busy">已应答，等待 Claude…</div> : null}
          </div>
        </div>
      )}
      <div ref={stackRef} className="pet-stack" style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: 4, marginBottom: 2 }}>
        {showPanel && (
          <div className="pet-panel">
            <div className="head">
              <span className="title">{model}</span>
              <span className="cwd" title={state.session.cwd}>{cwdName}</span>
            </div>
            <div className={`hp ${hp}`}>
              <div className="hp-track">
                <div className="hp-fill" style={{ width: `${typeof usedPct === "number" && isFinite(usedPct) ? Math.min(100, usedPct) : 0}%` }} />
              </div>
            </div>
            <div className="status">
              <span className={`dot ${sev === "error" ? "err" : sev === "warning" ? "warn" : ""}`} />
              <span>{status?.label ?? "ready"}</span>
            </div>
            {status?.detail ? <div className="detail">{status.detail}</div> : null}
            <div className="stats">
              <span>输入 <b>{formatNumber(state.tokens?.sessionInput ?? 0)}</b></span>
              <span>输出 <b>{formatNumber(state.tokens?.sessionOutput ?? 0)}</b></span>
              <span>{gitStr}</span>
            </div>
          </div>
        )}
        <div
          className={barClass}
          onPointerDown={(e) => {
            e.stopPropagation();
            setPanelOpen((o) => !o);
          }}
        >
          <span className={`dot ${sev === "error" ? "err" : sev === "warning" ? "warn" : ""}`} />
          <span className="model">{model}</span>
          <span className="ctx">上下文 <b>{pctText}</b></span>
          <span className="caret">▼</span>
        </div>
      </div>
      <div
        ref={stageRef}
        className="pet-stage"
        onPointerDown={onPointerDown}
        onContextMenu={(e) => {
          e.preventDefault();
          setMenu({ x: e.clientX, y: e.clientY });
        }}
        style={{ cursor: dragging ? "grabbing" : "grab" }}
      >
        <PetCanvas pet={pet} animation={animation} scale={PET_SCALE} />
      </div>

      {menu && (
        <div
          ref={menuRef}
          className="pet-context-menu"
          role="menu"
          aria-label="桌宠菜单"
          style={{ left: menu.x, top: menu.y }}
          onPointerDown={(e) => e.stopPropagation()}
        >
          <button
            className="pet-context-action danger"
            role="menuitem"
            onClick={() => { setMenu(null); closePet(); }}
          >
            ✕ 关闭此宠物
          </button>
          <div className="pet-context-divider" role="separator" />
          <div className="pet-context-title">切换形象</div>
          <div className="pet-context-pets">
            {pets.map((p) => (
              <button
                key={p.id}
                role="menuitemradio"
                aria-checked={p.id === selectedPetId}
                className={`pet-context-pet ${p.id === selectedPetId ? "active" : ""}`}
                title={p.displayName}
                onClick={() => switchPet(p.id)}
              >
                <PetCanvas pet={p} animation="idle" scale={0.16} />
                <span className="pet-context-name">{p.displayName}</span>
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
