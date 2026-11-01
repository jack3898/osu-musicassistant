import { ask } from "@tauri-apps/plugin-dialog";
import { type ReactNode, useState } from "react";
import { api, errorMessage, type Settings, type Status } from "../api";

interface Props {
  status: Status;
  onSettingsChange: (settings: Settings) => void;
  defaultOpen: boolean;
}

export function ConnectCard({ status, onSettingsChange, defaultOpen }: Props) {
  const [error, setError] = useState<string | null>(null);

  const regenerate = async () => {
    const confirmed = await ask("Music Assistant will need the new password before it can play again.", {
      title: "Make a new password?",
      kind: "warning",
    });
    if (!confirmed) return;
    try {
      onSettingsChange(await api.regeneratePassword());
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const [primary, ...others] = status.addresses;

  return (
    <details className="card" open={defaultOpen}>
      <summary>
        <h2>Connect Music Assistant</h2>
      </summary>
      <p className="hint">
        In Music Assistant open <b>Settings → Music providers → Add a music provider</b>, choose{" "}
        <b>OpenSubsonic Media Server Library</b> and enter:
      </p>

      <CopyField label="Base URL" value={primary ?? "http://<this computer's IP>"} />
      {others.length > 0 && (
        <p className="hint small">
          If Music Assistant can't connect, try {others.join(" or ")} instead (e.g. when it reaches this computer over
          a VPN).
        </p>
      )}
      <CopyField label="Port" value={String(status.port)} />
      <CopyField label="Server Path" value="" placeholder="leave empty" />
      <CopyField label="Username" value={status.username} />
      <CopyField label="Password" value={status.password}>
        <button className="link" onClick={regenerate}>
          New password
        </button>
      </CopyField>
      {error && <p className="feedback error">{error}</p>}
    </details>
  );
}

function CopyField({
  label,
  value,
  placeholder,
  children,
}: {
  label: string;
  value: string;
  placeholder?: string;
  children?: ReactNode;
}) {
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    await navigator.clipboard.writeText(value);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };
  return (
    <div className="copy-field">
      <span className="copy-label">{label}</span>
      <code className={value ? "" : "placeholder"}>{value || placeholder}</code>
      {value && (
        <button className="icon-button" onClick={copy} title={`Copy ${label}`} aria-label={`Copy ${label}`}>
          {copied ? "✓" : "⧉"}
        </button>
      )}
      {children}
    </div>
  );
}
