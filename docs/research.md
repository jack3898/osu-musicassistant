# Research: exposing osu!lazer songs to Music Assistant

Status: pre-build research, November 2026.
Sources: the current `ppy/osu` and `music-assistant/server` source trees, which were read directly, plus the links at the end.

---

## TL;DR

| Question | Answer |
|---|---|
| Can we read the osu!lazer library from outside the game? | **Yes, but only through its Realm database** (`client.realm`). Song titles, artists, the filename→hash mapping and collections exist **only** in Realm. The audio files themselves are plain files on disk, named by their SHA-256 hash. |
| Is there a ready-made Rust Realm reader? | **No, not one that works.** `realm-db-reader` (pure Rust) can't read lazer's file format v24. The workable options are FFI to realm-core's C API, a small .NET helper using the same Realm library as lazer, or extending a pure-Rust parser. See §1.5. |
| How should Music Assistant get the songs? | **Make the Rust app an (Open)Subsonic server.** MA has a built-in OpenSubsonic music provider. Collections become playlists, songs get proper title, artist and cover art, and streaming supports seeking. MA has no supported way to load third-party providers, so we can't ship our own provider. |
| How do we do "pick a collection + shuffle/sequential" from our app? | Use MA's **WebSocket API**: `player_queues/play_media` takes a playlist URI and a `shuffle: bool` flag. This needs a long-lived MA token. |
| Codecs? | osu! audio is almost always **MP3** or **OGG Vorbis**, occasionally something else. MA decodes everything with ffmpeg and re-encodes for each player itself, so **we never transcode**. We serve the original bytes with the correct MIME type and support HTTP Range. |
| Which playback devices are supported? | Whatever MA supports, and we don't have to care: AirPlay, Chromecast, Sonos, DLNA/UPnP, Squeezelite, Snapcast, Sendspin, HEOS, MusicCast, WiiM/LinkPlay, Bluesound, Home Assistant media players, Alexa, and more. |

---

## 1. How osu!lazer stores its library

### 1.1 Data directory

| OS | Default location |
|---|---|
| Windows | `%APPDATA%\osu` |
| Linux | `$XDG_DATA_HOME/osu` (usually `~/.local/share/osu`). Flatpak: `~/.var/app/sh.ppy.osu/data/osu` |
| macOS | `~/Library/Application Support/osu` |

Users can move the data folder in settings. When they do, `storage.ini` in the default directory holds `FullPath = <new path>` (`osu.Game/IO/OsuStorage.cs`). Our app must check `storage.ini` first and also let the user override the path.

Contents that matter to us:

```
<data dir>/
  client.realm            ← Realm database: all metadata, collections, file mappings
  client.realm.lock       ← Realm's multi-process lock file (+ .note / .management)
  files/
    a/ab/abcdef0123…      ← every imported file, stored by SHA-256, no extension
```

The path for a file is `files/{hash[0]}/{hash[0..2]}/{hash}` (`osu.Game/Extensions/ModelExtensions.cs: GetStoragePath`).
Release builds always use `client.realm`. Debug builds of lazer use `client_<schema>.realm`, which we can ignore.

### 1.2 Realm schema (the parts we need)

Lazer is on **Realm .NET 20.1.0**, which bundles **realm-core 20.1.2**. Its app-level **schema_version is 52** (last bump 2026-07-28). Realm stores classes under their `[MapTo]` names:

```
BeatmapSet  (BeatmapSetInfo)
  ID: Guid (PK)            OnlineID: int          DateAdded / DateSubmitted / DateRanked
  Beatmaps: List<Beatmap>  Files: List<RealmNamedFileUsage>
  DeletePending: bool      Protected: bool        Status: int     Hash: string

Beatmap  (BeatmapInfo)          ← one per difficulty
  ID: Guid  DifficultyName  Metadata: BeatmapMetadata  BeatmapSet: BeatmapSet
  MD5Hash: string  ← what collections reference
  Length (ms, gameplay length, NOT audio length)   BPM   Hidden   LastPlayed

BeatmapMetadata
  Title, TitleUnicode, Artist, ArtistUnicode, Source, Tags, UserTags: List<string>
  Author: RealmUser   PreviewTime: int   AudioFile: string   BackgroundFile: string
                                          ^ filenames, resolved via BeatmapSet.Files

RealmNamedFileUsage (embedded)  { File: File, Filename: string }
File                             { Hash: string (PK, SHA-256) }

BeatmapCollection
  ID: Guid  Name: string  BeatmapMD5Hashes: List<string>  LastModified
```

### 1.3 Resolving a song's audio file

```
Beatmap.Metadata.AudioFile  ("audio.mp3")
  → BeatmapSet.Files.where(Filename == "audio.mp3").File.Hash
  → <data>/files/h/ha/hash…
```

The same steps work for `BackgroundFile`, which gives us cover art.

### 1.4 Collections

Since 2022, collections live in Realm (`BeatmapCollection`) and no longer in stable's `collection.db`. Each collection is a **list of beatmap MD5 hashes**, one per difficulty. To turn a collection into a playlist:

1. MD5 → `Beatmap` (skip it if the map isn't installed; collections can reference missing maps)
2. → `BeatmapSet` → audio hash
3. **Dedupe by audio hash.** One set usually has 3–10 difficulties that share one audio file, though some sets have different audio per difficulty. Keep the first occurrence, so "sequential" follows the order the collection was built in.

"All" = every non-`DeletePending` set. Probably exclude `Protected` sets, which are lazer's bundled default maps.

### 1.5 Reading Realm from Rust: options

This is the main technical risk. There's no maintained, working Rust Realm library.

| Option | How it works | Pros | Cons |
|---|---|---|---|
| **A. realm-core C API via FFI** | Build realm-core (Apache-2.0, C++17/CMake) from `build.rs`, `bindgen` its `realm.h` C API (≈426 `RLM_API` functions), and open the file with `RLM_SCHEMA_MODE_READ_ONLY`/`IMMUTABLE` and a dynamic schema (`realm_get_class_keys`, `realm_get_schema`). | One native binary. Uses the same engine as lazer, so format compatibility follows whichever core version we pin. | Heavy C++ build in CI for 3 OSes. `unsafe` bindings to maintain. MongoDB stopped maintaining the Realm SDKs in Sept 2025 and they now live on as community open source. |
| **B. .NET helper process** | A small C# tool references the `Realm` NuGet package at lazer's version, copies and opens `client.realm`, and prints JSON. The Rust app runs it. | Proven: `osu-sync` and `lazer2stable` both do exactly this. Low effort, and we can copy lazer's model classes. | Two languages. Needs a .NET runtime or a self-contained/AOT publish (tens of MB). Must track lazer's Realm version. |
| **C. Pure-Rust parser** | Extend `realm-db-reader` (MIT, v0.2.1, Aug 2025) or write a read-only parser for file format 24 that only handles the types we need. | Cleanest end result: pure Rust and trivial cross-compiling. | Most effort: reverse-engineering B-tree, array and string encodings from realm-core sources. Breaks if lazer upgrades core and the format changes. |

Hidden option D, rejected: scan `files/` for `.osu` files and parse them. That gets titles but **not** which hash is the audio file, and **no collections**. It's a dead end.

**Recommendation:** put Realm access behind a `LibrarySource` trait, then run a **1–2 day spike on option A**. If it builds cleanly on Windows, macOS and Linux with sync and encryption turned off, we ship a single binary. If the C++ build turns into a time sink, fall back to **B**, which is proven to work, and keep **C** as a long-term "pure Rust" goal.

### 1.6 Safety: never touch the live database

- **Always copy `client.realm` to a temp dir and open the copy read-only.** Opening the live file from a different realm-core build risks lock-file or format mismatches, and in the worst case a file-format upgrade of the user's database.
- A copy made while lazer is writing may be torn. Validate it, retry on failure, and prefer re-snapshotting after lazer goes idle or quits.
- Watch `client.realm` with the `notify` crate and re-index (debounced) when it changes, e.g. when the user imports maps or edits a collection.
- Audio files are content-addressed and immutable, so streaming them straight from `files/` is safe.

---

## 2. Music Assistant: how we get music in

MA (Python server, usually the Home Assistant add-on) has three kinds of providers: **music providers** (sources), **player providers** (output devices) and **plugins**. Providers are loaded **only from the server's own `providers/` directory** (`mass.py: __load_provider_manifests`). There's no supported way to install a third-party provider, so a custom Python provider would mean forking MA or getting it merged upstream.

### 2.1 Integration options

| Option | What MA sees | Shuffle/sequential | Effort | Verdict |
|---|---|---|---|---|
| **1. Rust app is an OpenSubsonic server** | A full library (artists, albums, tracks) with **collections as playlists**, cover art and seekable streams | MA's queue shuffle, or driven from our app via the WS API (§2.3) | Medium | **Recommended** |
| 2. Rust app controls MA and serves raw HTTP audio | Nothing in the library. Our app sends `http://pc:port/track/<hash>` URLs into a queue via `play_media`, and the builtin provider plays them | Our app orders the list | Low | Good as a throwaway prototype. Metadata comes from file tags, which osu! MP3s often lack, so the queue shows ugly names. |
| 3. Expose files over SMB/WebDAV with symlinks + `.m3u` | Filesystem provider scans the files | MA | Low–Med | Weak. MA's filesystem provider relies on embedded tags, and osu! files have bad or missing tags. |
| 4. Jellyfin/Emby/Plex-compatible API | Library | MA | High | Much bigger API surface than Subsonic for no extra benefit |
| 5. Upstream MA provider (Python) | Native | MA | Medium + review process | Not Rust, and depends on upstream acceptance. Maybe later. |

### 2.2 What MA's OpenSubsonic provider needs from us

From `providers/opensubsonic/sonic_provider.py` (it uses `py-opensonic==10.4.1`):

- **Config:** base URL, port, path, and username+password (Subsonic token auth: `t = md5(password + salt)`, `s = salt`) **or** an API key (OpenSubsonic `apiKeyAuthentication` extension).
- **Library sync:** tracks come from paginated **`search3` with an empty query** (Navidrome-style), artists from `getArtists`/`getArtist`, albums from `getAlbumList2`/`getAlbum`, playlists from `getPlaylists`/`getPlaylist`.
- **Playback:** `getSong` (for `contentType`, `samplingRate`, `bitDepth`, `channelCount`), then `stream` (optionally `format=raw`). MA marks these streams `can_seek=True`, so **`stream` must honour HTTP `Range`**.
- **Other calls** it makes, which we can stub with empty or "not found" responses: `ping`, `getOpenSubsonicExtensions`, `getCoverArt` (real: serve the background image), `getStarred2`, `getTopSongs`, `getSimilarSongs`, `getArtistInfo2`, `getAlbumInfo2`, `getLyricsBySongId`, `getInternetRadioStations`, `getPodcasts`, `getNewestPodcasts`, `getBookmarks`, plus the write calls (`star`, `createPlaylist`, `updatePlaylist`…). Return errors for write calls, or support them later.

Proposed mapping:

| Subsonic | osu! |
|---|---|
| Song (id = audio SHA-256) | unique audio file |
| Album | beatmap set (title = song title; cover = background image) |
| Artist | `Artist` / `ArtistUnicode` (user setting: romanised vs. original script) |
| Playlist | a collection, plus a virtual **"osu! — All songs"** playlist |
| `year` | `DateRanked` or `DateSubmitted` |
| `duration` | probed from the audio (gameplay `Length` is NOT audio length) |

Testing tip: write integration tests that drive our server with **`py-opensonic` 10.4.1**, the exact client MA uses.

### 2.3 Controlling playback from our app (collection + shuffle/sequential)

MA exposes a WebSocket API at `ws://<ma-host>:8095/ws`:

1. Send the `auth` command with a token. The user creates a **long-lived token** (1-year expiry) in MA's settings and pastes it into our app.
2. `players/all` or `player_queues/all` lists the target devices for a picker.
3. `player_queues/play_media` with these parameters:
   - `queue_id`: the chosen player
   - `media`: the playlist's MA URI (look it up in MA's library, or use the provider URI `<opensubsonic-instance>://playlist/<collection-id>`)
   - `option: "replace"`
   - **`shuffle: true | false`**. The source says: *"Play the media shuffled (or explicitly in order). Only applies to the options that start playing right away (play/replace)."*

   `player_queues/shuffle` and `player_queues/repeat` are also there for later toggling.

This gives exactly the MVP UX: **[Collection ▾ (All / …)] [Shuffle | Sequential] [Player ▾] [▶ Play]**. Without the token the feature still degrades gracefully: the user can open the playlist in MA and press shuffle there.

### 2.4 Devices (player providers in MA today)

AirPlay, Chromecast, DLNA/UPnP, Sonos (and S1), Squeezelite/Slimproto, Snapcast, **Sendspin** (MA's own synchronised-streaming protocol), HEOS, Yamaha MusicCast, WiiM/LinkPlay, Bluesound, Bose SoundTouch, Samsung WAM, Teufel Raumfeld, AmpliPi, MPD, Fully Kiosk, Roku (Media Assistant), Alexa, Yandex Station, Home Assistant media players, local audio out, plus sync/universal groups.
**All of these work without changes on our side.** MA pulls from our server, decodes, and re-streams to each device in whatever format that protocol needs.

---

## 3. Audio formats and codecs

### 3.1 What's in an osu! library

- **Ranking criteria:** song audio must be **MP3 (128–192 kbps avg)** or **OGG Vorbis (128–208 kbps avg)**. WAV is only allowed for hitsounds.
- **In practice:** ranked maps are overwhelmingly MP3, some are OGG, and unranked/graveyard/older maps can contain odd files: low-bitrate MPEG-2/2.5 MP3s, wrong extensions (OGG data named `.mp3`), and occasionally WAV.
- **Tags:** ID3/Vorbis comments are often missing or junk. **Realm metadata is the source of truth**, never file tags.

### 3.2 What we do with it

- **No transcoding.** MA runs everything through ffmpeg, so MP3, Vorbis, WAV, FLAC and Opus are all fine, and it re-encodes per player (FLAC, PCM, MP3, AAC/ALAC, Opus depending on protocol). Serving originals keeps our app light and lossless relative to the source.
- **Correct `contentType`/`suffix`.** Take the extension from the Realm filename, then **verify with magic bytes** (`ID3`/`FF Fx` → `audio/mpeg`, `OggS` → `audio/ogg`, `RIFF…WAVE` → `audio/wav`, `fLaC` → `audio/flac`).
- **Duration and stream info.** Probe each file once with **`symphonia`** (pure Rust: MP3, Vorbis, WAV, FLAC, AAC) or **`lofty`**, and cache it in a small SQLite DB keyed by **SHA-256**. Files are content-addressed, so cache entries never go stale.
- **Loudness.** osu! tracks vary a lot in volume. MA has its own loudness measurement and normalisation, so we don't need to do anything.
- **Cover art.** Serve `BackgroundFile` (JPG/PNG) through `getCoverArt`, with optional resizing via the `image` crate. Video backgrounds are ignored.

---

## 4. Proposed architecture

```
┌──────────────── Desktop app (Rust) ─────────────────┐
│  UI / tray (Tauri 2)                                │
│   collection picker · shuffle/sequential · player   │
│                                                     │
│  LazerLibrary ──(LibrarySource trait)──┐            │
│   • locate data dir (+ storage.ini)    │            │
│   • snapshot client.realm → read       │            │
│   • notify watcher → re-index          │            │
│  Index (in-memory + SQLite cache)   ◄──┘            │
│   songs by audio hash · sets · collections          │
│                                                     │
│  Subsonic server (axum)   ◄── MA OpenSubsonic       │
│   search3/getPlaylist/stream(Range)/getCoverArt…    │      provider pulls
│                                                     │
│  MA client (tokio-tungstenite) ──► MA WebSocket API │
│   play_media(playlist, shuffle)                     │
└─────────────────────────────────────────────────────┘
          serves files from  <osu data>/files/…
```

Suggested crates: `tauri` 2 (UI, tray, autostart, single-instance plugins), `axum` + `tower-http` (server and Range), `tokio`, `serde`/`serde_json` (+ `quick-xml` if we need XML responses), `notify`, `rusqlite`, `symphonia` or `lofty`, `image`, `md-5` (Subsonic token auth), `tokio-tungstenite`, `directories`. Optional: `mdns-sd` for discovery. Realm access depends on the §1.5 spike: `bindgen` + `cmake`, a `.NET` helper, or a parser.

Networking: MA usually runs on another machine (an HA box). The server must bind to a LAN interface, show the user the URL to paste into MA (LAN IP, not `localhost`), and expect a Windows Firewall prompt. Keep it LAN-only: it's copyrighted audio for personal use.

---

## 5. MVP scope

1. Detect the lazer data dir and index songs and collections from a Realm snapshot.
2. OpenSubsonic server: auth, `ping`, `getOpenSubsonicExtensions`, `getArtists`/`getArtist`, `getAlbumList2`/`getAlbum`, `getSong`, `search3`, `getPlaylists`/`getPlaylist` (collections + "All songs"), `stream` with Range, `getCoverArt`, plus stubs for the rest.
3. Desktop UI: status, the URL and credentials to paste into MA, collection dropdown (All + each collection), **Shuffle / Sequential** toggle, player dropdown, Play button (MA WebSocket + long-lived token).
4. Re-index when `client.realm` changes.

Out of scope for MVP: editing collections from MA, scrobbling and play counts, starring, lyrics, transcoding, internet exposure.

---

## 6. Open questions and risks

| Risk | Mitigation |
|---|---|
| realm-core C++ build pain across 3 OSes | Spike first. The .NET helper is a proven fallback. |
| Lazer upgrades Realm/core and the file format changes | Pin our core to lazer's version, detect the format version on open, and fail clearly with "update osu-musicassistant". Watch `osu.Game.csproj` in CI. |
| Torn snapshot while lazer writes | Validate, retry and debounce. Optionally detect whether lazer is running. |
| MA's Subsonic client is strict about some fields | Integration-test against `py-opensonic` 10.4.1 and a real MA dev instance. |
| Playlist URI lookup for `play_media` | Confirm the exact URI form during implementation (library item vs. provider URI). |
| MA API/auth changes | Its WS API is versioned with the server. Keep the MA client thin. |

---

## 7. Sources

- osu!lazer source: <https://github.com/ppy/osu>. Key files: `osu.Game/Database/RealmAccess.cs`, `osu.Game/Beatmaps/BeatmapSetInfo.cs`, `BeatmapInfo.cs`, `BeatmapMetadata.cs`, `osu.Game/Collections/BeatmapCollection.cs`, `osu.Game/Models/RealmFile.cs`, `RealmNamedFileUsage.cs`, `osu.Game/Extensions/ModelExtensions.cs`, `osu.Game/IO/OsuStorage.cs`, `osu.Game/osu.Game.csproj`
- Realm .NET (community): <https://github.com/realm/realm-dotnet>. realm-core: <https://github.com/realm/realm-core> (`src/realm.h` C API)
- MongoDB Atlas Device SDK deprecation: <https://mongodb.com/docs/atlas/device-sdks/deprecation>
- `realm-db-reader` crate: <https://crates.io/crates/realm-db-reader> (repo: <https://github.com/maartenstaa/realm-db-reader>)
- osu-sync PR documenting the format-24 problem and the .NET helper approach: <https://github.com/Finesssee/osu-sync/pull/3>
- lazer2stable (C# bridge + Rust): <https://github.com/RiaLnN/lazer2stable>
- Music Assistant server: <https://github.com/music-assistant/server>. Key files: `music_assistant/providers/opensubsonic/*`, `controllers/player_queues/controller.py`, `controllers/webserver/README.md`, `helpers/uri.py`, `mass.py`
- OpenSubsonic API spec: <https://opensubsonic.netlify.app/>
- osu! audio guidance: <https://osu.ppy.sh/wiki/en/Guides/Compressing_files>, <https://osu.ppy.sh/wiki/en/Ranking_criteria>
