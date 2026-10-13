import { useState } from "react";

interface AppListProps {
  /** Exe names, as saved (Rust normalizes them: lowercase, `.exe`, sorted). */
  apps: string[];
  placeholder: string;
  disabled?: boolean;
  onChange: (apps: string[]) => void;
}

/** A list of apps as removable chips, with a field to add one. Used for the block list and the
 *  clipboard-first list. */
export default function AppList({ apps, placeholder, disabled = false, onChange }: AppListProps) {
  const [draft, setDraft] = useState("");

  const add = () => {
    const app = draft.trim();
    if (!app) return;
    onChange([...apps, app]);
    setDraft("");
  };

  return (
    <div className={disabled ? "settings__applist settings__applist--disabled" : "settings__applist"}>
      {apps.length > 0 && (
        <ul className="settings__chips">
          {apps.map((app) => (
            <li key={app} className="settings__chip">
              <span>{app}</span>
              <button
                type="button"
                aria-label={`Remove ${app}`}
                disabled={disabled}
                onClick={() => onChange(apps.filter((a) => a !== app))}
              >
                ×
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="settings__row">
        <input
          className="settings__input"
          placeholder={placeholder}
          value={draft}
          spellCheck={false}
          disabled={disabled}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && add()}
        />
        <button type="button" onClick={add} disabled={disabled || !draft.trim()}>
          Add
        </button>
      </div>
    </div>
  );
}
