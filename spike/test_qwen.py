"""
Test qwen3.5:4b on your 30 real inputs (test_inputs.txt).

Works with local Ollama now, and with a hosted Ollama server later
without code changes - only the environment variables change.

Install:  pip install ollama
Local:    ollama pull qwen3.5:4b
          python test_qwen.py
Hosted:   $env:OLLAMA_HOST = "https://your-server"     (PowerShell)
          $env:OLLAMA_API_KEY = "your-key"             (only if the server needs one)
          python test_qwen.py

Output:   results_qwen.csv -> open in Excel, fill the 'score' column:
          0 = wrong, 1 = understandable but weak, 2 = good
"""
import csv
import os
import sys
import time
from pathlib import Path

from ollama import Client

MODEL = "qwen3.5:4b"
HOST = os.environ.get("OLLAMA_HOST", "http://127.0.0.1:11434")
API_KEY = os.environ.get("OLLAMA_API_KEY", "")

HERE = Path(__file__).resolve().parent
INPUTS_FILE = HERE / "test_inputs.txt"
OUT_FILE = HERE / "results_qwen.csv"

SYSTEM = """You are a friendly pocket explainer for a Pakistani user.
Detect the input language yourself. Keep answers short.
If the input is a single word or short phrase:
  Meaning (simple English): <1 line>
  Urdu: <meaning in Urdu script>
  Type: <noun/verb/adjective/...>
  Example: <1 short sentence in the input's language>
If the input is a sentence:
  Simple English: <translation or plain rewording>
  Urdu: <translation in Urdu script>
  Note: <only if there is an idiom or slang; otherwise leave this line out>
Roman Urdu is Urdu written in English letters; treat it as Urdu.
No extra text."""

headers = {"Authorization": f"Bearer {API_KEY}"} if API_KEY else None
client = Client(host=HOST, headers=headers)


def load_inputs(path: Path) -> list[tuple[str, str]]:
    """'# Heading' lines set the category for the lines below them."""
    items, category = [], "uncategorized"
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line:
            continue
        if line.startswith("#"):
            category = line.lstrip("#").strip()
            continue
        items.append((category, line))
    return items


def check_model() -> None:
    """Fail early with a clear message instead of 30 identical errors."""
    try:
        names = [m.model for m in client.list().models]
    except Exception as e:
        sys.exit(f"Cannot reach Ollama at {HOST}\n  {e}\n"
                 "Local: start the Ollama app.  Hosted: check OLLAMA_HOST and OLLAMA_API_KEY.")
    if not any(n == MODEL or n.startswith(MODEL + "-") for n in names):
        sys.exit(f"Model {MODEL} not found on {HOST}. Run:  ollama pull {MODEL}")


def ask(text: str) -> tuple[str, float, float]:
    """Stream one answer. Returns (answer, seconds to first word, total seconds).
    First-word time matters most: it is how long the popup stays empty."""
    kwargs = dict(
        model=MODEL,
        messages=[{"role": "system", "content": SYSTEM},
                  {"role": "user", "content": text}],
        options={"temperature": 0.2},
        keep_alive="10m",
        stream=True,
    )
    for use_think_flag in (True, False):
        t0 = time.perf_counter()
        first, parts = None, []
        try:
            stream = client.chat(think=False, **kwargs) if use_think_flag else client.chat(**kwargs)
            for chunk in stream:
                piece = chunk["message"]["content"]
                if piece and first is None:
                    first = time.perf_counter() - t0
                parts.append(piece)
        except Exception:
            if use_think_flag and not parts:
                continue                # older server or library without 'think': retry once
            raise
        total = time.perf_counter() - t0
        return "".join(parts).strip(), round(first or total, 2), round(total, 2)
    raise RuntimeError("unreachable")


def main() -> None:
    items = load_inputs(INPUTS_FILE)
    print(f"{len(items)} inputs  |  model {MODEL}  |  host {HOST}")
    check_model()

    t0 = time.perf_counter()
    ask("hello")                        # first call loads the model into memory
    print(f"Model load + warmup: {time.perf_counter() - t0:.1f}s\n")

    rows = []
    for i, (category, text) in enumerate(items, 1):
        answer, first, total = ask(text)
        print(f"--- {i}/{len(items)}  [{category}]  first word {first}s, total {total}s")
        print(f">> {text}")
        print(answer, "\n")
        rows.append({"category": category, "input": text, "output": answer,
                     "first_word_s": first, "total_s": total, "score": ""})

    # utf-8-sig so Excel shows Urdu correctly
    with OUT_FILE.open("w", newline="", encoding="utf-8-sig") as f:
        writer = csv.DictWriter(f, fieldnames=list(rows[0].keys()))
        writer.writeheader()
        writer.writerows(rows)

    print("Speed by category (average)")
    for cat in dict.fromkeys(r["category"] for r in rows):
        sel = [r for r in rows if r["category"] == cat]
        fw = sum(r["first_word_s"] for r in sel) / len(sel)
        tt = sum(r["total_s"] for r in sel) / len(sel)
        print(f"  {cat:<18} first word {fw:.2f}s   total {tt:.2f}s")
    print(f"\nSaved {OUT_FILE.name}. Score each row 0 / 1 / 2.  Max score = {2 * len(rows)}.")


if __name__ == "__main__":
    main()
