//! Finding osu!lazer's data directory.

use std::fs;
use std::path::{Path, PathBuf};

pub const DATABASE_FILENAME: &str = "client.realm";

/// A located osu!lazer data directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDir {
    root: PathBuf,
}

impl DataDir {
    /// Use `root` as the data directory if it contains a lazer database.
    pub fn new(root: impl Into<PathBuf>) -> Option<Self> {
        let root = root.into();
        root.join(DATABASE_FILENAME)
            .is_file()
            .then_some(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn database(&self) -> PathBuf {
        self.root.join(DATABASE_FILENAME)
    }

    /// Where lazer stores the file with this SHA-256 hash: `files/a/ab/abcdef…`.
    pub fn file_path(&self, hash: &str) -> Option<PathBuf> {
        let valid = hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit());
        valid.then(|| {
            self.root
                .join("files")
                .join(&hash[..1])
                .join(&hash[..2])
                .join(hash)
        })
    }
}

/// The default data directory for this platform, before any `storage.ini` redirect.
pub fn default_data_dir() -> Option<PathBuf> {
    if cfg!(target_os = "linux") {
        // Lazer follows XDG; the Flatpak build keeps its data inside the sandbox.
        let native = dirs::data_dir().map(|d| d.join("osu"));
        let flatpak = dirs::home_dir().map(|h| h.join(".var/app/sh.ppy.osu/data/osu"));
        return native
            .filter(|p| p.is_dir())
            .or(flatpak.filter(|p| p.is_dir()));
    }
    // %APPDATA%\osu on Windows, ~/Library/Application Support/osu on macOS.
    dirs::data_dir().map(|d| d.join("osu"))
}

/// Locate lazer's data directory, honouring a custom location configured in `storage.ini`.
pub fn locate_data_dir() -> Option<DataDir> {
    let default = default_data_dir()?;
    if let Some(custom) = custom_storage_path(&default)
        && let Some(dir) = DataDir::new(custom)
    {
        return Some(dir);
    }
    DataDir::new(default)
}

/// Lazer writes `FullPath = <dir>` to `storage.ini` in the default directory when the user moves
/// their data elsewhere.
fn custom_storage_path(default_dir: &Path) -> Option<PathBuf> {
    let ini = fs::read_to_string(default_dir.join("storage.ini")).ok()?;
    parse_full_path(&ini)
}

fn parse_full_path(ini: &str) -> Option<PathBuf> {
    ini.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        let value = value.trim();
        (key.trim() == "FullPath" && !value.is_empty()).then(|| PathBuf::from(value))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_storage_ini() {
        assert_eq!(
            parse_full_path("FullPath = /Volumes/Games/osu\n"),
            Some("/Volumes/Games/osu".into())
        );
        assert_eq!(parse_full_path("FullPath = \n"), None);
        assert_eq!(parse_full_path("Other = x"), None);
    }

    #[test]
    fn file_path_uses_hash_prefixes() {
        let dir = DataDir {
            root: "/osu".into(),
        };
        let hash = "ab".repeat(32);
        assert_eq!(
            dir.file_path(&hash),
            Some(PathBuf::from(format!("/osu/files/a/ab/{hash}")))
        );
        assert_eq!(dir.file_path("../../etc/passwd"), None);
    }
}
