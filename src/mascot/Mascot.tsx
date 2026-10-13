import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { BUDDY_EVENT, type BuddyEvent } from "../shared/events";

/** Character states, driven by `buddy` events from the selection flow. */
export type MascotState = "idle" | "noticed" | "thinking" | "talking" | "cant_read";

const NOTICED_MS = 300;
const DONE_IDLE_MS = 2000;
const CANT_READ_MS = 1500;

export default function Mascot() {
  const [state, setState] = useState<MascotState>("idle");
  const [paused, setPaused] = useState(false);
  const timer = useRef<number | undefined>(undefined);

  useEffect(() => {
    // Paused may be saved from the last run.
    invoke<boolean>("is_paused").then(setPaused).catch(() => {});
    const unlisten = listen<boolean>("paused-changed", (event) => setPaused(event.payload));
    return () => {
      unlisten.then((stop) => stop());
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
        case "notice":
        case "cant_read":
          // Kept short and subtle: some apps simply don't expose their text.
          return show("cant_read", "idle", CANT_READ_MS);
        case "hide":
          return show("idle");
      }
    });
    return () => {
      window.clearTimeout(timer.current);
      unlisten.then((stop) => stop());
    };
  }, []);

  // Right-click: Pause/Resume, Settings, Quit (the tray's menu). Left-drag still moves the mascot.
  const openMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    invoke("show_mascot_menu").catch(() => {});
  };

  return (
    // The whole window is a drag handle; children must carry the attribute too.
    <div
      className={`mascot mascot--${state}${paused ? " mascot--paused" : ""}`}
      data-tauri-drag-region
      onContextMenu={openMenu}
    >
      <div className="mascot__body" data-tauri-drag-region>
        <div className="mascot__eyes" data-tauri-drag-region>
          <span className="mascot__eye" data-tauri-drag-region />
          <span className="mascot__eye" data-tauri-drag-region />
        </div>
        <span className="mascot__mouth" data-tauri-drag-region />
      </div>
    </div>
  );
}
