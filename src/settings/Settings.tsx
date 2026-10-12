import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";

export default function Settings() {
  const [version, setVersion] = useState("");

  useEffect(() => {
    getVersion().then(setVersion).catch(() => setVersion(""));
  }, []);

  return (
    <main className="settings">
      <h1>Screen Buddy</h1>
      <p>Settings arrive in Task 7: model name, block list, pause and history.</p>
      {version && <p className="settings__version">Version {version}</p>}
    </main>
  );
}
