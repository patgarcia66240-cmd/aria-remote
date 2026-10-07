//! ARIA Remote Desktop : le contrôleur autonome.
//!
//! Réglages (serveur + clé) et relais des appels vers le serveur de rendez-vous. La liste des appareils, l'appairage et la session
//! (écran distant, souris, clavier, WebRTC) tournent dans la fenêtre (web/), qui passe par `api_request`.
mod client;
mod config;

use client::Client;
use config::Config;
use serde::Serialize;
use serde_json::Value;

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

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![app_version, load_settings, save_settings, api_request])
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
