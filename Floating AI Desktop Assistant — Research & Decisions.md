# Floating AI Desktop Assistant — Research & Decisions

Oct 11, 2026 · @Rajnesh Kumar Charan

## Summary

The idea is feasible and nobody ships it yet: no existing app pairs a desktop character with select-to-explain on a local model. The hard part, reading a selection without copying, works through Windows UI Automation in browsers and many standard apps, but not everywhere.

**Key decisions**

1. **Text capture:** UI Automation's Text pattern only (`GetSelection`). No clipboard, no screenshots by default. A clipboard fallback exists as an opt-in, per-app setting.
2. **Trigger:** select and release the mouse (drag or double-click). No hotkey needed, which is the main UX difference from POT and DeepL.
3. **Model:** Qwen 3.5 4B through Ollama as the default on a 6 GB GPU, tested head to head against Gemma 4 E2B on your own Urdu sentences. Smaller tiers for weaker PCs.
4. **Stack:** Tauri v2 with a small Rust core and web UI. Python stays for the Phase 0 experiment.
5. **Privacy:** mouse hook only, password fields and blocked apps skipped, model on `localhost`, history off by default.
6. **Order of work:** prove UIA coverage and Urdu quality in Phase 0 before writing product code.

**Correction to earlier chat advice:** POT is no longer an active project (archived September 2026), and Gemma 4 E4B is too large to fit fully in 6 GB of VRAM. Both are reflected below.

## Requirements as understood

Version 1 does one job well: you select text anywhere, and a friendly on-screen character explains and translates it using a model running on your own machine.

**Must have in v1**

- A small animated character that stays on screen without blocking your work (always on top, click-through when idle, movable).
- Selected text is read automatically. You never press Ctrl+C and never take a screenshot.
- Meaning and translation into simple English and Urdu, shown in a small popup near the selection.
- Local AI through Ollama by default. No paid API, nothing leaves the machine.
- Works on your laptop (Windows 11, 16 GB RAM, RTX 4050 6 GB) and degrades gracefully on weaker PCs.
- Code structured so other people can install and use it later: installer, settings, error handling, privacy controls.

**Not in v1**

- Reading text under the cursor without selecting it.
- OCR of images, videos or games.
- Cloud models, accounts, sync, voice, or agent actions.

**Hard constraint to keep in mind:** the no-copy, no-screenshot rule means some apps will simply not work in v1. This document says which, and proposes an opt-in fallback the user switches on knowingly rather than a silent workaround.

## Existing applications compared

No existing app combines a desktop character with select-to-explain translation on a local model. Every serious selection tool still falls back to the clipboard when Windows accessibility fails, and none reads selections purely through accessibility.

| App | How it gets the text | AI and local models | Floating UI | License and status | What to learn |
| --- | --- | --- | --- | --- | --- |
| [POT](https://github.com/pot-app/pot-desktop) | Select, then press a hotkey. Its [Selection library](https://github.com/pot-app/Selection) asks UI Automation for the focused element's selection first, then simulates Ctrl+C and restores the old clipboard | Ollama, OpenAI, Gemini, many web translators. Offline OCR via Windows.Media.OCR | Popup near cursor. A floating button needs the third-party SnipDo app | GPL-3.0. Archived (read-only) since 7 Sep 2026. Built on Tauri 1.6.8 | The UIA-first, clipboard-second order. Its plugin system for translators and dictionaries |
| [DeepL for Windows](https://www.deepl.com/windows-app) | Select, then press [Ctrl+C twice](https://support.deepl.com/hc/en-us/articles/360020613059-Use-DeepL-shortcuts-for-desktop-apps) or click a floating DeepL icon. Clipboard-based | DeepL cloud only. No local model | Floating icon plus small result window | Proprietary, free tier plus Pro | Floating icon as an alternative trigger. Per-app exclusion list for privacy |
| [PowerToys Advanced Paste](https://learn.microsoft.com/en-us/windows/powertoys/advanced-paste) | Clipboard. An opt-in setting tries to copy the current selection before running | Opt-in AI. Local providers: Ollama, Foundry Local, Phi Silica. Online: OpenAI, Azure, Google, Mistral | Window opened by Win+Shift+V | Open source (Microsoft PowerToys, MIT) | AI off by default. A clean provider list where local and cloud models are interchangeable |
| [Screen Translator](https://github.com/OneMoreGres/ScreenTranslator) | Hotkey, drag a region, Tesseract OCR on the screenshot | Online translators only | Result overlay on the captured region | MIT. Author says the project is almost abandoned | OCR is the only way to reach images and games, but it is slow and lossy |
| [Open-LLM-VTuber](https://github.com/Open-LLM-VTuber/Open-LLM-VTuber) | Not a selection tool. Voice and chat companion | Ollama and many cloud providers | Pet mode: transparent, always on top, click-through, draggable | Open source | How to build a friendly on-screen character that does not get in the way |
| [Clippy (local LLM edition)](https://www.guru3d.com/story/clippy-ai-assistant-transform-mascot-into-a-local-llm-interface) | Not a selection tool. You type into it | Downloads Gemma 3 1B on first run, runs offline | Character waits in a screen corner | MIT, Electron | Proof that a mascot plus a tiny local model is a pleasant experience |

**Gap your product fills:** a friendly character, select-and-release with no hotkey, Urdu and simple-English explanations, and local AI by default.

**Reuse rule:** POT and its Selection library are GPL-3.0. Copying their code would force your whole app to be GPL-3.0. Read them to learn the approach, then write your own implementation against the public Windows APIs. PowerToys (MIT) and Clippy (MIT) code can be reused with attribution.

## Core challenge: reading selected text without copying

Yes, it can be done without the clipboard or a screenshot, through Windows UI Automation (UIA). But it only works in apps that expose their text to UIA. No method covers every app.

**Plain-language version.** Windows asks every app to keep an up-to-date report of its text for screen readers used by blind people. Your assistant reads that same report and asks: "what is selected right now?" It never touches the clipboard or the screen pixels. Apps that do not keep this report are invisible to this method.

**How it works, step by step (verified against Microsoft docs and POT's source):**

1. Detect that a selection just finished: a low-level mouse hook sees a left-button release after a drag, or a double-click.
2. Ask UIA for the focused element, then for its **Text pattern**.
3. Call `GetSelection()`. It returns one range per selected span, or an empty range at the caret if nothing is selected ([Microsoft: Text and TextRange patterns](https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-implementingtextandtextrange)).
4. Call `GetText(-1)` on each range and join them.

Microsoft's guidance says Edit and Document controls, and any control that lets the user select read-only text, should support the Text pattern. It also requires providers to raise a `TextSelectionChanged` event whenever the selection or caret moves. That event is a cleaner trigger than a mouse hook, but it also fires on every caret move, so it needs filtering.

POT's open-source code uses exactly steps 2 to 4 first ([source](https://raw.githubusercontent.com/pot-app/Selection/master/src/windows.rs)), and its own comment notes the UIA path works for Edge, Chrome and UWP apps. Only when that returns nothing does it simulate Ctrl+C.

### Four situations that must not be confused

| Situation | Possible without copy or screenshot? | Method | When to build |
| --- | --- | --- | --- |
| Text you selected, in an app that exposes UIA text | Yes | `TextPattern.GetSelection()` | v1 |
| Word under the cursor, nothing selected | Yes, in the same apps | `RangeFromPoint()` returns an empty range nearest the point; expand it to a word with `ExpandToEnclosingUnit(Word)` | v2, behind a modifier key |
| Text in an app with no UIA text support | No | Only simulated Ctrl+C (clipboard) | Opt-in fallback, off by default |
| Text inside images, video, games | No, there is no text, only pixels | Screenshot of a small region + OCR | Later, opt-in, explicit user action only |

**Decision:** v1 uses UIA only. The clipboard fallback exists as a setting the user turns on knowingly, with a per-app allow list. OCR is out of v1. This keeps your no-copy, no-screenshot rule intact by default instead of breaking it silently, which is what POT and DeepL do.

## Which apps expose selected text

Browsers are the strongest case: both Chromium and Firefox now speak UIA natively. Everything else must be tested on your machine before you promise it to users.

| App group | Expected result with UIA only | Evidence level | Notes |
| --- | --- | --- | --- |
| Chrome, Edge, Brave and other Chromium browsers | Works | Verified | Native UIA is on by default [from Chrome 138](https://developer.chrome.com/blog/windows-uia-support-update). POT's code relies on this path for Edge and Chrome |
| Firefox | Likely works | Verified support, untested selection | Firefox [143](https://www.firefox.com/en-US/firefox/143.0/releasenotes) added Windows UIA support |
| Edge's built-in PDF viewer | Likely works | Assumption | Runs on Chromium; test with real PDFs |
| Notepad, WinUI and UWP text boxes | Likely works | Microsoft spec + POT comment | Edit controls are expected to implement the Text pattern |
| Word, Outlook | Likely works | Microsoft spec, untested | Document controls are expected to implement the Text pattern |
| Electron apps (VS Code, Discord, Slack, Obsidian) | Uncertain | Verified risk | Electron [keeps accessibility off by default](https://github.com/electron/electron/pull/10305/files/75b2915feef8e742111f05965d2542d661d7932d) for speed and turns it on when it detects assistive technology. Whether your query triggers that must be tested |
| Adobe Acrobat Reader | Unknown | Not verified | Could not confirm UIA Text pattern support; test |
| Windows Terminal, consoles | Unknown | Not verified | Never use the clipboard fallback here: a simulated Ctrl+C stops the running command |
| Java, Qt or custom-drawn apps, games | Unlikely | General pattern | Often expose no Text pattern |
| Text in images, video subtitles | No | Certain | Only OCR can reach it |

**How to check any app in two minutes:** run Microsoft's free **Inspect** tool from the Windows SDK, click into the app, and look for `IsTextPatternAvailable: true` on the focused element. Microsoft recommends Inspect and Accessible Event Watcher for exactly this check.

**User-facing behaviour when an app is not supported:** the character shows a short, friendly message ("I can't read this app yet") and offers the opt-in fallback. It never fails silently.

## Local AI models via Ollama

Start with **Qwen 3.5 4B** as the default: it is the largest current small model that fits entirely in your 6 GB GPU. Compare it against Gemma 4 E2B on your own Urdu sentences before locking the choice.

| Model | Ollama tag | Download size | Fits fully in 6 GB VRAM? | Role |
| --- | --- | --- | --- | --- |
| [Qwen 3.5 4B](https://ollama.com/library/qwen3.5) | `qwen3.5:4b` | 3.3 to 4.0 GB | Yes | Default on your laptop |
| [Gemma 4 E2B](https://ollama.com/library/gemma4) | `gemma4:e2b` | 4.6 to 7.5 GB | Smallest variant only, tight | Head-to-head challenger |
| Gemma 4 E4B | `gemma4:e4b` | 6.6 to 9.5 GB | No, spills to CPU and slows down | Quality reference only |
| Qwen 3.5 9B | `qwen3.5:9b` | 6.6 to 7.6 GB | No | Users with 8 GB+ GPUs |
| Qwen 3.5 2B | `qwen3.5:2b` | 2.7 to 3.1 GB | Yes | Weaker PCs, 4 GB GPUs |
| Qwen 3.5 0.8B | `qwen3.5:0.8b` | 1.2 to 1.3 GB | Yes, also usable on CPU | No-GPU PCs, simple meanings only |

Sizes are Ollama download sizes. Real memory use is higher once the model runs, so confirm with `ollama ps`: it must say 100% GPU.

**Correction to my earlier chat advice:** I suggested Gemma 4 E4B as the default. Its smallest download is 6.6 GB, so it cannot sit fully in your 6 GB GPU. Qwen 3.5 4B is the safer default.

**What is verified**

- Licenses: Gemma 4 moved to [Apache 2.0](https://the-decoder.com/googles-gemma-4-is-now-available-with-apache-2-0-licensing-for-the-first-time/), the same license Qwen 3.5 uses. Both allow commercial distribution.
- Qwen 3.5 claims support for 201 languages and dialects.
- Gemma 4 E2B and E4B leave thinking off unless the system prompt asks for it, which keeps answers fast.

**What is not verified**

- No published benchmark measures these small models on Urdu translation. Multilingual scores exist (Gemma 4 E4B scores 76.6% on MMMLU, E2B 67.4%), but they test knowledge, not translation quality.
- So the deciding test is yours: 30 real sentences you read often (English words, English sentences, Urdu, Roman Urdu), scored side by side for correctness and speed.

**Fit for weaker computers.** The app checks free VRAM at first run and suggests a tier: 0.8B for no GPU, 2B for 4 GB, 4B for 6 GB, 9B for 8 GB and above. A small built-in dictionary (for example an offline English dictionary file) answers single words instantly on any machine and saves model calls.

**Prompt design.** Two short templates: a single word gets meaning, part of speech, one example and the Urdu meaning; a sentence gets a translation plus a one-line simple-English explanation. Low temperature, thinking off, model kept loaded with Ollama's `keep_alive`, answers streamed into the popup.

## Recommended architecture and tech stack

Build the product on **Tauri v2**: a small Rust core talks to Windows, and web pages draw the character and popup. Validate the capture idea first in your existing Python prototype, then port it.

&#91;embedded content: v1 architecture · 7 parts, all on the user's PC\]

Text flows left to right through the watcher and privacy gate to the core, which asks the local model and streams the answer into the popup. The dashed clipboard path exists only if the user turns it on.

| Layer | Choice | Why | Alternative considered |
| --- | --- | --- | --- |
| App shell | Tauri v2 | Transparent, always-on-top, no-taskbar windows and click-through (`set_ignore_cursor_events`) are built in ([docs](https://docs.rs/tauri/latest/tauri/window/struct.WindowBuilder.html)). Uses the WebView2 already in Windows 11, so the installer stays small. POT proved this stack for the same job | Electron: far larger install. Python + PySide6: fastest for you to write, but a heavier installer and more antivirus false alarms (common experience, not measured here) |
| Windows access | Rust with Microsoft's `windows` crate: UIA Text pattern + low-level mouse hook | Same official APIs POT calls. No extra runtime | Python `uiautomation` for the spike only |
| UI | TypeScript web pages in three Tauri windows: mascot, popup, settings | Easy to make polished; reusable for agent features later | Native WinUI: more work, little gain for v1 |
| Character | Rive or Lottie animation with states: idle, noticed, thinking, talking, can't read | Smooth motion that reacts to what is happening | A static image feels cheap |
| AI | Ollama HTTP API on `localhost:11434`, called from Rust, behind a `Provider` interface | Swap models, or add an opt-in cloud provider later, without touching the UI. Calling from Rust avoids browser CORS issues | Embedding llama.cpp directly: more control, much more work |
| Storage | Settings in the Tauri store plugin. History optional, in local SQLite | Small, local, no server | None needed |

**Module boundaries** (each testable on its own): `SelectionSource` returns text, source app and on-screen rectangle. `PrivacyGate` says allow or skip. `Explainer` handles cache, language detection and prompt. `Provider` streams tokens. The UI only listens to events. The selection rectangle comes from UIA's `GetBoundingRectangles`, so the popup can appear right next to the selected words.

**Deliberately left out of v1:** no backend server, no Docker, no bundled Python, no accounts, no telemetry.

**Honest trade-off:** you will write some Rust. It is three small modules (mouse hook, UIA reader, Ollama client). If that feels too slow, ship v1 in Python + PySide6 for yourself and move to Tauri before giving it to others.

## Privacy, security and distribution

The privacy promise is simple and checkable: selected text goes from the app to a model on `localhost`, and nothing else is read, kept or sent unless the user turns it on.

**Privacy rules built into v1**

- Read only the selection, only after a mouse release. Never read the whole screen, never take screenshots.
- Hook the mouse only. No keyboard hook, so the app can never see keystrokes. Global shortcuts use the normal Windows hotkey registration.
- Skip password fields (UIA exposes an `IsPassword` property) and any app on a user-editable block list. Ship the list pre-filled with password managers and banking apps.
- Ollama listens on `localhost` by default. The app refuses to send text to a non-local address unless the user explicitly adds a cloud provider.
- History is off by default. When on, it stays in a local SQLite file with a one-click "clear all".
- The popup window uses Tauri's content protection so screen recorders and other apps cannot capture translations.
- No telemetry in v1. If crash reporting is added later, it is opt-in and never includes selected text.

**Security basics**

- Treat model output as untrusted text: render it as plain text in the popup, never as HTML.
- Tauri capability permissions: each window gets only the commands it needs.
- Signed auto-updates through Tauri's updater plugin.

**Packaging for other users**

| Item | Plan | Why it matters |
| --- | --- | --- |
| Installer | Tauri's NSIS or MSI bundle | One normal Windows installer |
| Code signing | Buy a code-signing certificate or use a cloud signing service before public release | Unsigned apps trigger Windows SmartScreen warnings that scare users away |
| Ollama | First-run check: if Ollama is missing, show a guided install link; then pull the tier model with a progress bar | The model is a 1 to 4 GB download, so set expectations clearly |
| Updates | Tauri updater with a signed update feed | Fixes reach users without reinstalling |
| Licenses | Your code under a license you choose. Include Apache 2.0 notices for the model and MIT notices for any reused MIT code | Keeps commercial options open |

**Open question for distribution:** requiring users to install Ollama separately adds friction. Bundling a local runtime is possible later but is a bigger engineering job; v1 accepts the friction.

## Risks and what to test first

The two risks that can kill the idea are UIA coverage in your daily apps and Urdu quality from a 4B model. Both are testable in a weekend, before any product code.

| Risk | Impact if it happens | Mitigation | Test |
| --- | --- | --- | --- |
| UIA returns nothing in apps you use daily | Core promise fails in those apps | Opt-in clipboard fallback per app; clear "can't read here" message | Run the Python prototype for a week across your real apps and log hit rate per app |
| Electron apps keep accessibility off | VS Code, Discord, Slack may not work | Test whether a UIA query wakes them up; document unsupported apps | Same week-long log |
| Popup steals focus and clears the selection | Every translation breaks the user's flow | Create the popup as a non-activating window (no focus on show) | Select text in Chrome, show popup, check the selection survives |
| Small model gives weak Urdu | Answers look wrong; trust lost | Head-to-head test; dictionary for single words; allow model switching | 30-sentence scored comparison |
| GPU busy with other apps | Slow or stalled answers | Smaller tier on demand; show "thinking" state; unload model after idle | Run with a game or video editor open |
| Accidental triggers | Popup appears on every click | Trigger only on drag or double-click, minimum 2 characters, non-empty `GetSelection` | Normal browsing for a day |
| Antivirus or SmartScreen warnings | Users refuse to install | Code signing; no keyboard hook; no injected code | Scan the installer on VirusTotal before release |
| Copying GPL code by accident | Whole app becomes GPL | Write your own selection code from Microsoft docs | Code review before release |

**Assumptions in this document that are still untested:** Firefox selection behaviour, Word and Outlook, Edge PDF viewer, Adobe Reader, Windows Terminal, Electron wake-up behaviour, and any Urdu quality claim. Treat them as hypotheses until your log says otherwise.

## Incremental build plan

Do not write product code until Phase 0 answers the two risky questions. Each later phase ends at a gate you can check yourself.

&#91;embedded content: build plan · 5 phases, 4 gates\]

Each gate is a yes/no check; if a gate fails, fix that phase instead of moving on.

**Phase 0 checklist (this week)**

- [x] Add per-app logging to `selection_reader.py` (app name, UIA hit or miss) and use it for a week
- [x] For every app that misses, check it with Microsoft's Inspect tool
- [ ] Write 30 test inputs you really meet: English words, English sentences, Urdu, Roman Urdu
- [x] Run `compare_models.py` with `qwen3.5:4b` and `gemma4:e2b`; score accuracy and speed
- [x] Decide: Tauri from Phase 1, or Python first for personal use
- [x] License or commission the character art (the current image carries a stock-site watermark)

## Sources

- [POT desktop repository](https://github.com/pot-app/pot-desktop): features, GPL-3.0 license, archived status, Tauri version
- [POT Selection library](https://github.com/pot-app/Selection) and its [Windows source](https://raw.githubusercontent.com/pot-app/Selection/master/src/windows.rs): UIA-first, clipboard-second method
- [DeepL shortcuts for desktop apps](https://support.deepl.com/hc/en-us/articles/360020613059-Use-DeepL-shortcuts-for-desktop-apps) and [DeepL for Windows](https://www.deepl.com/windows-app)
- [PowerToys Advanced Paste](https://learn.microsoft.com/en-us/windows/powertoys/advanced-paste): clipboard input, local AI providers
- [Screen Translator](https://github.com/OneMoreGres/ScreenTranslator): OCR approach, maintenance status
- [Open-LLM-VTuber](https://github.com/Open-LLM-VTuber/Open-LLM-VTuber): desktop pet mode
- [Clippy local LLM assistant](https://www.guru3d.com/story/clippy-ai-assistant-transform-mascot-into-a-local-llm-interface)
- [Microsoft: Text and TextRange control patterns](https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-implementingtextandtextrange)
- [Chrome: native UI Automation on Windows](https://developer.chrome.com/blog/windows-uia-support-update)
- [Firefox 143 release notes](https://www.firefox.com/en-US/firefox/143.0/releasenotes)
- [Electron accessibility defaults](https://github.com/electron/electron/pull/10305/files/75b2915feef8e742111f05965d2542d661d7932d)
- [Ollama: Gemma 4](https://ollama.com/library/gemma4) and [Ollama: Qwen 3.5](https://ollama.com/library/qwen3.5): sizes, context, languages
- [Gemma 4 Apache 2.0 license](https://the-decoder.com/googles-gemma-4-is-now-available-with-apache-2-0-licensing-for-the-first-time/)
- [Tauri WindowBuilder](https://docs.rs/tauri/latest/tauri/window/struct.WindowBuilder.html): transparent, always-on-top, skip-taskbar windows
