// Mirrors `BuddyEvent` and `Notice` in src-tauri/src/flow.rs and notice.rs (tested there
// against this JSON shape).

export const BUDDY_EVENT = "buddy";

export type NoticeAction = "open_settings" | "retry";

export interface Notice {
  kind: string;
  title: string;
  detail: string;
  action: NoticeAction | null;
}

export type BuddyEvent =
  /** `clipboard_offer`: the app's name if the popup offers "Wrong text? Use the clipboard in {app}". */
  | { kind: "start"; id: number; title: string; clipboard_offer: string | null }
  | { kind: "chunk"; id: number; text: string }
  | { kind: "done"; id: number; cached: boolean }
  | { kind: "error"; id: number; message: string }
  | { kind: "notice"; id: number; notice: Notice }
  | { kind: "cant_read" }
  | { kind: "hide" };
