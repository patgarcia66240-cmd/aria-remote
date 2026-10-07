//! ARIA Remote Desktop : le contrôleur autonome.
//!
//! Étapes faites : réglages (serveur + clé), liste des appareils, appairage par code. À venir : la session (écran distant, souris, clavier ;
//! voir README.md, « Feuille de route »).
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

#[tauri::command]
async fn list_devices() -> Result<Value, String> {
    saved_client()?.devices().await
}

#[tauri::command]
async fn pair_device(code: String) -> Result<Value, String> {
    saved_client()?.pair(&code).await
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![app_version, load_settings, save_settings, list_devices, pair_device])
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
