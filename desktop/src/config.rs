//! Réglages mémorisés : adresse du serveur et clé du contrôleur, dans le dossier de configuration de l'utilisateur.
//! La clé n'est jamais renvoyée à l'interface : elle ne sert qu'aux appels faits par ce programme.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct Config {
    #[serde(default)]
    pub server: String,
    #[serde(default)]
    pub key: String,
}

impl Config {
    pub fn configured(&self) -> bool {
        !self.server.is_empty() && !self.key.is_empty()
    }
}

pub fn path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("aria-remote-desktop").join("config.json")
}

pub fn load_from(path: &std::path::Path) -> Config {
    std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

pub fn save_to(path: &std::path::Path, config: &Config) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Dossier de configuration impossible : {e}"))?;
    }
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| format!("Enregistrement impossible : {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub fn load() -> Config {
    load_from(&path())
}

pub fn save(config: &Config) -> Result<(), String> {
    save_to(&path(), config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_a_missing_file_gives_defaults() {
        let dir = std::env::temp_dir().join(format!("aria-remote-desktop-test-{}", std::process::id()));
        let file = dir.join("config.json");
        assert_eq!(load_from(&file), Config::default());
        assert!(!Config::default().configured());
        let config = Config { server: "https://rv.exemple.fr".into(), key: "k".into() };
        save_to(&file, &config).unwrap();
        assert_eq!(load_from(&file), config);
        assert!(config.configured());
        std::fs::remove_dir_all(dir).ok();
    }
}
