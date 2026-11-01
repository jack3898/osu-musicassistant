import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import { api, errorMessage, type MaInfo, type Settings, type SettingsUpdate, type Status } from "../api";

interface Props {
  settings: Settings;
  status: Status;
  onSaved: (settings: Settings) => void;
  onTokenChange: () => void;
}

type Feedback = { kind: "ok" | "error"; text: string } | null;

function formFrom(settings: Settings): SettingsUpdate {
  return {
    osuDataDir: settings.osuDataDir,
    port: settings.port,
    hideRateEdits: settings.hideRateEdits,
    blockedPatterns: settings.blockedPatterns,
    preferUnicode: settings.preferUnicode,
    maUrl: settings.maUrl,
    keepInTray: settings.keepInTray,
    launchAtLogin: settings.launchAtLogin,
  };
}

export function SettingsCard({ settings, status, onSaved, onTokenChange }: Props) {
  const [form, setForm] = useState<SettingsUpdate>(() => formFrom(settings));
  const [patterns, setPatterns] = useState(settings.blockedPatterns.join("\n"));
  const [token, setToken] = useState("");
  const [saving, setSaving] = useState(false);
  const [feedback, setFeedback] = useState<Feedback>(null);
  const [maTest, setMaTest] = useState<Feedback>(null);
  const [reindexing, setReindexing] = useState(false);

  useEffect(() => {
    setForm(formFrom(settings));
    setPatterns(settings.blockedPatterns.join("\n"));
  }, [settings]);

  const update = <K extends keyof SettingsUpdate>(key: K, value: SettingsUpdate[K]) =>
    setForm((f) => ({ ...f, [key]: value }));

  const blockedPatterns = patterns
    .split("\n")
    .map((p) => p.trim())
    .filter(Boolean);
  const dirty =
    JSON.stringify({ ...form, blockedPatterns }) !== JSON.stringify(formFrom(settings)) || token.trim() !== "";

  const save = async () => {
    setSaving(true);
    setFeedback(null);
    try {
      if (token.trim()) {
        await api.setMaToken(token.trim());
        setToken("");
        onTokenChange();
      }
      onSaved(await api.saveSettings({ ...form, blockedPatterns }));
      setFeedback({ kind: "ok", text: "Saved." });
    } catch (e) {
      setFeedback({ kind: "error", text: errorMessage(e) });
    } finally {
      setSaving(false);
    }
  };

  const testConnection = async () => {
    setMaTest(null);
    try {
      const info: MaInfo = await api.testMusicAssistant();
      const name = info.serverName ?? "Music Assistant";
      setMaTest(
        info.provider
          ? { kind: "ok", text: `Connected to ${name} (${info.serverVersion}). Your osu! library is set up there.` }
          : {
              kind: "error",
              text: `Connected to ${name} (${info.serverVersion}), but it doesn't have this library yet — follow "Connect Music Assistant" above.`,
            },
      );
    } catch (e) {
      setMaTest({ kind: "error", text: errorMessage(e) });
    }
  };

  const removeToken = async () => {
    await api.setMaToken("");
    onTokenChange();
    setMaTest(null);
  };

  const browse = async () => {
    const dir = await open({ directory: true, title: "Choose your osu!lazer data folder" });
    if (typeof dir === "string") update("osuDataDir", dir);
  };

  const reindex = async () => {
    setReindexing(true);
    try {
      await api.reindex();
    } finally {
      setReindexing(false);
    }
  };

  return (
    <details className="card">
      <summary>
        <h2>Settings</h2>
      </summary>

      <h3>Music Assistant</h3>
      <label className="field">
        <span>Address</span>
        <input
          type="url"
          placeholder="https://music.example.com or http://192.168.1.5:8095"
          value={form.maUrl}
          onChange={(e) => update("maUrl", e.target.value)}
        />
      </label>
      <label className="field">
        <span>Token</span>
        <input
          type="password"
          autoComplete="off"
          placeholder={settings.hasMaToken ? "Saved in your keychain — paste to replace" : "Paste a long-lived token"}
          value={token}
          onChange={(e) => setToken(e.target.value)}
        />
      </label>
      <p className="hint small">
        Create a long-lived token in Music Assistant from your user profile. It's stored in your system keychain.
      </p>
      <div className="row">
        <button onClick={testConnection} disabled={!settings.maUrl || !settings.hasMaToken || dirty}>
          Test connection
        </button>
        {settings.hasMaToken && (
          <button className="link" onClick={removeToken}>
            Remove token
          </button>
        )}
      </div>
      {maTest && <p className={`feedback ${maTest.kind}`}>{maTest.text}</p>}

      <h3>Library</h3>
      <label className="field">
        <span>osu! folder</span>
        <div className="row">
          <input
            type="text"
            placeholder={status.dataDir ?? "Detected automatically"}
            value={form.osuDataDir ?? ""}
            onChange={(e) => update("osuDataDir", e.target.value || null)}
          />
          <button onClick={browse}>Browse…</button>
        </div>
      </label>
      <label className="check">
        <input type="checkbox" checked={form.hideRateEdits} onChange={(e) => update("hideRateEdits", e.target.checked)} />
        Hide practice rate edits (e.g. <code>audio 1.200x.mp3</code>)
      </label>
      <label className="field">
        <span>Hide songs matching</span>
        <textarea
          rows={3}
          placeholder={"One pattern per line, e.g.\nnightcore\n(sped up)"}
          value={patterns}
          onChange={(e) => setPatterns(e.target.value)}
        />
      </label>
      <p className="hint small">Matches song titles, artists and audio filenames, ignoring case.</p>
      <div className="field">
        <span>Names</span>
        <div className="segmented">
          <button className={!form.preferUnicode ? "active" : ""} onClick={() => update("preferUnicode", false)}>
            Romanised
          </button>
          <button className={form.preferUnicode ? "active" : ""} onClick={() => update("preferUnicode", true)}>
            Original script
          </button>
        </div>
      </div>
      <div className="row">
        <button onClick={reindex} disabled={reindexing}>
          {reindexing ? "Re-reading…" : "Re-read library now"}
        </button>
        {status.loadedAt && (
          <span className="hint small">Last read {new Date(status.loadedAt * 1000).toLocaleTimeString()}</span>
        )}
      </div>

      <h3>App</h3>
      <label className="check">
        <input type="checkbox" checked={form.keepInTray} onChange={(e) => update("keepInTray", e.target.checked)} />
        Keep running in the menu bar when the window is closed
      </label>
      <label className="check">
        <input
          type="checkbox"
          checked={form.launchAtLogin}
          onChange={(e) => update("launchAtLogin", e.target.checked)}
        />
        Open at login
      </label>
      <label className="field narrow">
        <span>Server port</span>
        <input
          type="number"
          min={1024}
          max={65535}
          value={form.port}
          onChange={(e) => update("port", Number(e.target.value))}
        />
      </label>

      <div className="save-bar">
        <button className="primary" onClick={save} disabled={!dirty || saving}>
          {saving ? "Saving…" : "Save settings"}
        </button>
        {feedback && <span className={`feedback inline ${feedback.kind}`}>{feedback.text}</span>}
      </div>
    </details>
  );
}
