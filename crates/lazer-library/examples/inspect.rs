//! Snapshot the local osu!lazer database and print an overview of what's in it.
//!
//! Usage: cargo run -p lazer-library --example inspect [-- <osu data dir>]

use lazer_library::realm::Realm;
use lazer_library::{DataDir, Snapshot, locate_data_dir};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = match std::env::args().nth(1) {
        Some(path) => DataDir::new(&path).ok_or(format!("no client.realm in {path}"))?,
        None => locate_data_dir().ok_or("couldn't find an osu!lazer data directory")?,
    };
    println!("data dir:       {}", data_dir.root().display());

    let snapshot = Snapshot::take(&data_dir.database())?;
    let realm = Realm::open_immutable(&snapshot.path())?;
    println!("schema version: {}", realm.schema_version());

    let mut names = realm.class_names()?;
    names.sort();
    for name in names {
        let class = realm.class(&name)?;
        println!("  {:<28} {:>7} objects", name, realm.all(&class)?.len()?);
    }
    Ok(())
}
