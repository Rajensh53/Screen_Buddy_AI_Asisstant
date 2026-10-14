import type { BuddyEvent, Notice, PopupLook } from "../shared/events";

/** `notice`: a problem shown instead of an answer. `greeting`: the startup speech bubble. */
export type Status = "thinking" | "streaming" | "done" | "error" | "notice" | "greeting";

export interface AnswerView {
  id: number;
  title: string;
  text: string;
  status: Status;
  look: PopupLook;
  /** The window has appeared: the bloom animation runs. */
  shown: boolean;
  error?: string;
  notice?: Notice;
  /** The app to offer "Use the clipboard in {app}" for, while that hasn't been clicked. */
  clipboardOffer?: string;
}

/** Applies one event. Events for an older selection (stale `id`) are ignored. */
export function reduce(view: AnswerView | null, event: BuddyEvent): AnswerView | null {
  switch (event.kind) {
    case "start":
      return {
        id: event.id,
        title: event.title,
        text: "",
        status: "thinking",
        look: event.look,
        shown: false,
        clipboardOffer: event.clipboard_offer ?? undefined,
      };
    case "notice":
      return {
        id: event.id,
        title: event.notice.title,
        text: "",
        status: "notice",
        look: event.look,
        shown: false,
        notice: event.notice,
      };
    case "greeting":
      return { id: event.id, title: "", text: event.text, status: "greeting", look: event.look, shown: false };
    case "shown":
      return view?.id === event.id ? { ...view, shown: true } : view;
    case "chunk":
      return view?.id === event.id ? { ...view, text: view.text + event.text, status: "streaming" } : view;
    case "done":
      return view?.id === event.id ? { ...view, status: "done" } : view;
    case "error":
      return view?.id === event.id ? { ...view, status: "error", error: event.message } : view;
    case "hide":
      return null;
    case "cant_read":
    case "greeting_pending":
      return view;
  }
}

export interface Line {
  label?: string;
  value: string;
  /** Contains Urdu (Arabic script): shown right-to-left. */
  rtl: boolean;
}

const ARABIC_SCRIPT = /[؀-ۿݐ-ݿﭐ-﷿ﹰ-﻿]/;
/** Small models sometimes write an empty Note anyway ("Note: None"); such lines are hidden. */
const EMPTY_VALUE = /^(none|n\/a|nothing|-|—)\.?$/i;
/** "Meaning (simple English): ..." -> label + value. */
const LABELED = /^([A-Za-z][A-Za-z ()/-]{0,40}):\s*(.*)$/;

/** Splits the model's answer into lines, with an optional "Label:" prefix. */
export function parseAnswer(text: string): Line[] {
  return text
    .split("\n")
    .map((raw) => raw.replace(/\*\*/g, "").trim())
    .filter((line) => line.length > 0)
    .map((line) => {
      const match = LABELED.exec(line);
      const label = match?.[1].trim();
      const value = (match ? match[2] : line).trim();
      return { label, value, rtl: ARABIC_SCRIPT.test(value) };
    })
    .filter((line) => !(line.label && EMPTY_VALUE.test(line.value)));
}
