import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import AppList from "./AppList";

/** Mirrors `Settings` in src-tauri/src/settings.rs (tested there against these names). */
interface SettingsData {
  model: string;
  blockList: string[];
  paused: boolean;
  historyEnabled: boolean;
  clipboardFallback: boolean;
  clipboardFirstApps: string[];
  mascotPosition: { x: number; y: number } | null;
}

interface ClipboardStatus {
  cloudSync: boolean;
  history: boolean;
}

type Check = { kind: "idle" } | { kind: "checking" } | { kind: "ok" } | { kind: "error"; message: string };

const errorText = (e: unknown) => (typeof e === "string" ? e : "Something went wrong.");

// All text is rendered as plain React text.
export default function Settings() {
  const [settings, setSettings] = useState<SettingsData | null>(null);
  const [modelDraft, setModelDraft] = useState("");
  const [models, setModels] = useState<string[]>([]);
  const [check, setCheck] = useState<Check>({ kind: "idle" });
  const [clipboard, setClipboard] = useState<ClipboardStatus>({ cloudSync: false, history: false });
  const [error, setError] = useState("");
  const [version, setVersion] = useState("");

  useEffect(() => {
    invoke<SettingsData>("get_settings")
      .then((s) => {
        setSettings(s);
        setModelDraft(s.model);
      })
      .catch((e) => setError(errorText(e)));
    invoke<string[]>("list_models").then(setModels).catch(() => setModels([]));
    getVersion().then(setVersion).catch(() => setVersion(""));

    // Windows clipboard settings can change while this window is open.
    const refreshClipboard = () =>
      invoke<ClipboardStatus>("clipboard_status").then(setClipboard).catch(() => {});
    refreshClipboard();
    window.addEventListener("focus", refreshClipboard);

    // Changes from elsewhere (tray Pause, mascot moved).
    const unlisten = listen<SettingsData>("settings-changed", ({ payload }) => setSettings(payload));
    return () => {
      window.removeEventListener("focus", refreshClipboard);
      unlisten.then((stop) => stop());
    };
  }, []);

  const save = (change: Partial<SettingsData>) => {
    if (!settings) return;
    invoke<SettingsData>("save_settings", { settings: { ...settings, ...change } })
      .then((saved) => {
        setSettings(saved);
        setError("");
      })
      .catch((e) => setError(errorText(e)));
  };

  if (!settings) {
    return (
      <main className="settings">
        <h1>Screen Buddy</h1>
        {error ? <p className="settings__error">{error}</p> : <p className="settings__muted">Loading…</p>}
      </main>
    );
  }

  const saveModel = () => {
    const model = modelDraft.trim();
    if (model && model !== settings.model) {
      setCheck({ kind: "idle" });
      save({ model });
    } else {
      setModelDraft(settings.model);
    }
  };

  const checkModel = () => {
    setCheck({ kind: "checking" });
    invoke("check_model", { model: modelDraft })
      .then(() => setCheck({ kind: "ok" }))
      .catch((e) => setCheck({ kind: "error", message: errorText(e) }));
  };

  const restoreDefaults = () => {
    invoke<string[]>("default_block_list")
      .then((blockList) => save({ blockList }))
      .catch((e) => setError(errorText(e)));
  };

  const resetMascot = () => {
    invoke("reset_mascot_position").catch((e) => setError(errorText(e)));
  };

  return (
    <main className="settings">
      <h1>Screen Buddy</h1>
      {error && <p className="settings__error">{error}</p>}

      <section className="settings__section">
        <h2>General</h2>
        <label className="settings__switch">
          <input type="checkbox" checked={settings.paused} onChange={(e) => save({ paused: e.target.checked })} />
          <span>Paused (selections are ignored)</span>
        </label>
        <div className="settings__row">
          <button type="button" onClick={resetMascot}>
            Reset mascot position
          </button>
        </div>
      </section>

      <section className="settings__section">
        <h2>Model</h2>
        <p className="settings__muted">The Ollama model that explains your selections.</p>
        <div className="settings__row">
          <input
            className="settings__input"
            list="installed-models"
            value={modelDraft}
            spellCheck={false}
            onChange={(e) => {
              setModelDraft(e.target.value);
              setCheck({ kind: "idle" });
            }}
            onBlur={saveModel}
            onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
          />
          <datalist id="installed-models">
            {models.map((m) => (
              <option key={m} value={m} />
            ))}
          </datalist>
          <button type="button" onClick={checkModel} disabled={check.kind === "checking"}>
            Check
          </button>
        </div>
        {check.kind === "checking" && <p className="settings__muted">Checking…</p>}
        {check.kind === "ok" && <p className="settings__ok">✓ Ready</p>}
        {check.kind === "error" && <p className="settings__error">{check.message}</p>}
      </section>

      <section className="settings__section">
        <h2>Privacy</h2>
        <p className="settings__muted">Text in these apps is never read.</p>
        <AppList
          apps={settings.blockList}
          placeholder="App to block, e.g. notepad.exe"
          onChange={(blockList) => save({ blockList })}
        />
        <div className="settings__row">
          <button type="button" className="settings__link" onClick={restoreDefaults}>
            Restore defaults
          </button>
        </div>
        <label className="settings__switch">
          <input
            type="checkbox"
            checked={settings.historyEnabled}
            onChange={(e) => save({ historyEnabled: e.target.checked })}
          />
          <span>Keep a history of explanations</span>
        </label>
        <p className="settings__muted settings__note">Saving history comes in a later version.</p>
      </section>

      <section className="settings__section">
        <h2>Clipboard fallback</h2>
        <p className="settings__muted">
          Screen Buddy first asks the app for your selection. When the app can't tell it (or tells the wrong words,
          like PDFs in Edge), Screen Buddy copies your selection with Ctrl+C and then puts your clipboard back as it
          was. Terminals get their own safe copy keys (Ctrl+Insert), so a running program is never stopped.
          Never in password fields or blocked apps.
        </p>
        <label className="settings__switch">
          <input
            type="checkbox"
            checked={settings.clipboardFallback && !clipboard.cloudSync}
            disabled={clipboard.cloudSync}
            onChange={(e) => save({ clipboardFallback: e.target.checked })}
          />
          <span>Use the clipboard when needed</span>
        </label>
        {clipboard.cloudSync && (
          <p className="settings__warning">
            Your clipboard syncs across devices, so copied text would leave this PC. Turn off clipboard sync in Windows
            Settings → System → Clipboard to use this.
          </p>
        )}
        {!clipboard.cloudSync && clipboard.history && settings.clipboardFallback && (
          <p className="settings__warning">Copied text will also appear in your clipboard history (Win+V).</p>
        )}

        <h3>Always use the clipboard in these apps</h3>
        <p className="settings__muted">
          For apps that show the wrong words. Their own Copy is used first. You can also add an app from the popup:
          "Wrong text? Use the clipboard in …". PDFs in Edge and Chrome are handled automatically.
        </p>
        <AppList
          apps={settings.clipboardFirstApps}
          placeholder="App, e.g. olk.exe"
          disabled={!settings.clipboardFallback || clipboard.cloudSync}
          onChange={(clipboardFirstApps) => save({ clipboardFirstApps })}
        />
      </section>

      {version && <p className="settings__muted settings__version">Version {version}</p>}
    </main>
  );
}
