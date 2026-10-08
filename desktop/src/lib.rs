//! ARIA Remote Desktop : le contrôleur autonome.
//!
//! Réglages (serveur + clé) et relais des appels vers le serveur de rendez-vous. La liste des appareils, l'appairage et la session
//! (écran distant, souris, clavier, WebRTC) tournent dans la fenêtre (web/), qui passe par `api_request`.
//! Le presse-papiers local et l'enregistrement des captures passent aussi par ici (la fenêtre n'a pas accès au système de fichiers).
mod autostart;
mod client;
mod config;
mod files;
mod host;

use client::Client;
use config::Config;
use serde::Serialize;
use serde_json::Value;
use tauri::Manager;

/// Même limite que l'agent : un texte plus gros n'est ni envoyé ni reçu.
const CLIPBOARD_MAX_BYTES: usize = 256 * 1024;

#[derive(Serialize)]
struct Settings {
    server: String,
    configured: bool,
    mode: String,
    autostart: bool,
}

impl From<Config> for Settings {
    fn from(config: Config) -> Self {
        Self { configured: config.configured(), server: config.server, mode: config.mode, autostart: config.autostart }
    }
}

#[tauri::command]
fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Ce que l'interface a le droit de savoir : l'adresse, et seulement si une clé est enregistrée (jamais la clé elle-même).
#[tauri::command]
fn load_settings() -> Settings {
    config::load().into()
}

/// Teste la connexion avec ces réglages, puis les enregistre. Une clé vide garde la clé déjà enregistrée.
#[tauri::command]
async fn save_settings(server: String, key: String) -> Result<Settings, String> {
    let current = config::load();
    let key = if key.trim().is_empty() { current.key } else { key.trim().to_string() };
    let server = client::normalize_server(&server)?;
    Client::new(&server, &key)?.status().await?;
    let config = Config { server, key, ..current };
    config::save(&config)?;
    Ok(config.into())
}

/// Ce que fait ce PC : contrôler, être contrôlé, ou les deux. L'agent démarre ou s'arrête en conséquence.
#[tauri::command]
async fn set_mode(mode: String, host: tauri::State<'_, host::Host>) -> Result<Settings, String> {
    if !config::MODES.contains(&mode.as_str()) {
        return Err("Mode inconnu.".into());
    }
    let config = Config { mode, ..config::load() };
    config::save(&config)?;
    if config.hosts() { host.start().await? } else { host.stop().await }
    Ok(config.into())
}

#[tauri::command]
fn set_autostart(enabled: bool) -> Result<Settings, String> {
    autostart::set(enabled)?;
    let config = Config { autostart: enabled, ..config::load() };
    config::save(&config)?;
    Ok(config.into())
}

fn saved_client() -> Result<Client, String> {
    let config = config::load();
    if !config.configured() {
        return Err("Renseigne d'abord le serveur et la clé dans les réglages.".into());
    }
    Client::new(&config.server, &config.key)
}

/// Relais des appels du contrôleur vers le serveur de rendez-vous : la clé reste ici, la fenêtre ne la voit jamais.
#[tauri::command]
async fn api_request(method: String, path: String, body: Option<Value>) -> Result<Value, String> {
    saved_client()?.request(&method, &path, body).await
}

/// Texte du presse-papiers local (None s'il n'y en a pas ou s'il est trop gros pour être partagé).
#[tauri::command]
fn clipboard_read() -> Option<String> {
    let text = arboard::Clipboard::new().ok()?.get_text().ok()?;
    (text.len() <= CLIPBOARD_MAX_BYTES).then_some(text)
}

/// Met du texte dans le presse-papiers local (texte venu de l'appareil contrôlé).
#[tauri::command]
fn clipboard_write(text: String) -> Result<(), String> {
    if text.len() > CLIPBOARD_MAX_BYTES {
        return Err("Texte trop gros pour le presse-papiers.".into());
    }
    arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text)).map_err(|_| "Presse-papiers indisponible.".to_string())
}

/// Enregistre une capture d'écran (PNG en base64) dans le dossier Images ; renvoie le chemin du fichier. Le nom est fabriqué ici, jamais choisi par la fenêtre.
#[tauri::command]
fn save_capture(png_base64: String, device: String) -> Result<String, String> {
    let bytes = files::decode_png(&png_base64)?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let path = files::save_png(&files::captures_dir(), &files::capture_filename(&device, now), &bytes)?;
    Ok(path.display().to_string())
}

/// Remet la fenêtre au premier plan (réduite, cachée près de l'horloge, ou derrière d'autres fenêtres).
fn show_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Icône près de l'horloge : état de l'appareil, ouvrir, couper la session, quitter. Le texte suit l'état de l'agent (`watch`).
fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    let status = MenuItem::with_id(app, "status", "ARIA Remote", false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Ouvrir ARIA Remote", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "Couper la session en cours", false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&status, &open, &stop, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main").tooltip("ARIA Remote").menu(&menu).show_menu_on_left_click(false);
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.on_menu_event(|app, event| match event.id.as_ref() {
        "open" => show_main(app),
        "stop" => {
            app.state::<host::Host>().send(remote_agent::UiCommand::StopSession);
        }
        "quit" => app.exit(0),
        _ => {}
    })
    .on_tray_icon_event(|tray, event| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
            show_main(tray.app_handle());
        }
    })
    .build(app)?;
    watch(app.handle().clone(), status, stop);
    Ok(())
}

/// Texte de l'icône d'après l'état de l'agent, et si une session est en cours.
fn tray_text(state: &Option<Value>) -> (String, bool) {
    let Some(state) = state else { return ("ARIA Remote".to_string(), false) };
    let name = state["device_name"].as_str().unwrap_or("Cet appareil");
    if state["session"].is_object() {
        (format!("{name} : contrôlé à distance"), true)
    } else if state["consent"].is_object() {
        (format!("{name} : demande de contrôle"), false)
    } else if state["link"] == "online" {
        (format!("{name} : disponible"), false)
    } else {
        (format!("{name} : connexion…"), false)
    }
}

/// Surveille l'agent : met à jour l'icône, et remonte la fenêtre dès qu'une demande de contrôle attend l'accord (jamais d'accord donné dans le dos).
fn watch(app: tauri::AppHandle, status: tauri::menu::MenuItem<tauri::Wry>, stop: tauri::menu::MenuItem<tauri::Wry>) {
    std::thread::spawn(move || {
        let mut asking: Option<String> = None;
        let mut last = String::new();
        loop {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let state = app.state::<host::Host>().state();
            let (text, in_session) = tray_text(&state);
            if text != last {
                let _ = status.set_text(&text);
                let _ = stop.set_enabled(in_session);
                if let Some(tray) = app.tray_by_id("main") {
                    let _ = tray.set_tooltip(Some(&text));
                }
                last = text;
            }
            let consent = state.as_ref().and_then(|s| s["consent"]["session_id"].as_str()).map(String::from);
            if consent.is_some() && consent != asking {
                show_main(&app);
            }
            asking = consent;
        }
    });
}

pub fn run() {
    let app = tauri::Builder::default()
        .manage(host::Host::default())
        .invoke_handler(tauri::generate_handler![
            app_version, load_settings, save_settings, set_mode, set_autostart, api_request, clipboard_read, clipboard_write, save_capture,
            host::agent_start, host::agent_stop, host::agent_state, host::agent_command
        ])
        .setup(|app| {
            build_tray(app)?;
            // Démarré avec Windows : près de l'horloge, sans fenêtre (elle remonte seule si quelqu'un demande le contrôle).
            if autostart::started_minimized() && config::load().hosts() {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Quand ce PC peut être contrôlé, fermer la fenêtre la range près de l'horloge : l'appareil reste disponible (« Quitter » est dans le menu de l'icône).
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.app_handle().state::<host::Host>().running() {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("échec du démarrage d'ARIA Remote Desktop");
    app.run(|app, event| {
        if let tauri::RunEvent::Exit = event {
            app.state::<host::Host>().stop_now();
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_the_cargo_version() {
        assert_eq!(super::app_version(), env!("CARGO_PKG_VERSION"));
    }
}
