import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

/** Character states. Only `idle` is used until the selection flow is wired (Task 6). */
export type MascotState = "idle" | "noticed" | "thinking" | "talking" | "cant_read";

export default function Mascot() {
  const [state] = useState<MascotState>("idle");
  const [paused, setPaused] = useState(false);

  useEffect(() => {
    const unlisten = listen<boolean>("paused-changed", (event) => setPaused(event.payload));
    return () => {
      unlisten.then((stop) => stop());
    };
  }, []);

  return (
    // The whole window is a drag handle; children must carry the attribute too.
    <div className={`mascot mascot--${state}${paused ? " mascot--paused" : ""}`} data-tauri-drag-region>
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
