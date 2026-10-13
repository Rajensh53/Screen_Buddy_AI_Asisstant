import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { BUDDY_EVENT, type BuddyEvent } from "../shared/events";
import { type AnswerView, parseAnswer, reduce } from "./answer";

/** Space around the card (its CSS margin), part of the window size. */
const OUTER_MARGIN = 12;

// Model output is rendered as plain text only (React text nodes, never innerHTML).
export default function Popup() {
  const [view, setView] = useState<AnswerView | null>(null);
  const cardRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const reportPending = useRef(false);
  // The current view, read by the size report (which runs after the render).
  const viewRef = useRef<AnswerView | null>(null);
  viewRef.current = view;

  useEffect(() => {
    const unlisten = listen<BuddyEvent>(BUDDY_EVENT, ({ payload }) => {
      setView((current) => reduce(current, payload));
    });
    return () => {
      unlisten.then((stop) => stop());
    };
  }, []);

  // Tell Rust how tall the content is. Rust decides the window size (it grows in steps while
  // the answer streams, to avoid repeated resizes) and shows the popup on the first report for
  // a new selection, so it never flashes the previous answer.
  // A microtask, not requestAnimationFrame: the window is still hidden at that point, and hidden
  // pages get no animation frames. Several changes in one task give one report.
  const reportHeight = () => {
    if (reportPending.current) return;
    reportPending.current = true;
    queueMicrotask(() => {
      reportPending.current = false;
      const card = cardRef.current;
      const content = contentRef.current;
      const current = viewRef.current;
      if (!card || !content || !current) return;
      // The card fills the window, so measure what is inside it.
      const style = getComputedStyle(card);
      const padding = parseFloat(style.paddingTop) + parseFloat(style.paddingBottom);
      const borders = card.offsetHeight - card.clientHeight;
      const height = content.offsetHeight + padding + borders + OUTER_MARGIN;
      const complete = current.status === "done" || current.status === "error";
      invoke("popup_resize", { id: current.id, height, complete }).catch(() => {});
    });
  };

  useLayoutEffect(reportHeight, [view]);

  useEffect(() => {
    const content = contentRef.current;
    if (!content) return;
    const observer = new ResizeObserver(reportHeight);
    observer.observe(content);
    return () => observer.disconnect();
  }, []);

  const lines = view ? parseAnswer(view.text) : [];

  return (
    <div className="popup" ref={cardRef}>
      <div className="popup__content" ref={contentRef}>
        {view && (
          // Keyed by selection, so each new popup fades in once.
          <div key={view.id} className="popup__enter">
            <div className="popup__title" dir="auto">
              {view.title}
            </div>
            {view.status === "thinking" && <p className="popup__muted">Thinking…</p>}
            {lines.map((line, i) => (
              <div key={i} className={line.rtl ? "popup__line popup__line--rtl" : "popup__line"}>
                {line.label && <span className="popup__label">{line.label}</span>}
                <span className="popup__value" dir={line.rtl ? "rtl" : "auto"}>
                  {line.value}
                </span>
              </div>
            ))}
            {view.status === "error" && <p className="popup__error">{view.error}</p>}
          </div>
        )}
      </div>
    </div>
  );
}
