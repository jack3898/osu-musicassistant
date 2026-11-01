// Typed wrappers around the Rust commands in src-tauri/src/commands.rs.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface Status {
  dataDir: string | null;
  songs: number;
  collections: number;
  loadedAt: number | null;
  libraryError: string | null;
  serverRunning: boolean;
  port: number;
  addresses: string[];
  username: string;
  password: string;
  serverError: string | null;
}

export interface Settings {
  osuDataDir: string | null;
  port: number;
  username: string;
  password: string;
  hideRateEdits: boolean;
  blockedPatterns: string[];
  preferUnicode: boolean;
  collectionPrefix: string;
  maUrl: string;
  keepInTray: boolean;
  launchAtLogin: boolean;
  lastPlayerId: string | null;
  lastPlaylistId: string | null;
  shuffle: boolean;
  hasMaToken: boolean;
}

export type SettingsUpdate = Pick<
  Settings,
    | "osuDataDir"
  | "port"
  | "hideRateEdits"
  | "blockedPatterns"
  | "preferUnicode"
  | "collectionPrefix"
  | "maUrl"
  | "keepInTray"
  | "launchAtLogin"
>;

export interface Playlist {
  id: string;
  name: string;
  songCount: number;
}

export interface Player {
  id: string;
  name: string;
  state: string | null;
}

export interface MaInfo {
  serverName: string | null;
  serverVersion: string;
  provider: string | null;
}

export const api = {
  status: () => invoke<Status>("get_status"),
  settings: () => invoke<Settings>("get_settings"),
  saveSettings: (update: SettingsUpdate) => invoke<Settings>("save_settings", { update }),
  regeneratePassword: () => invoke<Settings>("regenerate_password"),
  setMaToken: (token: string) => invoke<Settings>("set_ma_token", { token }),
  reindex: () => invoke<Status>("reindex"),
  playlists: () => invoke<Playlist[]>("list_playlists"),
  testMusicAssistant: () => invoke<MaInfo>("test_music_assistant"),
  players: () => invoke<Player[]>("list_players"),
  play: (playerId: string, playlistId: string, shuffle: boolean) =>
    invoke<void>("play", { playerId, playlistId, shuffle }),
  onStatus: (handler: (status: Status) => void): Promise<UnlistenFn> =>
    listen<Status>("status", (event) => handler(event.payload)),
};

/** Tauri rejects with the Rust error string; normalise anything else. */
export function errorMessage(error: unknown): string {
  return typeof error === "string" ? error : error instanceof Error ? error.message : String(error);
}
