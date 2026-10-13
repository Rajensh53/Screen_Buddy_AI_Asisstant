import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { BUDDY_EVENT, type BuddyEvent } from "../shared/events";
import Robot, { type FlightLook } from "./Robot";

/**
 * Character states, driven by `buddy` events. The last four are short reactions to notices,
 * shown at home (the mascot only flies for answers). Paused and flight are separate.
 */
export type MascotState =
  | "idle"
  | "noticed"
  | "thinking"
  | "talking"
  | "cant_read"
  | "shy"
  | "refuse"
  | "warning"
  | "offline";

const NOTICED_MS = 300;
const DONE_IDLE_MS = 2000;
const REACTION_MS = 2500;

/** Which reaction a notice gets (`NoticeKind` in src-tauri/src/notice.rs). */
const NOTICE_REACTIONS: Record<string, MascotState> = {
  cant_read: "cant_read",
  nothing_selected: "cant_read",
  password_field: "shy",
  blocked_app: "refuse",
  clipboard_sync: "warning",
  hook_failed: "warning",
  uia_unavailable: "warning",
  ollama_not_running: "offline",
  model_missing: "offline",
};

const GROUNDED: FlightLook = { flying: false, direction: "none" };

/** Debug builds: the tray's "Preview mascot states" shows each look for this long. */
const PREVIEW_STEP_MS = 2000;
type Look = { state: MascotState; paused: boolean; flight: FlightLook };
const PREVIEW: Look[] = [
  { state: "idle", paused: false, flight: GROUNDED },
  { state: "noticed", paused: false, flight: GROUNDED },
  { state: "noticed", paused: false, flight: { flying: true, direction: "left" } },
  { state: "thinking", paused: false, flight: { flying: true, direction: "right" } },
  { state: "thinking", paused: false, flight: GROUNDED },
  { state: "talking", paused: false, flight: GROUNDED },
  { state: "cant_read", paused: false, flight: GROUNDED },
  { state: "shy", paused: false, flight: GROUNDED },
  { state: "refuse", paused: false, flight: GROUNDED },
  { state: "warning", paused: false, flight: GROUNDED },
  { state: "offline", paused: false, flight: GROUNDED },
  { state: "idle", paused: true, flight: GROUNDED },
];

export default function Mascot() {
  const [state, setState] = useState<MascotState>("idle");
  const [paused, setPaused] = useState(false);
  const [flight, setFlight] = useState<FlightLook>(GROUNDED);
  const [preview, setPreview] = useState<Look | null>(null);
  const timer = useRef<number | undefined>(undefined);

  useEffect(() => {
    // Paused may be saved from the last run.
    invoke<boolean>("is_paused").then(setPaused).catch(() => {});
    const unlistenPaused = listen<boolean>("paused-changed", (event) => setPaused(event.payload));
    // Take-off and landing of the flight to the popup (src-tauri/src/windows/flight.rs).
    const unlistenFlight = listen<FlightLook>("mascot-flight", (event) => setFlight(event.payload));
    return () => {
      unlistenPaused.then((stop) => stop());
      unlistenFlight.then((stop) => stop());
    };
  }, []);

  useEffect(() => {
    /** Switches to `now`, then to `next` after `ms`. */
    const show = (now: MascotState, next?: MascotState, ms = 0) => {
      window.clearTimeout(timer.current);
      setState(now);
      if (next) timer.current = window.setTimeout(() => setState(next), ms);
    };

    const unlisten = listen<BuddyEvent>(BUDDY_EVENT, ({ payload }) => {
      switch (payload.kind) {
        case "start":
          return show("noticed", "thinking", NOTICED_MS);
        case "chunk":
          return show("talking");
        case "done":
          return show("talking", "idle", DONE_IDLE_MS);
        case "error":
          return show("warning", "idle", REACTION_MS);
        case "notice":
          return show(NOTICE_REACTIONS[payload.notice.kind] ?? "cant_read", "idle", REACTION_MS);
        case "cant_read":
          return show("cant_read", "idle", REACTION_MS);
        case "hide":
          return show("idle");
      }
    });
    return () => {
      window.clearTimeout(timer.current);
      unlisten.then((stop) => stop());
    };
  }, []);

  // Debug builds only: cycle through every look once, then back to the real state.
  useEffect(() => {
    if (!import.meta.env.DEV) return;
    const timers: number[] = [];
    const unlisten = listen("mascot-preview", () => {
      timers.forEach((t) => window.clearTimeout(t));
      timers.length = 0;
      PREVIEW.forEach((step, i) => timers.push(window.setTimeout(() => setPreview(step), i * PREVIEW_STEP_MS)));
      timers.push(window.setTimeout(() => setPreview(null), PREVIEW.length * PREVIEW_STEP_MS));
    });
    return () => {
      timers.forEach((t) => window.clearTimeout(t));
      unlisten.then((stop) => stop());
    };
  }, []);

  // A left press may start a drag: any flight stops, and the drop spot becomes the new home.
  const grab = (e: React.MouseEvent) => {
    if (e.button === 0) invoke("mascot_grabbed").catch(() => {});
  };

  // Right-click: Pause/Resume, Settings, Quit (the tray's menu).
  const openMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    invoke("show_mascot_menu").catch(() => {});
  };

  const shown = preview ?? { state, paused, flight };

  return (
    <div className="mascot">
      <Robot state={shown.state} paused={shown.paused} flight={shown.flight} />
      {/* Above the drawing (which ignores the mouse): every pixel drags the mascot. Tauri only
          starts a drag on an element that carries the attribute itself. */}
      <div className="mascot__hit" data-tauri-drag-region onMouseDown={grab} onContextMenu={openMenu} />
    </div>
  );
}
