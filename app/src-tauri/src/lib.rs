//! osu! Music Assistant: serves an osu!lazer library to Music Assistant over OpenSubsonic and
//! starts playlists on its speakers.

mod commands;
mod settings;
mod state;

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;
use tracing_subscriber::EnvFilter;

use state::AppState;

/// Passed by the login item so the app starts in the background.
const MINIMIZED_ARG: &str = "--minimized";

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,lofty=error".into()),
        )
        .init();

    tauri::Builder::default()
        // A second launch just brings the running app forward.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_window(app)
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![MINIMIZED_ARG]),
        ))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            let cache_dir = app.path().app_cache_dir()?;
            app.manage(AppState::new(
                config_dir.join("settings.json"),
                cache_dir.join("audio-cache.json"),
            ));

            build_tray(app.handle())?;

            let state = app.state::<AppState>();
            let started_minimized = std::env::args().any(|a| a == MINIMIZED_ARG);
            if started_minimized && state.settings().keep_in_tray {
                hide_window(app.handle());
            }

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                handle.state::<AppState>().start(&handle).await;
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                if app.state::<AppState>().settings().keep_in_tray {
                    api.prevent_close();
                    hide_window(app);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::get_settings,
            commands::save_settings,
            commands::regenerate_password,
            commands::set_ma_token,
            commands::reindex,
            commands::list_playlists,
            commands::test_music_assistant,
            commands::list_players,
            commands::play,
        ])
        .run(tauri::generate_context!())
        .expect("error while running osu! Music Assistant");
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open osu! Music Assistant", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    TrayIconBuilder::with_id("main")
        .icon(Image::from_bytes(include_bytes!("../icons/tray.png"))?)
        .icon_as_template(true)
        .tooltip("osu! Music Assistant")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

fn show_window(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Hide the window but keep serving; on macOS also drop the Dock icon so it lives in the menu bar.
fn hide_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
}
