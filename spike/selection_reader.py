"""
Selection Reader - prototype (Windows)

Text ko mouse se select karo (blue highlight), mouse chhoro ->
agent selected text khud padh lega. Clipboard ya screenshot use nahi hota.

Tareeqa: Windows UI Automation (UIA) -> TextPattern.GetSelection()
Yeh wohi system hai jis se screen readers (NVDA, Narrator) text padhte hain.

Install:  pip install uiautomation pynput ollama
Run:      python selection_reader.py   (Ollama background mein chal raha ho)
"""
import math
import queue
import time

import ollama
import uiautomation as auto
from pynput import mouse

MODEL = "qwen3.5:4b"          # 6 GB GPU mein poora fit; compare_models.py ke baad badal sakte ho
TARGET = "simple Roman Urdu"

SYSTEM = f"""You are a pocket translator. Reply in {TARGET}. Be short.
If the input is a single word or short phrase:
  Meaning: <1 line>
  Type: <noun/verb/adjective/...>
  Example: <1 short sentence in the original language>
If the input is a sentence or paragraph:
  Translation: <translation>
  Matlab: <1 line simple explanation>
No extra text."""

_cache: dict[str, str] = {}   # same text dobara aaye to model call nahi


def explain(text: str) -> None:
    """Selected text ko local model se translate/explain karo (streaming)."""
    key = text.strip().lower()
    if key in _cache:
        print(_cache[key])
        return

    kwargs = dict(
        model=MODEL,
        messages=[{"role": "system", "content": SYSTEM},
                  {"role": "user", "content": text[:2000]}],
        options={"temperature": 0.2},
        keep_alive="30m",     # model memory mein rahe, har dafa load na ho
        stream=True,
    )
    try:
        stream = ollama.chat(think=False, **kwargs)
    except Exception:
        stream = ollama.chat(**kwargs)

    out = []
    for chunk in stream:
        piece = chunk["message"]["content"]
        out.append(piece)
        print(piece, end="", flush=True)
    print()
    _cache[key] = "".join(out)

DRAG_MIN_PX = 6           # itna drag ho to "selection" samjho
DOUBLE_CLICK_MS = 400     # double-click = word selection
MAX_PARENT_DEPTH = 15     # UIA tree mein kitna upar tak TextPattern dhoondna hai

# Fallback: agent khud chupke se Ctrl+C dabata hai aur purana clipboard wapas rakhta hai.
# Default OFF. Terminal mein Ctrl+C process band kar deta hai, is liye dhyan se on karna.
USE_CLIPBOARD_FALLBACK = False

events: "queue.Queue[tuple[int, int]]" = queue.Queue()
_state = {"down_pos": None, "last_click_t": 0.0}


def on_click(x, y, button, pressed):
    """Mouse hook: sirf detect karta hai ke selection hua ya nahi."""
    if button != mouse.Button.left:
        return
    now = time.time()
    if pressed:
        _state["down_pos"] = (x, y)
        return

    down = _state["down_pos"] or (x, y)
    dragged = math.hypot(x - down[0], y - down[1]) >= DRAG_MIN_PX
    double = (now - _state["last_click_t"]) * 1000 <= DOUBLE_CLICK_MS
    _state["last_click_t"] = now

    if dragged or double:
        events.put((x, y))  # asal kaam main thread karega


def _selection_from(start) -> str | None:
    """Element se upar ki taraf chalte hue TextPattern dhoondo aur selection padho."""
    el, depth = start, 0
    while el and depth < MAX_PARENT_DEPTH:
        try:
            tp = el.GetPattern(auto.PatternId.TextPattern)
        except Exception:
            tp = None
        if tp:
            try:
                ranges = tp.GetSelection()
                text = " ".join(r.GetText(-1) for r in ranges).strip()
                if text:
                    return text
            except Exception:
                pass
        try:
            el = el.GetParentControl()
        except Exception:
            break
        depth += 1
    return None


def read_selection_uia(x: int, y: int) -> str | None:
    """Pehle focused element, phir cursor ke neeche wala element check karo."""
    starts = []
    try:
        starts.append(auto.GetFocusedControl())
    except Exception:
        pass
    try:
        starts.append(auto.ControlFromPoint(x, y))
    except Exception:
        pass

    for start in starts:
        if start is None:
            continue
        text = _selection_from(start)
        if text:
            return text
    return None


def read_selection_clipboard() -> str | None:
    """Fallback: user copy nahi karta, agent karta hai, aur clipboard restore karta hai.
    Note: sirf text restore hota hai; agar clipboard mein image thi to woh chali jayegi."""
    old = auto.GetClipboardText() or ""
    auto.SetClipboardText("")
    auto.SendKeys("{Ctrl}c", waitTime=0)
    time.sleep(0.12)
    text = (auto.GetClipboardText() or "").strip()
    if old:
        auto.SetClipboardText(old)
    return text or None


def main():
    listener = mouse.Listener(on_click=on_click)
    listener.start()
    print("Ready. Kisi bhi app mein text select karo... (band karne ke liye Ctrl+C)")

    try:
        while True:
            x, y = events.get()
            time.sleep(0.08)  # app ko selection update karne ka waqt do

            text, source = read_selection_uia(x, y), "UIA"
            if not text and USE_CLIPBOARD_FALLBACK:
                text, source = read_selection_clipboard(), "clipboard"

            if text:
                print(f"\n[{source}] {text[:200]}")
                print("-" * 40)
                explain(text)
            else:
                print("\n[x] Is app se selection nahi mili")
    except KeyboardInterrupt:
        listener.stop()


if __name__ == "__main__":
    main()
