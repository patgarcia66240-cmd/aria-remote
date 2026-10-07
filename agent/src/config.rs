//! Configuration locale de l'agent : identité de l'appareil et adresse du serveur. Le fichier contient la CLÉ PRIVÉE : il reste sur l'appareil,
//! lisible par son seul propriétaire (0600 sous Unix ; sous Windows, le profil de l'utilisateur).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::identity::Identity;

#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
    pub device_id: String,
    pub name: String,
    pub seed: String,
    /// Adresse de PC Assistant (le PC qui CONTRÔLE), retenue après le premier lancement : sur le PC contrôlé, plus rien à retaper.
    #[serde(default)]
    pub server: Option<String>,
    /// Clé d'API de ce backend (API_AUTH_TOKEN), retenue avec l'adresse. Même fichier et mêmes protections que la clé privée.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Permissions accordées en plus de la vue (« mouse,keyboard »), réglées dans la fenêtre de l'agent et retenues.
    #[serde(default)]
    pub allow: Option<String>,
}

pub fn default_path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("pc-assistant-remote-agent").join("config.json")
}

pub fn hostname() -> String {
    std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "PC".to_string())
}

/// Nom affichable : le serveur le refuse s'il contient « | » ou un caractère de contrôle (le nom est signé).
pub fn clean_name(name: &str) -> String {
    let cleaned: String = name.chars().filter(|c| *c != '|' && !c.is_control()).take(60).collect();
    if cleaned.trim().is_empty() { "PC".to_string() } else { cleaned.trim().to_string() }
}

pub fn load_or_create(path: &Path, name: Option<&str>) -> Result<(Config, Identity)> {
    if let Ok(text) = std::fs::read_to_string(path) {
        let mut config: Config = serde_json::from_str(&text).with_context(|| format!("configuration illisible : {}", path.display()))?;
        if let Some(name) = name {
            config.name = clean_name(name);
        }
        let identity = Identity::from_seed(&config.seed)?;
        return Ok((config, identity));
    }
    let identity = Identity::generate();
    let mut random = [0u8; 8];
    rand::rngs::OsRng.fill_bytes(&mut random);
    let config = Config {
        device_id: format!("dev_{}", hex::encode(random)),
        name: clean_name(name.unwrap_or(&hostname())),
        seed: identity.seed(),
        server: None,
        api_key: None,
        allow: None,
    };
    save(path, &config)?;
    Ok((config, identity))
}

pub fn save(path: &Path, config: &Config) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(config)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub const DEFAULT_SERVER: &str = "http://127.0.0.1:8000";

/// Adresse normalisée : schéma ajouté si absent (http par défaut), pas de « / » final. None si vide ou si le schéma n'est pas http(s).
pub fn normalize_server(raw: &str) -> Option<String> {
    let raw = raw.trim().trim_end_matches('/');
    if raw.is_empty() {
        return None;
    }
    let with_scheme = if raw.contains("://") { raw.to_string() } else { format!("http://{raw}") };
    (with_scheme.starts_with("http://") || with_scheme.starts_with("https://")).then_some(with_scheme)
}

/// Adresse et clé à utiliser : ligne de commande > valeurs retenues > question posée à l'utilisateur (`ask`, None si pas de terminal) > défaut local.
/// Renvoie aussi `true` quand la configuration a changé et doit être sauvegardée : la prochaine fois, plus rien à saisir.
pub fn resolve_connection(cli_server: Option<String>, cli_key: Option<String>, config: &mut Config,
                          ask: &mut dyn FnMut() -> Option<(String, String)>) -> (String, String, bool) {
    let mut server = cli_server.as_deref().and_then(normalize_server);
    let mut key = cli_key;
    if server.is_none() && config.server.is_none() {
        if let Some((asked_server, asked_key)) = ask() {
            server = normalize_server(&asked_server);
            key = key.or(Some(asked_key.trim().to_string()));
        }
    }
    let mut changed = false;
    if let Some(server) = &server {
        if config.server.as_ref() != Some(server) { config.server = Some(server.clone()); changed = true; }
    }
    if let Some(key) = &key {
        if config.api_key.as_ref() != Some(key) { config.api_key = Some(key.clone()); changed = true; }
    }
    let server = server.or_else(|| config.server.clone()).unwrap_or_else(|| DEFAULT_SERVER.to_string());
    (server, config.api_key.clone().unwrap_or_default(), changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_identity_is_created_once_then_reloaded() {
        let dir = std::env::temp_dir().join(format!("remote-agent-test-{}", hex::encode(rand::random::<[u8; 6]>())));
        let path = dir.join("config.json");
        let (first, identity) = load_or_create(&path, Some("Bureau")).unwrap();
        let (second, again) = load_or_create(&path, None).unwrap();
        assert_eq!(first.device_id, second.device_id);
        assert_eq!(identity.public_key(), again.public_key());
        assert!(first.device_id.starts_with("dev_") && first.device_id.len() == 20);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        std::fs::remove_dir_all(dir).ok();
    }

    fn blank() -> Config {
        Config { device_id: "dev_0123456789abcdef".into(), name: "Salon".into(), seed: String::new(), server: None, api_key: None, allow: None }
    }

    #[test]
    fn server_addresses_are_normalized() {
        assert_eq!(normalize_server("192.168.1.20:8000").as_deref(), Some("http://192.168.1.20:8000"));
        assert_eq!(normalize_server("  https://aria.example/ ").as_deref(), Some("https://aria.example"));
        assert_eq!(normalize_server("ftp://x"), None);
        assert_eq!(normalize_server("   "), None);
    }

    #[test]
    fn the_command_line_wins_then_saved_values_then_the_question_then_the_local_default() {
        let never = &mut || -> Option<(String, String)> { panic!("ne doit pas poser de question") };
        let mut config = blank();
        let (server, key, changed) = resolve_connection(Some("192.168.1.20:8000".into()), Some("abc".into()), &mut config, never);
        assert_eq!((server.as_str(), key.as_str(), changed), ("http://192.168.1.20:8000", "abc", true));
        // Deuxième lancement sans arguments : tout est retenu, rien à sauvegarder, aucune question.
        let (server, key, changed) = resolve_connection(None, None, &mut config, never);
        assert_eq!((server.as_str(), key.as_str(), changed), ("http://192.168.1.20:8000", "abc", false));
        // Un argument différent remplace la valeur retenue.
        let (server, _, changed) = resolve_connection(Some("https://aria.example".into()), None, &mut config, never);
        assert_eq!((server.as_str(), changed), ("https://aria.example", true));
        assert_eq!(config.api_key.as_deref(), Some("abc"));
    }

    #[test]
    fn a_first_launch_without_arguments_asks_and_remembers_the_answer() {
        let mut config = blank();
        let mut asked = 0;
        let (server, key, changed) = resolve_connection(None, None, &mut config, &mut || { asked += 1; Some(("192.168.1.20".into(), " cle ".into())) });
        assert_eq!((server.as_str(), key.as_str(), changed, asked), ("http://192.168.1.20", "cle", true, 1));
        assert_eq!(config.server.as_deref(), Some("http://192.168.1.20"));
    }

    #[test]
    fn without_a_terminal_the_local_default_is_used_and_nothing_is_saved() {
        let mut config = blank();
        let (server, key, changed) = resolve_connection(None, None, &mut config, &mut || None);
        assert_eq!((server.as_str(), key.as_str(), changed), (DEFAULT_SERVER, "", false));
    }

    #[test]
    fn an_old_configuration_file_without_server_fields_still_loads() {
        let old = r#"{"device_id":"dev_0123456789abcdef","name":"Salon","seed":"AAAA"}"#;
        let config: Config = serde_json::from_str(old).unwrap();
        assert!(config.server.is_none() && config.api_key.is_none());
    }

    #[test]
    fn names_are_cleaned_for_the_signed_registration() {
        assert_eq!(clean_name("Bu|reau\n"), "Bureau");
        assert_eq!(clean_name("  |  "), "PC");
        assert_eq!(clean_name(&"x".repeat(100)).len(), 60);
    }
}
