import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { BUDDY_EVENT, type BuddyEvent } from "../shared/events";
import Robot, { type FlightLook, type Play } from "./Robot";

/**
 * Character states, driven by `buddy` events. The last four are short reactions to notices,
 * shown at home (the mascot only flies for answers). Paused, flight and idle play are separate.
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

/** Idle play: after a random 45-90 s of quiet, one short behaviour (CSS in mascot.css). */
const PLAY_AFTER_MS: [number, number] = [45_000, 90_000];
const PLAY_MS: Record<Play, number> = { wave: 1600, spin: 1100, patrol: 2400, stretch: 1400 };
const PLAYS: Play[] = ["wave", "spin", "patrol", "stretch"];
/** The wave that goes with the startup greeting. */
const GREETING_WAVE_MS = 1800;

const randomDelay = () => PLAY_AFTER_MS[0] + Math.random() * (PLAY_AFTER_MS[1] - PLAY_AFTER_MS[0]);
const randomPlay = () => PLAYS[Math.floor(Math.random() * PLAYS.length)];
const reducedMotion = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/** Debug builds: the tray's "Preview mascot states" shows each look for this long. */
const PREVIEW_STEP_MS = 2000;
type Look = { state: MascotState; paused: boolean; flight: FlightLook; play: Play | null };
const look = (state: MascotState, extra: Partial<Look> = {}): Look => ({
  state,
  paused: false,
  flight: GROUNDED,
  play: null,
  ...extra,
});
const PREVIEW: Look[] = [
  look("idle"),
  look("noticed"),
  look("noticed", { flight: { flying: true, direction: "left" } }),
  look("thinking", { flight: { flying: true, direction: "right" } }),
  look("thinking"),
  look("talking"),
  look("cant_read"),
  look("shy"),
  look("refuse"),
  look("warning"),
  look("offline"),
  look("idle", { play: "wave" }),
  look("idle", { play: "spin" }),
  look("idle", { play: "patrol" }),
  look("idle", { play: "stretch" }),
  look("idle", { paused: true }),
];

interface MascotSettings {
  paused: boolean;
  idlePlay: boolean;
}

export default function Mascot() {
  const [state, setState] = useState<MascotState>("idle");
  const [paused, setPaused] = useState(false);
  const [idlePlay, setIdlePlay] = useState(true);
  const [flight, setFlight] = useState<FlightLook>(GROUNDED);
  const [play, setPlay] = useState<Play | null>(null);
  const [greetingWave, setGreetingWave] = useState(false);
  // Bumped by anything real (an event, a grab): idle play stops at once and its wait restarts.
  const [activity, setActivity] = useState(0);
  const [preview, setPreview] = useState<Look | null>(null);
  const timer = useRef<number | undefined>(undefined);

  useEffect(() => {
    // Paused may be saved from the last run; idle play can be off in the settings.
    invoke<MascotSettings>("mascot_settings")
      .then((s) => {
        setPaused(s.paused);
        setIdlePlay(s.idlePlay);
      })
      .catch(() => {});
    const unlistenPaused = listen<boolean>("paused-changed", (event) => setPaused(event.payload));
    const unlistenSettings = listen<{ idlePlay: boolean }>("settings-changed", (event) =>
      setIdlePlay(event.payload.idlePlay),
    );
    // Take-off and landing of the flight to the popup (src-tauri/src/windows/flight.rs).
    const unlistenFlight = listen<FlightLook>("mascot-flight", (event) => {
      setFlight(event.payload);
      setActivity((n) => n + 1);
    });
    return () => {
      unlistenPaused.then((stop) => stop());
      unlistenSettings.then((stop) => stop());
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
      setActivity((n) => n + 1);
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
        case "greeting_pending":
          // The model is writing the greeting.
          return show("thinking");
        case "greeting":
          // Waves hello while the greeting bubble shows.
          setGreetingWave(true);
          window.setTimeout(() => setGreetingWave(false), GREETING_WAVE_MS);
          return show("idle");
        case "hide":
          return show("idle");
        case "shown":
          return;
      }
    });
    return () => {
      window.clearTimeout(timer.current);
      unlisten.then((stop) => stop());
    };
  }, []);

  // Idle play: only while idle at home, awake, allowed, and with motion welcome. Any activity
  // re-runs this effect, which stops a running behaviour at once and restarts the wait.
  useEffect(() => {
    setPlay(null);
    if (state !== "idle" || paused || flight.flying || !idlePlay || reducedMotion()) return;
    let handle = 0;
    const wait = () => {
      handle = window.setTimeout(() => {
        const next = randomPlay();
        setPlay(next);
        handle = window.setTimeout(() => {
          setPlay(null);
          wait();
        }, PLAY_MS[next]);
      }, randomDelay());
    };
    wait();
    return () => window.clearTimeout(handle);
  }, [state, paused, flight.flying, idlePlay, activity]);

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

  // A left press may start a drag: any flight or play stops, and the drop spot becomes the new home.
  const grab = (e: React.MouseEvent) => {
    if (e.button !== 0) return;
    setActivity((n) => n + 1);
    invoke("mascot_grabbed").catch(() => {});
  };

  // Right-click: Pause/Resume, Settings, Quit (the tray's menu).
  const openMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    invoke("show_mascot_menu").catch(() => {});
  };

  const live: Look = { state, paused, flight, play: greetingWave && !paused ? "wave" : play };
  const shown = preview ?? live;

  return (
    <div className="mascot">
      <Robot state={shown.state} paused={shown.paused} flight={shown.flight} play={shown.play} />
      {/* Above the drawing (which ignores the mouse): every pixel drags the mascot. Tauri only
          starts a drag on an element that carries the attribute itself. */}
      <div className="mascot__hit" data-tauri-drag-region onMouseDown={grab} onContextMenu={openMenu} />
    </div>
  );
}
