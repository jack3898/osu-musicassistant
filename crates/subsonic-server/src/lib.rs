//! An OpenSubsonic server that exposes an osu!lazer library to Music Assistant (or any Subsonic
//! client): beatmap sets become albums, collections become playlists, and audio is streamed
//! straight from lazer's file store.

pub mod audio;
pub mod auth;
pub mod catalog;

mod api;

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::{Arc, RwLock, RwLockReadGuard};

use axum::Router;
use lazer_library::{DataDir, Library};

pub use audio::{AudioCache, AudioInfo};
pub use auth::Credentials;
pub use catalog::{Catalog, ViewOptions};

/// Shared server state. Cheap to clone; the library and settings can be swapped while serving.
#[derive(Clone)]
pub struct Server {
    inner: Arc<Inner>,
}

struct Inner {
    data_dir: DataDir,
    credentials: RwLock<Credentials>,
    catalog: RwLock<Arc<Catalog>>,
    audio: RwLock<AudioCache>,
}

impl Server {
    pub fn new(
        data_dir: DataDir,
        credentials: Credentials,
        library: Arc<Library>,
        view: &ViewOptions,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                data_dir,
                credentials: RwLock::new(credentials),
                catalog: RwLock::new(Arc::new(Catalog::new(library, view))),
                audio: RwLock::new(AudioCache::default()),
            }),
        }
    }

    /// The axum router serving the Subsonic API.
    pub fn router(&self) -> Router {
        Router::new().fallback(api::handle).with_state(self.clone())
    }

    pub fn data_dir(&self) -> &DataDir {
        &self.inner.data_dir
    }

    pub fn catalog(&self) -> Arc<Catalog> {
        self.inner.catalog.read().unwrap().clone()
    }

    /// Replace the library (after a re-index) or how it's presented (after a settings change).
    pub fn set_library(&self, library: Arc<Library>, view: &ViewOptions) {
        *self.inner.catalog.write().unwrap() = Arc::new(Catalog::new(library, view));
    }

    pub fn credentials(&self) -> Credentials {
        self.inner.credentials.read().unwrap().clone()
    }

    pub fn set_credentials(&self, credentials: Credentials) {
        *self.inner.credentials.write().unwrap() = credentials;
    }

    pub(crate) fn audio(&self) -> RwLockReadGuard<'_, AudioCache> {
        self.inner.audio.read().unwrap()
    }

    /// Probe every song in the library that isn't in the audio cache yet, loading and saving the
    /// cache at `cache_path` if given. Blocking; run it on a background thread. Returns how many
    /// files were newly probed.
    pub fn probe_audio(&self, cache_path: Option<PathBuf>) -> usize {
        if let Some(path) = &cache_path {
            let loaded = AudioCache::load(path);
            let mut audio = self.inner.audio.write().unwrap();
            if audio.is_empty() {
                *audio = loaded;
            }
        }
        let catalog = self.catalog();
        let pending: Vec<String> = {
            let audio = self.audio();
            catalog
                .library()
                .songs
                .iter()
                .map(|s| s.id.clone())
                .filter(|id| !audio.contains(id))
                .collect()
        };
        let mut probed = 0;
        for id in pending {
            let Some(path) = self.data_dir().file_path(&id) else {
                continue;
            };
            match AudioInfo::probe(&path) {
                Ok(info) => {
                    self.inner.audio.write().unwrap().insert(id, info);
                    probed += 1;
                }
                Err(err) => tracing::warn!(%id, %err, "couldn't probe audio file"),
            }
        }
        if let Some(path) = cache_path
            && probed > 0
            && let Err(err) = self.audio().save(&path)
        {
            tracing::warn!(%err, "couldn't save audio cache");
        }
        probed
    }
}

/// This machine's IPv4 addresses, best guess first, for telling the user what to enter in Music
/// Assistant. Home network ranges (192.168/16, 10/8, 172.16/12) beat VPN ones such as
/// Tailscale's 100.64/10.
pub fn lan_ips() -> Vec<Ipv4Addr> {
    let mut ips: Vec<Ipv4Addr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|iface| match iface.ip() {
            IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_link_local() => Some(ip),
            _ => None,
        })
        .collect();
    ips.sort_by_key(|ip| !ip.is_private());
    ips.dedup();
    ips
}
