import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { BUDDY_EVENT, type BuddyEvent, type NoticeAction } from "../shared/events";
import { type AnswerView, parseAnswer, reduce } from "./answer";

/** Space around the card (its CSS margin), part of the window size. */
const OUTER_MARGIN = 12;
/** After this long without an answer, explain that the model is loading. */
const SLOW_AFTER_MS = 5000;
/** How long "Copied ✓" stays. */
const COPIED_MS = 1500;

const ACTION_LABELS: Record<NoticeAction, string> = {
  open_settings: "Open Settings",
  retry: "Retry",
};

// Model output is rendered as plain text only (React text nodes, never innerHTML).
// The popup never takes focus; its buttons still work because clicks don't need focus.
export default function Popup() {
  const [view, setView] = useState<AnswerView | null>(null);
  const [slow, setSlow] = useState(false);
  const [copied, setCopied] = useState(false);
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

  // "Loading the model…" when the first piece takes long (the model is loaded on first use).
  const thinkingId = view?.status === "thinking" ? view.id : null;
  useEffect(() => {
    setSlow(false);
    setCopied(false);
    if (thinkingId === null) return;
    const timer = window.setTimeout(() => setSlow(true), SLOW_AFTER_MS);
    return () => window.clearTimeout(timer);
  }, [thinkingId]);

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
      const complete = ["done", "error", "notice"].includes(current.status);
      invoke("popup_resize", { id: current.id, height, complete }).catch(() => {});
    });
  };

  useLayoutEffect(reportHeight, [view, slow, copied]);

  useEffect(() => {
    const content = contentRef.current;
    if (!content) return;
    const observer = new ResizeObserver(reportHeight);
    observer.observe(content);
    return () => observer.disconnect();
  }, []);

  const close = () => invoke("popup_close").catch(() => {});
  const retry = () => invoke("popup_retry").catch(() => {});
  // Hides the link at once; the re-read answer arrives as a new selection.
  const useClipboard = () => {
    setView((current) => (current ? { ...current, clipboardOffer: undefined } : current));
    invoke("popup_use_clipboard").catch(() => {});
  };
  const runAction = (action: NoticeAction) =>
    invoke(action === "retry" ? "popup_retry" : "popup_open_settings").catch(() => {});
  const copy = () => {
    if (!view) return;
    invoke("popup_copy", { text: view.text.trim() })
      .then(() => {
        setCopied(true);
        window.setTimeout(() => setCopied(false), COPIED_MS);
      })
      .catch(() => {});
  };

  const lines = view ? parseAnswer(view.text) : [];

  return (
    <div className="popup" ref={cardRef}>
      <div className="popup__content" ref={contentRef}>
        {view && (
          // Keyed by selection, so each new popup fades in once.
          <div key={view.id} className="popup__enter">
            <div className="popup__header">
              <div
                className={view.status === "notice" ? "popup__title popup__title--notice" : "popup__title"}
                dir="auto"
              >
                {view.title}
              </div>
              {view.status === "done" && (
                <button type="button" className="popup__icon-button" onClick={copy} title="Copy the answer">
                  {copied ? "Copied ✓" : "Copy"}
                </button>
              )}
              <button type="button" className="popup__icon-button" onClick={close} aria-label="Close" title="Close">
                ×
              </button>
            </div>

            {view.status === "thinking" && (
              <p className="popup__muted">
                {slow ? "Loading the model — the first answer can take a few seconds…" : "Thinking…"}
              </p>
            )}

            {lines.map((line, i) => (
              <div key={i} className={line.rtl ? "popup__line popup__line--rtl" : "popup__line"}>
                {line.label && <span className="popup__label">{line.label}</span>}
                <span className="popup__value" dir={line.rtl ? "rtl" : "auto"}>
                  {line.value}
                </span>
              </div>
            ))}

            {view.status === "error" && (
              <div className="popup__problem">
                <p className="popup__error">{view.error}</p>
                <button type="button" className="popup__action" onClick={retry}>
                  Retry
                </button>
              </div>
            )}

            {view.status === "done" && view.clipboardOffer && (
              <p className="popup__footer">
                Wrong text?{" "}
                <button type="button" className="popup__link" onClick={useClipboard}>
                  Use the clipboard in {view.clipboardOffer}
                </button>
              </p>
            )}

            {view.status === "notice" && view.notice && (
              <div className="popup__problem">
                <p className="popup__muted">{view.notice.detail}</p>
                {view.notice.action && (
                  <button
                    type="button"
                    className="popup__action"
                    onClick={() => view.notice?.action && runAction(view.notice.action)}
                  >
                    {ACTION_LABELS[view.notice.action]}
                  </button>
                )}
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
