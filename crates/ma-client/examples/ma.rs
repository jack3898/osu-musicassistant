//! Try the Music Assistant client from the command line.
//!
//! Usage (MA_URL and MA_TOKEN in the environment):
//!   cargo run -p ma-client --example ma -- players
//!   cargo run -p ma-client --example ma -- play <player id> <playlist id> [--shuffle]

use ma_client::MaClient;

/// Must match subsonic-server's "All songs" playlist.
const ALL_SONGS_ID: &str = "pl-all";
const ALL_SONGS_NAME: &str = "osu! — All songs";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("MA_URL")?;
    let token = std::env::var("MA_TOKEN")?;
    let args: Vec<String> = std::env::args().skip(1).collect();

    let client = MaClient::connect(&url, &token).await?;
    let info = client.server_info();
    println!(
        "connected to {} (Music Assistant {})",
        info.name.as_deref().unwrap_or("?"),
        info.server_version
    );

    let provider = client
        .find_subsonic_provider(ALL_SONGS_ID, ALL_SONGS_NAME)
        .await?;
    match &provider {
        Some(p) => println!("osu! provider: {} ({})", p.name, p.instance_id),
        None => println!(
            "osu! provider: not found — add the OpenSubsonic provider in Music Assistant first"
        ),
    }

    match args.first().map(String::as_str) {
        Some("play") => {
            let (Some(player), Some(playlist)) = (args.get(1), args.get(2)) else {
                return Err("usage: play <player id> <playlist id> [--shuffle]".into());
            };
            let provider = provider.ok_or("no osu! provider")?;
            let shuffle = args.iter().any(|a| a == "--shuffle");
            client
                .play_playlist(player, &provider.instance_id, playlist, shuffle)
                .await?;
            println!(
                "playing {playlist} on {player} ({})",
                if shuffle { "shuffled" } else { "in order" }
            );
        }
        _ => {
            println!("\nplayers:");
            for p in client.players().await? {
                let state = p.playback_state.as_deref().unwrap_or("?");
                println!(
                    "  {:<32} {:<10} {:<12} {}",
                    p.label(),
                    p.player_type,
                    state,
                    p.player_id
                );
            }
        }
    }
    Ok(())
}
