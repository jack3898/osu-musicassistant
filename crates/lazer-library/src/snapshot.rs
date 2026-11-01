//! Copying lazer's live database somewhere safe to read.
//!
//! We never open `client.realm` in place: a different realm-core build touching lazer's lock files
//! or file could corrupt the user's library. Instead we copy it and open the copy immutably. Lazer
//! may be writing while we copy, so the copy is only accepted if the source looks unchanged
//! across the whole copy.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime};

use tempfile::TempDir;

use crate::data_dir::DATABASE_FILENAME;

const ATTEMPTS: u32 = 5;
const RETRY_DELAY: Duration = Duration::from_millis(500);

/// A private copy of a Realm database, deleted on drop.
#[derive(Debug)]
pub struct Snapshot {
    dir: TempDir,
}

impl Snapshot {
    /// Copy `source` into a fresh temporary directory, retrying if it changes mid-copy.
    pub fn take(source: &Path) -> io::Result<Self> {
        let dir = tempfile::Builder::new().prefix("osu-musicassistant-").tempdir()?;
        let dest = dir.path().join(DATABASE_FILENAME);
        for attempt in 1..=ATTEMPTS {
            let before = fingerprint(source)?;
            fs::copy(source, &dest)?;
            if fingerprint(source)? == before {
                return Ok(Self { dir });
            }
            if attempt < ATTEMPTS {
                thread::sleep(RETRY_DELAY);
            }
        }
        Err(io::Error::other("osu! kept writing to its database while it was being copied; try again later"))
    }

    pub fn path(&self) -> PathBuf {
        self.dir.path().join(DATABASE_FILENAME)
    }
}

fn fingerprint(path: &Path) -> io::Result<(u64, SystemTime)> {
    let meta = fs::metadata(path)?;
    Ok((meta.len(), meta.modified()?))
}
