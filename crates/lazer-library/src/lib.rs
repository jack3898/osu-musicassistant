//! Read-only access to an osu!lazer install: locating its data directory, snapshotting its Realm
//! database and (soon) indexing songs and collections from it.

pub mod data_dir;
pub mod realm;
pub mod snapshot;

pub use data_dir::{DataDir, locate_data_dir};
pub use snapshot::Snapshot;
