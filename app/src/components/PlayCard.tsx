import { useCallback, useEffect, useState } from "react";
import { api, errorMessage, type Player, type Playlist, type Settings, type Status } from "../api";

interface Props {
  status: Status;
  settings: Settings;
  maConfigured: boolean;
}

type Feedback = { kind: "ok" | "error"; text: string } | null;

export function PlayCard({ status, settings, maConfigured }: Props) {
  const [playlists, setPlaylists] = useState<Playlist[]>([]);
  const [players, setPlayers] = useState<Player[]>([]);
  const [playlistId, setPlaylistId] = useState(settings.lastPlaylistId ?? "pl-all");
  const [playerId, setPlayerId] = useState(settings.lastPlayerId ?? "");
  const [shuffle, setShuffle] = useState(settings.shuffle);
  const [loadingPlayers, setLoadingPlayers] = useState(false);
  const [playing, setPlaying] = useState(false);
  const [feedback, setFeedback] = useState<Feedback>(null);

  // Playlists change whenever the library is re-indexed or the filter changes.
  useEffect(() => {
    api.playlists().then((list) => {
      setPlaylists(list);
      setPlaylistId((current) => (list.some((p) => p.id === current) ? current : (list[0]?.id ?? "")));
    });
  }, [status.loadedAt, status.songs, status.collections]);

  const loadPlayers = useCallback(async () => {
    if (!maConfigured) return;
    setLoadingPlayers(true);
    try {
      const list = await api.players();
      setPlayers(list);
      setPlayerId((current) => (list.some((p) => p.id === current) ? current : (list[0]?.id ?? "")));
      setFeedback(null);
    } catch (error) {
      setFeedback({ kind: "error", text: errorMessage(error) });
    } finally {
      setLoadingPlayers(false);
    }
  }, [maConfigured]);

  useEffect(() => {
    loadPlayers();
  }, [loadPlayers, settings.maUrl]);

  const play = async () => {
    setPlaying(true);
    setFeedback(null);
    try {
      await api.play(playerId, playlistId, shuffle);
      const playlist = playlists.find((p) => p.id === playlistId)?.name ?? "playlist";
      const player = players.find((p) => p.id === playerId)?.name ?? "speaker";
      setFeedback({ kind: "ok", text: `Playing ${playlist} on ${player}${shuffle ? ", shuffled" : ""}.` });
    } catch (error) {
      setFeedback({ kind: "error", text: errorMessage(error) });
    } finally {
      setPlaying(false);
    }
  };

  return (
    <section className="card">
      <h2>Play</h2>
      {!maConfigured && (
        <p className="hint">Add your Music Assistant address and token in Settings to play from here.</p>
      )}

      <label className="field">
        <span>Collection</span>
        <select value={playlistId} onChange={(e) => setPlaylistId(e.target.value)} disabled={!playlists.length}>
          {playlists.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name} ({p.songCount.toLocaleString()})
            </option>
          ))}
        </select>
      </label>

      <div className="field">
        <span>Order</span>
        <div className="segmented" role="radiogroup" aria-label="Order">
          <button role="radio" aria-checked={shuffle} className={shuffle ? "active" : ""} onClick={() => setShuffle(true)}>
            Shuffle
          </button>
          <button role="radio" aria-checked={!shuffle} className={!shuffle ? "active" : ""} onClick={() => setShuffle(false)}>
            In order
          </button>
        </div>
      </div>

      <label className="field">
        <span>Speaker</span>
        <div className="row">
          <select
            value={playerId}
            onChange={(e) => setPlayerId(e.target.value)}
            disabled={!maConfigured || !players.length}
          >
            {players.length === 0 && <option value="">{maConfigured ? "No speakers found" : "—"}</option>}
            {players.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
                {p.state === "playing" ? " (playing)" : ""}
              </option>
            ))}
          </select>
          <button
            className="icon-button"
            onClick={loadPlayers}
            disabled={!maConfigured || loadingPlayers}
            title="Refresh speakers"
            aria-label="Refresh speakers"
          >
            ↻
          </button>
        </div>
      </label>

      <button
        className="primary play"
        onClick={play}
        disabled={!maConfigured || !playerId || !playlistId || playing || !status.serverRunning}
      >
        {playing ? "Starting…" : "▶  Play"}
      </button>
      {feedback && <p className={`feedback ${feedback.kind}`}>{feedback.text}</p>}
    </section>
  );
}
