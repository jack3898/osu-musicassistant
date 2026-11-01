//! User settings, persisted as JSON in the app's config directory. The Music Assistant token is
//! a secret, so it lives in the OS credential store (Keychain on macOS) instead.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use lazer_library::SongFilter;
use serde::{Deserialize, Serialize};
use subsonic_server::{Credentials, ViewOptions};

const KEYRING_SERVICE: &str = "com.jack3898.osu-musicassistant";
const KEYRING_TOKEN_USER: &str = "music-assistant-token";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// Overrides the auto-detected osu!lazer data directory.
    pub osu_data_dir: Option<PathBuf>,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub hide_rate_edits: bool,
    pub blocked_patterns: Vec<String>,
    pub prefer_unicode: bool,
    /// Prepended to collection names in Music Assistant, e.g. `osu! collection - Favourites`.
    pub collection_prefix: String,
    /// Music Assistant's address, e.g. `https://music.example.com`.
    pub ma_url: String,
    pub keep_in_tray: bool,
    pub launch_at_login: bool,
    pub last_player_id: Option<String>,
    pub last_playlist_id: Option<String>,
    pub shuffle: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            osu_data_dir: None,
            port: 4747,
            username: "osu".into(),
            password: generate_password(),
            hide_rate_edits: true,
            blocked_patterns: Vec::new(),
            prefer_unicode: false,
            collection_prefix: "osu! collection - ".into(),
            ma_url: String::new(),
            keep_in_tray: false,
            launch_at_login: false,
            last_player_id: None,
            last_playlist_id: None,
            shuffle: true,
        }
    }
}

impl Settings {
    /// Load settings, creating (and saving) defaults on first run.
    pub fn load_or_create(path: &Path) -> Self {
        match fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        {
            Some(settings) => settings,
            None => {
                let settings = Self::default();
                if let Err(err) = settings.save(path) {
                    tracing::warn!(%err, "couldn't save initial settings");
                }
                settings
            }
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        fs::rename(tmp, path)
    }

    pub fn credentials(&self) -> Credentials {
        Credentials {
            username: self.username.clone(),
            password: self.password.clone(),
            // One secret is easier to manage; Music Assistant can use it either way.
            api_key: self.password.clone(),
        }
    }

    pub fn view_options(&self) -> ViewOptions {
        ViewOptions {
            filter: SongFilter {
                hide_rate_edits: self.hide_rate_edits,
                blocked_patterns: self.blocked_patterns.clone(),
            },
            prefer_unicode: self.prefer_unicode,
            collection_prefix: self.collection_prefix.clone(),
        }
    }
}

/// A password that's easy to type into Music Assistant: `xxxx-xxxx-xxxx-xxxx`.
pub fn generate_password() -> String {
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("the OS random number generator is unavailable");
    let chars: Vec<char> = bytes
        .iter()
        .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
        .collect();
    chars
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

fn token_entry() -> keyring::Result<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_TOKEN_USER)
}

pub fn load_ma_token() -> Option<String> {
    token_entry()
        .and_then(|e| e.get_password())
        .ok()
        .filter(|t| !t.is_empty())
}

/// Store the token, or remove it when `token` is empty.
pub fn store_ma_token(token: &str) -> Result<(), String> {
    let entry = token_entry().map_err(|e| e.to_string())?;
    let result = match token.trim() {
        "" => entry.delete_credential(),
        token => entry.set_password(token),
    };
    match result {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(format!(
            "couldn't save the token to the system keychain: {err}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_are_grouped_and_unambiguous() {
        let password = generate_password();
        assert_eq!(password.len(), 19);
        assert_eq!(password.matches('-').count(), 3);
        assert!(!password.contains(['0', 'o', '1', 'l', 'i']));
        assert_ne!(password, generate_password());
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"port": 5000}"#).unwrap();
        assert_eq!(settings.port, 5000);
        assert!(settings.hide_rate_edits);
        assert!(!settings.keep_in_tray);
    }
}
