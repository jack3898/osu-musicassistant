import { useCallback, useEffect, useState } from "react";
import { api, type Settings, type Status } from "./api";
import { ConnectCard } from "./components/ConnectCard";
import { PlayCard } from "./components/PlayCard";
import { SettingsCard } from "./components/SettingsCard";
import icon from "./icon.svg";

export function App() {
  const [status, setStatus] = useState<Status | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);

  useEffect(() => {
    api.status().then(setStatus);
    api.settings().then(setSettings);
    const unlisten = api.onStatus(setStatus);
    return () => {
      unlisten.then((stop) => stop());
    };
  }, []);

  const refreshSettings = useCallback(() => api.settings().then(setSettings), []);

  if (!status || !settings) {
    return <main className="loading">Loading…</main>;
  }

  const maConfigured = settings.maUrl !== "" && settings.hasMaToken;

  return (
    <main>
      <header className="header">
        <img src={icon} alt="" className="header-icon" />
        <div>
          <h1>osu! Music Assistant</h1>
          <StatusLine status={status} />
        </div>
      </header>

      <PlayCard status={status} settings={settings} maConfigured={maConfigured} />
      <ConnectCard status={status} onSettingsChange={setSettings} defaultOpen={!maConfigured} />
      <SettingsCard settings={settings} status={status} onSaved={setSettings} onTokenChange={refreshSettings} />
    </main>
  );
}

function StatusLine({ status }: { status: Status }) {
  if (status.libraryError) {
    return <p className="status error">{status.libraryError}</p>;
  }
  if (status.serverError) {
    return <p className="status error">{status.serverError}</p>;
  }
  if (!status.loadedAt) {
    return <p className="status">Reading your osu! library…</p>;
  }
  return (
    <p className="status">
      <span className={status.serverRunning ? "dot ok" : "dot"} />
      {status.songs.toLocaleString()} songs · {status.collections} collections
      {status.serverRunning ? ` · serving on port ${status.port}` : " · server stopped"}
    </p>
  );
}
