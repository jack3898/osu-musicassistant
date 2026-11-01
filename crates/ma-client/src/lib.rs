//! A small client for Music Assistant's WebSocket API: just enough to list players and start an
//! osu! playlist on one of them, shuffled or in order.
//!
//! Protocol: the server greets with its server info, then each request is
//! `{"message_id", "command", "args"}` and answered with `{"message_id", "result"}` (list results
//! may arrive as several `"partial": true` chunks) or `{"message_id", "error_code", "details"}`.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::SinkExt;
use futures_util::stream::{SplitSink, StreamExt};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
pub enum MaError {
    #[error("invalid Music Assistant URL: {0}")]
    InvalidUrl(String),
    #[error("couldn't connect to Music Assistant: {0}")]
    Connect(String),
    #[error("timed out waiting for Music Assistant")]
    Timeout,
    #[error("the connection to Music Assistant was closed")]
    Closed,
    #[error("Music Assistant rejected the token: {0}")]
    Auth(String),
    #[error("Music Assistant error {code}: {details}")]
    Command { code: i64, details: String },
    #[error("unexpected response from Music Assistant: {0}")]
    Protocol(String),
}

pub type Result<T, E = MaError> = std::result::Result<T, E>;

#[derive(Debug, Clone, Deserialize)]
pub struct ServerInfo {
    pub server_id: String,
    pub server_version: String,
    pub schema_version: u32,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Player {
    pub player_id: String,
    pub name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    pub available: bool,
    /// Instance ID of the player provider (AirPlay, Sonos, ...).
    pub provider: String,
    #[serde(rename = "type")]
    pub player_type: String,
    #[serde(default)]
    pub playback_state: Option<String>,
    /// Set when this player is currently synced to (following) another player.
    #[serde(default)]
    pub synced_to: Option<String>,
}

impl Player {
    pub fn label(&self) -> &str {
        self.display_name
            .as_deref()
            .filter(|n| !n.is_empty())
            .unwrap_or(&self.name)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderInstance {
    pub instance_id: String,
    pub domain: String,
    pub name: String,
    pub available: bool,
}

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
type Pending = Arc<Mutex<HashMap<String, PendingCommand>>>;

struct PendingCommand {
    reply: oneshot::Sender<Result<Value>>,
    /// Items from `"partial": true` chunks, prepended to the final result.
    partial: Vec<Value>,
}

/// A connected, authenticated Music Assistant session.
pub struct MaClient {
    sink: Mutex<SplitSink<Socket, Message>>,
    pending: Pending,
    next_id: AtomicU64,
    server_info: ServerInfo,
    reader: tokio::task::JoinHandle<()>,
}

impl Drop for MaClient {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

impl MaClient {
    /// Connect to `server_url` (e.g. `https://music.example.com` or `http://192.168.1.5:8095`) and
    /// authenticate with a long-lived token.
    pub async fn connect(server_url: &str, token: &str) -> Result<Self> {
        let ws_url = websocket_url(server_url)?;
        // rustls needs a process-wide crypto backend; ignore the error if one is already set.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (socket, _) = tokio::time::timeout(CONNECT_TIMEOUT, connect_async(ws_url.as_str()))
            .await
            .map_err(|_| MaError::Timeout)?
            .map_err(|e| MaError::Connect(e.to_string()))?;
        let (sink, mut stream) = socket.split();

        // The first message is always the server info.
        let server_info: ServerInfo = loop {
            let message = tokio::time::timeout(CONNECT_TIMEOUT, stream.next())
                .await
                .map_err(|_| MaError::Timeout)?
                .ok_or(MaError::Closed)?
                .map_err(|e| MaError::Connect(e.to_string()))?;
            if let Message::Text(text) = message {
                break serde_json::from_str(&text).map_err(|e| MaError::Protocol(e.to_string()))?;
            }
        };

        let pending: Pending = Arc::default();
        let reader = tokio::spawn(read_loop(stream, pending.clone()));
        let client = Self {
            sink: Mutex::new(sink),
            pending,
            next_id: AtomicU64::new(1),
            server_info,
            reader,
        };
        client
            .raw_command("auth", json!({ "token": token }))
            .await
            .map_err(|e| match e {
                MaError::Command { details, .. } => MaError::Auth(details),
                other => other,
            })?;
        Ok(client)
    }

    pub fn server_info(&self) -> &ServerInfo {
        &self.server_info
    }

    /// Send any API command and decode its result.
    pub async fn command<T: DeserializeOwned>(&self, command: &str, args: Value) -> Result<T> {
        let value = self.raw_command(command, args).await?;
        serde_json::from_value(value).map_err(|e| MaError::Protocol(format!("{command}: {e}")))
    }

    async fn raw_command(&self, command: &str, args: Value) -> Result<Value> {
        let message_id = self.next_id.fetch_add(1, Ordering::Relaxed).to_string();
        let (reply, response) = oneshot::channel();
        self.pending.lock().await.insert(
            message_id.clone(),
            PendingCommand {
                reply,
                partial: Vec::new(),
            },
        );

        let message =
            json!({ "message_id": message_id, "command": command, "args": args }).to_string();
        if let Err(e) = self.sink.lock().await.send(Message::text(message)).await {
            self.pending.lock().await.remove(&message_id);
            return Err(MaError::Connect(e.to_string()));
        }
        match tokio::time::timeout(COMMAND_TIMEOUT, response).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(MaError::Closed),
            Err(_) => {
                self.pending.lock().await.remove(&message_id);
                Err(MaError::Timeout)
            }
        }
    }

    /// Players that can currently play, excluding ones synced to another player.
    pub async fn players(&self) -> Result<Vec<Player>> {
        let players: Vec<Player> = self.command("players/all", json!({})).await?;
        let mut players: Vec<Player> = players
            .into_iter()
            .filter(|p| p.available && p.synced_to.is_none())
            .collect();
        players.sort_by_key(|p| p.label().to_lowercase());
        Ok(players)
    }

    pub async fn music_providers(&self) -> Result<Vec<ProviderInstance>> {
        self.command("providers", json!({ "provider_type": "music" }))
            .await
    }

    /// Find the OpenSubsonic provider instance pointing at our server: the one that knows a
    /// playlist `marker_playlist_id` named `marker_playlist_name`.
    pub async fn find_subsonic_provider(
        &self,
        marker_playlist_id: &str,
        marker_playlist_name: &str,
    ) -> Result<Option<ProviderInstance>> {
        for provider in self.music_providers().await? {
            if provider.domain != "opensubsonic" || !provider.available {
                continue;
            }
            let uri = playlist_uri(&provider.instance_id, marker_playlist_id);
            match self
                .command::<Value>("music/item_by_uri", json!({ "uri": uri }))
                .await
            {
                Ok(item)
                    if item.get("name").and_then(Value::as_str) == Some(marker_playlist_name) =>
                {
                    return Ok(Some(provider));
                }
                Ok(_) | Err(MaError::Command { .. }) => continue,
                Err(other) => return Err(other),
            }
        }
        Ok(None)
    }

    /// Replace `player_id`'s queue with a playlist from `provider_instance` and start playing it.
    pub async fn play_playlist(
        &self,
        player_id: &str,
        provider_instance: &str,
        playlist_id: &str,
        shuffle: bool,
    ) -> Result<()> {
        let args = json!({
            "queue_id": player_id,
            "media": playlist_uri(provider_instance, playlist_id),
            "option": "replace",
            "shuffle": shuffle,
        });
        self.raw_command("player_queues/play_media", args)
            .await
            .map(|_| ())
    }

    /// Library playlists imported from `provider_instance`, as full item objects.
    pub async fn library_playlists(&self, provider_instance: &str) -> Result<Vec<Value>> {
        const PAGE: usize = 500;
        let mut playlists = Vec::new();
        loop {
            let page: Vec<Value> = self
                .command(
                    "music/playlists/library_items",
                    json!({
                        "provider": provider_instance,
                        "summary": false,
                        "limit": PAGE,
                        "offset": playlists.len(),
                    }),
                )
                .await?;
            let done = page.len() < PAGE;
            playlists.extend(page);
            if done {
                return Ok(playlists);
            }
        }
    }

    /// Rename a library playlist (an item from [`Self::library_playlists`]). Library syncs never
    /// rename playlists Music Assistant already has, so name changes have to be pushed this way.
    pub async fn rename_library_playlist(&self, playlist: &Value, name: &str) -> Result<()> {
        let mut update = playlist.clone();
        update["name"] = name.into();
        // Recomputed from the new name.
        update["sort_name"] = Value::Null;
        // Overwrite, or Music Assistant keeps the name it has. Everything else is sent back as is.
        let args = json!({ "item_id": playlist["item_id"], "update": update, "overwrite": true });
        self.raw_command("music/playlists/update", args)
            .await
            .map(|_| ())
    }

    /// Start a background playlist sync for one provider, picking up added and removed playlists.
    pub async fn sync_playlists(&self, provider_instance: &str) -> Result<()> {
        let args = json!({ "media_types": ["playlist"], "providers": [provider_instance] });
        self.raw_command("music/sync", args).await.map(|_| ())
    }
}

/// The ID `provider_instance` knows a library item by.
pub fn provider_item_id<'a>(item: &'a Value, provider_instance: &str) -> Option<&'a str> {
    item["provider_mappings"]
        .as_array()?
        .iter()
        .find(|m| m["provider_instance"] == provider_instance)?["item_id"]
        .as_str()
}

fn playlist_uri(provider_instance: &str, playlist_id: &str) -> String {
    format!("{provider_instance}://playlist/{playlist_id}")
}

/// `https://host` → `wss://host/ws`, `http://host:8095` → `ws://host:8095/ws`; no scheme means
/// plain HTTP.
fn websocket_url(server_url: &str) -> Result<String> {
    let url = server_url.trim();
    let (scheme, rest) = match url.split_once("://") {
        Some(("https" | "wss", rest)) => ("wss", rest),
        Some(("http" | "ws", rest)) => ("ws", rest),
        Some(_) => return Err(MaError::InvalidUrl(server_url.into())),
        None => ("ws", url),
    };
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix("/ws").unwrap_or(rest);
    if rest.is_empty() {
        return Err(MaError::InvalidUrl(server_url.into()));
    }
    Ok(format!("{scheme}://{rest}/ws"))
}

async fn read_loop(mut stream: futures_util::stream::SplitStream<Socket>, pending: Pending) {
    while let Some(Ok(message)) = stream.next().await {
        let Message::Text(text) = message else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        // Events have no message_id; we don't subscribe to anything, so ignore them.
        let Some(id) = value.get("message_id").and_then(|id| match id {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }) else {
            continue;
        };
        let mut pending = pending.lock().await;
        if value.get("partial").and_then(Value::as_bool) == Some(true) {
            if let (Some(command), Some(Value::Array(items))) =
                (pending.get_mut(&id), value.get("result"))
            {
                command.partial.extend(items.iter().cloned());
            }
            continue;
        }
        let Some(command) = pending.remove(&id) else {
            continue;
        };
        let result = if let Some(code) = value.get("error_code") {
            Err(MaError::Command {
                code: code.as_i64().unwrap_or(-1),
                details: value
                    .get("details")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error")
                    .to_string(),
            })
        } else {
            let result = value.get("result").cloned().unwrap_or(Value::Null);
            Ok(match (command.partial.is_empty(), result) {
                (true, result) => result,
                (false, Value::Array(rest)) => {
                    Value::Array(command.partial.into_iter().chain(rest).collect())
                }
                (false, _) => Value::Array(command.partial),
            })
        };
        let _ = command.reply.send(result);
    }
    // Connection gone: fail everything still waiting.
    for (_, command) in pending.lock().await.drain() {
        let _ = command.reply.send(Err(MaError::Closed));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_websocket_urls() {
        assert_eq!(
            websocket_url("https://music.example.com").unwrap(),
            "wss://music.example.com/ws"
        );
        assert_eq!(
            websocket_url("http://192.168.1.5:8095/").unwrap(),
            "ws://192.168.1.5:8095/ws"
        );
        assert_eq!(
            websocket_url("mass.local:8095").unwrap(),
            "ws://mass.local:8095/ws"
        );
        assert_eq!(websocket_url("ws://host/ws").unwrap(), "ws://host/ws");
        assert!(websocket_url("ftp://host").is_err());
        assert!(websocket_url("https://").is_err());
    }
}
