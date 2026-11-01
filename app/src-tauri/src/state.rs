//! Everything the app runs: the indexed library, the Subsonic server, the database watcher and
//! the Music Assistant connection.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lazer_library::{DataDir, Library, locate_data_dir};
use ma_client::{MaClient, MaError};
use notify_debouncer_mini::notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{Debouncer, new_debouncer};
use serde::Serialize;
use subsonic_server::Server;
use subsonic_server::catalog::{ALL_SONGS_PLAYLIST_ID, ALL_SONGS_PLAYLIST_NAME};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinHandle;

use crate::settings::{Settings, load_ma_token};

/// Run `$body` with `$client` bound to a connected [`MaClient`], reconnecting and retrying once
/// if the cached connection turns out to have dropped. (A macro rather than a closure because
/// async closures can't yet be proven `Send`.)
macro_rules! with_ma {
    ($state:expr, |$client:ident| $body:expr) => {{
        let mut result = Err(MaError::Closed);
        for _attempt in 0..2 {
            let mut session = $state.ma_session().await?;
            let $client = &session.as_ref().expect("connected above").client;
            result = $body.await;
            if matches!(
                result,
                Err(MaError::Closed | MaError::Connect(_) | MaError::Timeout)
            ) {
                *session = None;
                continue;
            }
            break;
        }
        result.map_err(|e| e.to_string())
    }};
}

/// How long the database must stay quiet before re-indexing. Lazer writes in bursts.
const REINDEX_DEBOUNCE: Duration = Duration::from_secs(5);

pub struct AppState {
    settings_path: PathBuf,
    audio_cache_path: PathBuf,
    pub settings: Mutex<Settings>,
    runtime: AsyncMutex<Runtime>,
    ma: AsyncMutex<Option<MaSession>>,
}

#[derive(Default)]
struct Runtime {
    data_dir: Option<DataDir>,
    library: Option<Arc<Library>>,
    library_error: Option<String>,
    loaded_at: Option<SystemTime>,
    server: Option<Server>,
    listener: Option<JoinHandle<()>>,
    port: Option<u16>,
    server_error: Option<String>,
    watcher: Option<Debouncer<RecommendedWatcher>>,
}

struct MaSession {
    client: MaClient,
    url: String,
    token: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub data_dir: Option<String>,
    pub songs: usize,
    pub collections: usize,
    /// Seconds since the Unix epoch.
    pub loaded_at: Option<u64>,
    pub library_error: Option<String>,
    pub server_running: bool,
    pub port: u16,
    pub addresses: Vec<String>,
    pub username: String,
    pub password: String,
    pub server_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistInfo {
    pub id: String,
    pub name: String,
    pub song_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerInfo {
    pub id: String,
    pub name: String,
    pub state: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaInfo {
    pub server_name: Option<String>,
    pub server_version: String,
    /// Name of the OpenSubsonic provider serving this library, if Music Assistant has one.
    pub provider: Option<String>,
}

impl AppState {
    pub fn new(settings_path: PathBuf, audio_cache_path: PathBuf) -> Self {
        let settings = Settings::load_or_create(&settings_path);
        Self {
            settings_path,
            audio_cache_path,
            settings: Mutex::new(settings),
            runtime: AsyncMutex::default(),
            ma: AsyncMutex::default(),
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    pub fn save_settings(&self, settings: Settings) -> Result<(), String> {
        settings
            .save(&self.settings_path)
            .map_err(|e| format!("couldn't save settings: {e}"))?;
        *self.settings.lock().unwrap() = settings;
        Ok(())
    }

    /// Index the library and start serving it. Safe to call again to start over, e.g. after
    /// the osu! folder changes.
    pub async fn start(&self, app: &AppHandle) {
        {
            let mut rt = self.runtime.lock().await;
            if let Some(listener) = rt.listener.take() {
                listener.abort();
            }
            *rt = Runtime::default();
        }
        self.reload_library(app).await;
        self.restart_server(app).await;
        self.watch_database(app).await;
    }

    /// Re-read lazer's database and hand the result to the server.
    pub async fn reload_library(&self, app: &AppHandle) {
        let settings = self.settings();
        let mut rt = self.runtime.lock().await;
        let data_dir = match &settings.osu_data_dir {
            Some(dir) => DataDir::new(dir),
            None => locate_data_dir(),
        };
        let Some(data_dir) = data_dir else {
            rt.library_error = Some(match &settings.osu_data_dir {
                Some(dir) => format!("No osu!lazer database (client.realm) in {}", dir.display()),
                None => "Couldn't find osu!lazer. Choose its data folder in Settings.".into(),
            });
            drop(rt);
            self.emit_status(app).await;
            return;
        };

        let load_dir = data_dir.clone();
        match tokio::task::spawn_blocking(move || Library::load(&load_dir)).await {
            Ok(Ok(library)) => {
                let library = Arc::new(library);
                match &rt.server {
                    Some(server) if rt.data_dir.as_ref() == Some(&data_dir) => {
                        server.set_library(library.clone(), &settings.view_options());
                    }
                    _ => {
                        rt.server = Some(Server::new(
                            data_dir.clone(),
                            settings.credentials(),
                            library.clone(),
                            &settings.view_options(),
                        ));
                    }
                }
                tracing::info!(songs = library.songs.len(), "library indexed");
                rt.data_dir = Some(data_dir);
                rt.library = Some(library);
                rt.library_error = None;
                rt.loaded_at = Some(SystemTime::now());
                if let Some(server) = rt.server.clone() {
                    let cache = self.audio_cache_path.clone();
                    tokio::task::spawn_blocking(move || server.probe_audio(Some(cache)));
                }
            }
            Ok(Err(err)) => rt.library_error = Some(err.to_string()),
            Err(err) => rt.library_error = Some(err.to_string()),
        }
        drop(rt);
        self.emit_status(app).await;
    }

    /// (Re)bind the Subsonic server to the configured port.
    pub async fn restart_server(&self, app: &AppHandle) {
        let port = self.settings().port;
        let mut rt = self.runtime.lock().await;
        if let Some(listener) = rt.listener.take() {
            listener.abort();
        }
        rt.port = None;
        if let Some(server) = rt.server.clone() {
            match tokio::net::TcpListener::bind(("0.0.0.0", port)).await {
                Ok(listener) => {
                    rt.listener = Some(tokio::spawn(async move {
                        if let Err(err) = server.serve(listener).await {
                            tracing::error!(%err, "subsonic server stopped");
                        }
                    }));
                    rt.port = Some(port);
                    rt.server_error = None;
                    tracing::info!(port, "subsonic server listening");
                }
                Err(err) => rt.server_error = Some(format!("Couldn't use port {port}: {err}")),
            }
        }
        drop(rt);
        self.emit_status(app).await;
    }

    /// Apply changed settings to the running server without restarting it.
    pub async fn apply_settings(&self, app: &AppHandle, old: &Settings) {
        let new = self.settings();
        if new.osu_data_dir != old.osu_data_dir {
            return self.start(app).await;
        }
        {
            let rt = self.runtime.lock().await;
            if let (Some(server), Some(library)) = (&rt.server, &rt.library) {
                if new.credentials() != old.credentials() {
                    server.set_credentials(new.credentials());
                }
                if new.view_options() != old.view_options() {
                    server.set_library(library.clone(), &new.view_options());
                }
            }
        }
        if new.port != old.port {
            self.restart_server(app).await;
        } else {
            self.emit_status(app).await;
        }
        if new.ma_url != old.ma_url {
            *self.ma.lock().await = None;
        }
    }

    /// Re-index whenever lazer's database changes (maps imported, collections edited, ...).
    async fn watch_database(&self, app: &AppHandle) {
        let mut rt = self.runtime.lock().await;
        let Some(data_dir) = rt.data_dir.clone() else {
            return;
        };
        let handle = app.clone();
        let debouncer = new_debouncer(
            REINDEX_DEBOUNCE,
            move |events: notify_debouncer_mini::DebounceEventResult| {
                let touched_db = events.is_ok_and(|events| {
                    events
                        .iter()
                        .any(|e| e.path.file_name().is_some_and(|n| n == "client.realm"))
                });
                if touched_db {
                    let handle = handle.clone();
                    tauri::async_runtime::spawn(async move {
                        tracing::info!("osu! database changed, re-indexing");
                        handle.state::<AppState>().reload_library(&handle).await;
                    });
                }
            },
        );
        match debouncer {
            Ok(mut debouncer) => match debouncer
                .watcher()
                .watch(data_dir.root(), RecursiveMode::NonRecursive)
            {
                Ok(()) => rt.watcher = Some(debouncer),
                Err(err) => tracing::warn!(%err, "couldn't watch the osu! database for changes"),
            },
            Err(err) => tracing::warn!(%err, "couldn't watch the osu! database for changes"),
        }
    }

    pub async fn status(&self) -> Status {
        let settings = self.settings();
        let rt = self.runtime.lock().await;
        let catalog = rt.server.as_ref().map(|s| s.catalog());
        Status {
            data_dir: rt.data_dir.as_ref().map(|d| d.root().display().to_string()),
            songs: catalog
                .as_ref()
                .map_or(0, |c| c.playlists.first().map_or(0, |p| p.songs.len())),
            collections: catalog
                .as_ref()
                .map_or(0, |c| c.playlists.len().saturating_sub(1)),
            loaded_at: rt
                .loaded_at
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs()),
            library_error: rt.library_error.clone(),
            server_running: rt.port.is_some(),
            port: settings.port,
            addresses: subsonic_server::lan_ips()
                .iter()
                .map(|ip| format!("http://{ip}"))
                .collect(),
            username: settings.username,
            password: settings.password,
            server_error: rt.server_error.clone(),
        }
    }

    pub async fn emit_status(&self, app: &AppHandle) {
        let _ = app.emit("status", self.status().await);
    }

    /// "All songs" first, then collections alphabetically.
    pub async fn playlists(&self) -> Vec<PlaylistInfo> {
        let rt = self.runtime.lock().await;
        let Some(server) = &rt.server else {
            return Vec::new();
        };
        server
            .catalog()
            .playlists
            .iter()
            .map(|p| PlaylistInfo {
                id: p.id.clone(),
                name: p.name.clone(),
                song_count: p.songs.len(),
            })
            .collect()
    }

    /// The cached Music Assistant session, connecting first if there is none (or the address or
    /// token changed since).
    async fn ma_session(&self) -> Result<tokio::sync::MutexGuard<'_, Option<MaSession>>, String> {
        let url = self.settings().ma_url.trim().to_string();
        if url.is_empty() {
            return Err("Enter your Music Assistant address in Settings first.".into());
        }
        let token = load_ma_token().ok_or("Add a Music Assistant token in Settings first.")?;
        let mut session = self.ma.lock().await;
        if session
            .as_ref()
            .is_none_or(|s| s.url != url || s.token != token)
        {
            let client = MaClient::connect(&url, &token)
                .await
                .map_err(|e| e.to_string())?;
            *session = Some(MaSession { client, url, token });
        }
        Ok(session)
    }

    pub async fn forget_ma_session(&self) {
        *self.ma.lock().await = None;
    }

    pub async fn music_assistant_info(&self) -> Result<MaInfo, String> {
        let provider = with_ma!(self, |client| client
            .find_subsonic_provider(ALL_SONGS_PLAYLIST_ID, ALL_SONGS_PLAYLIST_NAME))?;
        let session = self.ma_session().await?;
        let info = session
            .as_ref()
            .expect("connected above")
            .client
            .server_info();
        Ok(MaInfo {
            server_name: info.name.clone(),
            server_version: info.server_version.clone(),
            provider: provider.map(|p| p.name),
        })
    }

    pub async fn players(&self) -> Result<Vec<PlayerInfo>, String> {
        let players = with_ma!(self, |client| client.players())?;
        Ok(players
            .into_iter()
            .map(|p| PlayerInfo {
                id: p.player_id.clone(),
                name: p.label().to_string(),
                state: p.playback_state,
            })
            .collect())
    }

    pub async fn play(
        &self,
        player_id: &str,
        playlist_id: &str,
        shuffle: bool,
    ) -> Result<(), String> {
        let provider = with_ma!(self, |client| client
            .find_subsonic_provider(ALL_SONGS_PLAYLIST_ID, ALL_SONGS_PLAYLIST_NAME))?;
        let Some(provider) = provider else {
            return Err("Music Assistant doesn't have this library yet. Add it as an OpenSubsonic provider \
                        using the details under \"Connect Music Assistant\"."
                .into());
        };
        with_ma!(self, |client| client.play_playlist(
            player_id,
            &provider.instance_id,
            playlist_id,
            shuffle
        ))?;
        let mut settings = self.settings();
        settings.last_player_id = Some(player_id.into());
        settings.last_playlist_id = Some(playlist_id.into());
        settings.shuffle = shuffle;
        self.save_settings(settings)
    }
}
