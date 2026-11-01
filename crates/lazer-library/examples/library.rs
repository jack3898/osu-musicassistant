//! Index the local osu!lazer library and print a summary of songs and collections.
//!
//! Usage: cargo run --release -p lazer-library --example library [-- <osu data dir>]

use std::collections::BTreeMap;
use std::time::Instant;

use lazer_library::{DataDir, Library, SongFilter, locate_data_dir};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = match std::env::args().nth(1) {
        Some(path) => DataDir::new(&path).ok_or(format!("no client.realm in {path}"))?,
        None => locate_data_dir().ok_or("couldn't find an osu!lazer data directory")?,
    };
    let started = Instant::now();
    let library = Library::load(&data_dir)?;
    println!(
        "indexed {} in {:.2?} (schema {})",
        data_dir.root().display(),
        started.elapsed(),
        library.schema_version
    );

    let count = |f: fn(&&lazer_library::Song) -> bool| library.songs.iter().filter(f).count();
    println!(
        "songs: {} total, {} in \"All songs\" ({} practice rate edits, {} bundled with lazer)",
        library.songs.len(),
        library.all_songs(&SongFilter::default()).count(),
        count(|s| s.rate_edit),
        count(|s| s.protected),
    );

    let mut missing_audio = 0;
    let mut extensions: BTreeMap<String, usize> = BTreeMap::new();
    for song in &library.songs {
        if !data_dir.file_path(&song.id).is_some_and(|p| p.is_file()) {
            missing_audio += 1;
        }
        let ext = song
            .audio_filename
            .rsplit_once('.')
            .map_or("<none>", |(_, e)| e)
            .to_lowercase();
        *extensions.entry(ext).or_default() += 1;
    }
    println!("audio files missing on disk: {missing_audio}");
    println!("audio extensions: {extensions:?}");
    println!(
        "songs without a background: {}",
        library
            .songs
            .iter()
            .filter(|s| s.background_id.is_none())
            .count()
    );

    let mut per_set: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for song in &library.songs {
        per_set
            .entry(&song.set_id)
            .or_default()
            .push(&song.audio_filename);
    }
    let multi: Vec<_> = per_set.values().filter(|files| files.len() > 1).collect();
    println!("beatmap sets with several audio files: {}", multi.len());
    for files in multi.iter().take(8) {
        println!("  {files:?}");
    }

    println!("\ncollections ({}):", library.collections.len());
    for c in &library.collections {
        let missing = if c.missing > 0 {
            format!("  ({} beatmaps not installed)", c.missing)
        } else {
            String::new()
        };
        println!("  {:<40} {:>5} songs{missing}", c.name, c.song_ids.len());
    }

    println!("\nfirst and last imported songs:");
    let n = library.songs.len();
    for song in library
        .songs
        .iter()
        .take(5)
        .chain(library.songs.iter().skip(n.saturating_sub(5)))
    {
        println!(
            "  {} - {}  [{}]  mapped by {}",
            song.artist, song.title, song.audio_filename, song.creator
        );
    }
    Ok(())
}
