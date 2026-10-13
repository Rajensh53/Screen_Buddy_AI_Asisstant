// Mirrors `BuddyEvent` in src-tauri/src/flow.rs (tested there against this JSON shape).

export const BUDDY_EVENT = "buddy";

export type BuddyEvent =
  | { kind: "start"; id: number; title: string }
  | { kind: "chunk"; id: number; text: string }
  | { kind: "done"; id: number; cached: boolean }
  | { kind: "error"; id: number; message: string }
  | { kind: "cant_read" }
  | { kind: "hide" };
