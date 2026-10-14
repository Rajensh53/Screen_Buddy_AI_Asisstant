import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { BUDDY_EVENT, type BuddyEvent } from "../shared/events";
import Robot, { type FlightLook, type Glitch, type Play } from "./Robot";
import { TURN_MS } from "./turn";

/**
 * Character states, driven by `buddy` events. The last four are short reactions to notices,
 * shown at home (the mascot only flies for answers). Paused, flight, idle play and the hologram
 * glitches (only at start and on Quit) are separate.
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

/** Wide eyes, the radar sweep and the "!" (mascot.css), before thinking. */
const NOTICED_MS = 450;
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
const PLAY_MS: Record<Play, number> = {
  wink: 900,
  yawn: 2000,
  glance: 1800,
  turn: TURN_MS,
  patrol: 2400,
  greet: 1800,
};
const PLAYS: Play[] = ["wink", "yawn", "glance", "turn", "patrol"];

/** After this long with nothing happening, the endless animations (hover, glow, breathing) pause
 * where they are: a moving transparent window costs the GPU every frame. Anything wakes it. */
const REST_AFTER_MS = 20_000;

/** How long the boot glitch runs (mascot.css). The exit one stays: the app is closing. */
const BOOT_MS = 850;
/** Sent by the tray's Quit just before the app closes (src-tauri/src/tray.rs). */
const MASCOT_EXIT = "mascot-exit";

const randomDelay = () => PLAY_AFTER_MS[0] + Math.random() * (PLAY_AFTER_MS[1] - PLAY_AFTER_MS[0]);
const randomPlay = () => PLAYS[Math.floor(Math.random() * PLAYS.length)];
const reducedMotion = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/** Debug builds: the tray's "Preview mascot states" shows each look for this long. */
const PREVIEW_STEP_MS = 2000;
type Look = { state: MascotState; paused: boolean; flight: FlightLook; play: Play | null; glitch: Glitch | null };
const look = (state: MascotState, extra: Partial<Look> = {}): Look => ({
  state,
  paused: false,
  flight: GROUNDED,
  play: null,
  glitch: null,
  ...extra,
});
const PREVIEW: Look[] = [
  look("idle", { glitch: "boot" }),
  look("idle"),
  look("noticed"),
  look("noticed", { flight: { flying: true, direction: "left" } }),
  look("thinking", { flight: { flying: true, direction: "right" } }),
  look("thinking"),
  look("talking"),
  look("idle", { play: "greet" }),
  look("cant_read"),
  look("shy"),
  look("refuse"),
  look("warning"),
  look("offline"),
  look("idle", { play: "wink" }),
  look("idle", { play: "yawn" }),
  look("idle", { play: "glance" }),
  look("idle", { play: "turn" }),
  look("idle", { play: "patrol" }),
  look("idle", { paused: true }),
  look("idle", { glitch: "exit" }),
  look("idle", { glitch: "boot" }),
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
  const [greeting, setGreeting] = useState(false);
  // The robot appears as a hologram at start.
  const [glitch, setGlitch] = useState<Glitch | null>("boot");
  // Bumped by anything real (an event, a grab): idle play stops at once and its wait restarts.
  const [activity, setActivity] = useState(0);
  const [preview, setPreview] = useState<Look | null>(null);
  const [resting, setResting] = useState(false);
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
    // Quit: the robot glitches away into a beam of light.
    const unlistenExit = listen(MASCOT_EXIT, () => setGlitch("exit"));
    return () => {
      unlistenPaused.then((stop) => stop());
      unlistenSettings.then((stop) => stop());
      unlistenFlight.then((stop) => stop());
      unlistenExit.then((stop) => stop());
    };
  }, []);

  // The boot glitch ends by itself; the exit one stays. Glitches only ever run at start and on
  // Quit, never between states.
  useEffect(() => {
    if (glitch !== "boot") return;
    const handle = window.setTimeout(() => setGlitch((g) => (g === "boot" ? null : g)), BOOT_MS);
    return () => window.clearTimeout(handle);
  }, [glitch]);

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
          // Happy eyes and a wink while the greeting bubble shows.
          setGreeting(true);
          window.setTimeout(() => setGreeting(false), PLAY_MS.greet);
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

  // Rest after a quiet while; anything real wakes it (and restarts the wait). Idle play doesn't:
  // after a wink or a turn the robot goes straight back to resting.
  useEffect(() => {
    setResting(false);
    if (state !== "idle" || flight.flying || greeting || glitch) return;
    const handle = window.setTimeout(() => setResting(true), REST_AFTER_MS);
    return () => window.clearTimeout(handle);
  }, [state, paused, flight.flying, greeting, glitch, activity]);

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

  const live: Look = { state, paused, flight, play: greeting && !paused ? "greet" : play, glitch };
  const shown = preview ?? live;

  return (
    <div className="mascot">
      <Robot
        // A new key restarts the CSS animations of each previewed look.
        key={preview ? PREVIEW.indexOf(preview) : "live"}
        state={shown.state}
        paused={shown.paused}
        flight={shown.flight}
        play={shown.play}
        glitch={shown.glitch}
        resting={!preview && resting}
      />
      {/* Above the drawing (which ignores the mouse): every pixel drags the mascot. Tauri only
          starts a drag on an element that carries the attribute itself. */}
      <div className="mascot__hit" data-tauri-drag-region onMouseDown={grab} onContextMenu={openMenu} />
    </div>
  );
}
