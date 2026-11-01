//! Serve the local osu!lazer library over OpenSubsonic, for trying it out with Music Assistant
//! before the desktop app exists.
//!
//! Usage: OSU_MA_PASSWORD=secret cargo run --release -p subsonic-server --example serve [-- <port>]

use std::sync::Arc;
use std::time::Instant;

use lazer_library::{Library, locate_data_dir};
use subsonic_server::{Credentials, Server, ViewOptions, lan_ips};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,lofty=error".into()),
        )
        .init();

    let port: u16 = std::env::args()
        .nth(1)
        .map(|p| p.parse())
        .transpose()?
        .unwrap_or(4747);
    let password = std::env::var("OSU_MA_PASSWORD").unwrap_or_else(|_| "osu".into());
    let credentials = Credentials {
        username: "osu".into(),
        password: password.clone(),
        api_key: password,
    };

    let data_dir = locate_data_dir().ok_or("couldn't find an osu!lazer data directory")?;
    let library = Arc::new(
        tokio::task::spawn_blocking({
            let data_dir = data_dir.clone();
            move || Library::load(&data_dir)
        })
        .await??,
    );
    let server = Server::new(data_dir, credentials, library, &ViewOptions::default());
    let catalog = server.catalog();
    tracing::info!(
        songs = catalog.songs.len(),
        albums = catalog.albums.len(),
        artists = catalog.artists.len(),
        playlists = catalog.playlists.len(),
        "library loaded"
    );

    let cache = dirs::cache_dir().map(|d| d.join("osu-musicassistant").join("audio-cache.json"));
    let prober = server.clone();
    tokio::task::spawn_blocking(move || {
        let started = Instant::now();
        let probed = prober.probe_audio(cache);
        tracing::info!(probed, elapsed = ?started.elapsed(), "audio probing finished");
    });

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    let ips = lan_ips();
    let host = ips
        .first()
        .map_or("<this computer's IP>".to_string(), |ip| ip.to_string());
    println!("\nAdd an OpenSubsonic provider in Music Assistant with:");
    println!("  Base URL: http://{host}");
    if ips.len() > 1 {
        let others: Vec<String> = ips[1..].iter().map(|ip| format!("http://{ip}")).collect();
        println!(
            "            (or {}, depending on how Music Assistant reaches this computer)",
            others.join(", ")
        );
    }
    println!("  Port:     {port}");
    println!("  Path:     (leave empty)");
    println!("  Username: osu");
    println!("  Password: (OSU_MA_PASSWORD, default \"osu\")\n");
    axum::serve(listener, server.router()).await?;
    Ok(())
}
