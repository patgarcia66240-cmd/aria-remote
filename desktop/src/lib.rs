//! ARIA Remote Desktop : le contrôleur autonome.
//!
//! Réglages (serveur + clé) et relais des appels vers le serveur de rendez-vous. La liste des appareils, l'appairage et la session
//! (écran distant, souris, clavier, WebRTC) tournent dans la fenêtre (web/), qui passe par `api_request`.
//! Le presse-papiers local et l'enregistrement des captures passent aussi par ici (la fenêtre n'a pas accès au système de fichiers).
mod client;
mod config;
mod files;

use client::Client;
use config::Config;
use serde::Serialize;
use serde_json::Value;

/// Même limite que l'agent : un texte plus gros n'est ni envoyé ni reçu.
const CLIPBOARD_MAX_BYTES: usize = 256 * 1024;

#[derive(Serialize)]
struct Settings {
    server: String,
    configured: bool,
}

#[tauri::command]
fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Ce que l'interface a le droit de savoir : l'adresse, et seulement si une clé est enregistrée (jamais la clé elle-même).
#[tauri::command]
fn load_settings() -> Settings {
    let config = config::load();
    Settings { configured: config.configured(), server: config.server }
}

/// Teste la connexion avec ces réglages, puis les enregistre. Une clé vide garde la clé déjà enregistrée.
#[tauri::command]
async fn save_settings(server: String, key: String) -> Result<Settings, String> {
    let current = config::load();
    let key = if key.trim().is_empty() { current.key } else { key.trim().to_string() };
    let server = client::normalize_server(&server)?;
    Client::new(&server, &key)?.status().await?;
    let config = Config { server, key };
    config::save(&config)?;
    Ok(Settings { configured: true, server: config.server })
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

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![app_version, load_settings, save_settings, api_request, clipboard_read, clipboard_write, save_capture])
        .run(tauri::generate_context!())
        .expect("échec du démarrage d'ARIA Remote Desktop");
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_the_cargo_version() {
        assert_eq!(super::app_version(), env!("CARGO_PKG_VERSION"));
    }
}
