// Mirrors `BuddyEvent`, `PopupLook` and `Notice` in src-tauri/src/flow.rs and notice.rs (tested
// there against this JSON shape).

export const BUDDY_EVENT = "buddy";

export type NoticeAction = "open_settings" | "retry";

export interface Notice {
  kind: string;
  title: string;
  detail: string;
  action: NoticeAction | null;
}

/** How the popup comes out of the mascot (see `popup_look` in flow.rs). */
export interface PopupLook {
  /** The mascot's direction, as fractions of the popup: the transform-origin of the bloom. */
  origin_x: number;
  origin_y: number;
  /** The speech-balloon pointer toward the mascot, when it is right next to the popup. */
  tail: "left" | "right" | "top" | "bottom" | null;
  /** CSS pixels along that edge: from the top for left/right, from the left for top/bottom. */
  tail_offset: number;
}

export type BuddyEvent =
  /** `clipboard_offer`: the app's name if the popup offers "Wrong text? Use the clipboard in {app}". */
  | { kind: "start"; id: number; title: string; clipboard_offer: string | null; look: PopupLook }
  | { kind: "chunk"; id: number; text: string }
  | { kind: "done"; id: number; cached: boolean }
  | { kind: "error"; id: number; message: string }
  | { kind: "notice"; id: number; notice: Notice; look: PopupLook }
  /** The model is writing the startup greeting (the mascot thinks). */
  | { kind: "greeting_pending" }
  | { kind: "greeting"; id: number; text: string; look: PopupLook }
  /** The popup window has just appeared: its bloom starts now. */
  | { kind: "shown"; id: number }
  | { kind: "cant_read" }
  | { kind: "hide" };
