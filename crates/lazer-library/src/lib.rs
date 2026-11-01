//! Read-only access to an osu!lazer install: locating its data directory, snapshotting its Realm
//! database and indexing songs and collections from it.

pub mod data_dir;
pub mod index;
pub mod realm;
pub mod snapshot;

pub use data_dir::{DataDir, locate_data_dir};
pub use index::{Collection, Library, LibraryError, Song};
pub use snapshot::Snapshot;
