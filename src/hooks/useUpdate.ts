import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { UpdatePayload } from "../lib/types";

/**
 * Listen for `claudepet:update` events, keeping only the latest payload whose
 * session id matches `sessionId` (pet windows) or all of them (manager).
 */
export function useUpdate(sessionId: string, mode: "pet" | "manager" = "pet") {
  const [payload, setPayload] = useState<UpdatePayload | null>(null);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<UpdatePayload>("claudepet:update", (e) => {
      const p = e.payload;
      if (mode === "manager" || p.sessionId === sessionId) {
        setPayload(p);
      }
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
    };
  }, [sessionId, mode]);

  return payload;
}
