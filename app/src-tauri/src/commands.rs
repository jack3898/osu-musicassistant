//! Commands the frontend calls via `invoke`.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;

use crate::settings::{self, Settings, generate_password, load_ma_token};
use crate::state::{AppState, MaInfo, PlayerInfo, PlaylistInfo, Status};

/// Settings as the frontend sees them: never the token itself, just whether one is saved.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    #[serde(flatten)]
    settings: Settings,
    has_ma_token: bool,
}

fn view(settings: Settings) -> SettingsView {
    SettingsView {
        settings,
        has_ma_token: load_ma_token().is_some(),
    }
}

#[tauri::command]
pub async fn get_status(state: State<'_, AppState>) -> Result<Status, String> {
    Ok(state.status().await)
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> SettingsView {
    view(state.settings())
}

/// The fields the settings form edits.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsUpdate {
    osu_data_dir: Option<String>,
    port: u16,
    hide_rate_edits: bool,
    blocked_patterns: Vec<String>,
    prefer_unicode: bool,
    ma_url: String,
    keep_in_tray: bool,
    launch_at_login: bool,
}

#[tauri::command]
pub async fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    update: SettingsUpdate,
) -> Result<SettingsView, String> {
    if update.port < 1024 {
        return Err("Pick a port between 1024 and 65535.".into());
    }
    let old = state.settings();
    let new = Settings {
        osu_data_dir: update
            .osu_data_dir
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty())
            .map(Into::into),
        port: update.port,
        hide_rate_edits: update.hide_rate_edits,
        blocked_patterns: update
            .blocked_patterns
            .into_iter()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect(),
        prefer_unicode: update.prefer_unicode,
        ma_url: update.ma_url.trim().to_string(),
        keep_in_tray: update.keep_in_tray,
        launch_at_login: update.launch_at_login,
        ..old.clone()
    };
    if new.launch_at_login != old.launch_at_login {
        let autostart = app.autolaunch();
        let result = if new.launch_at_login {
            autostart.enable()
        } else {
            autostart.disable()
        };
        result.map_err(|e| format!("couldn't change the login item: {e}"))?;
    }
    state.save_settings(new.clone())?;
    state.apply_settings(&app, &old).await;
    Ok(view(new))
}

#[tauri::command]
pub async fn regenerate_password(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SettingsView, String> {
    let old = state.settings();
    let new = Settings {
        password: generate_password(),
        ..old.clone()
    };
    state.save_settings(new.clone())?;
    state.apply_settings(&app, &old).await;
    Ok(view(new))
}

#[tauri::command]
pub async fn set_ma_token(
    state: State<'_, AppState>,
    token: String,
) -> Result<SettingsView, String> {
    settings::store_ma_token(&token)?;
    state.forget_ma_session().await;
    Ok(view(state.settings()))
}

#[tauri::command]
pub async fn reindex(app: AppHandle, state: State<'_, AppState>) -> Result<Status, String> {
    state.reload_library(&app).await;
    Ok(state.status().await)
}

#[tauri::command]
pub async fn list_playlists(state: State<'_, AppState>) -> Result<Vec<PlaylistInfo>, String> {
    Ok(state.playlists().await)
}

#[tauri::command]
pub async fn test_music_assistant(state: State<'_, AppState>) -> Result<MaInfo, String> {
    state.forget_ma_session().await;
    state.music_assistant_info().await
}

#[tauri::command]
pub async fn list_players(state: State<'_, AppState>) -> Result<Vec<PlayerInfo>, String> {
    state.players().await
}

#[tauri::command]
pub async fn play(
    state: State<'_, AppState>,
    player_id: String,
    playlist_id: String,
    shuffle: bool,
) -> Result<(), String> {
    state.play(&player_id, &playlist_id, shuffle).await
}
