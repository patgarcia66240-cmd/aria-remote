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
    };
    save(path, &config)?;
    Ok((config, identity))
}

fn save(path: &Path, config: &Config) -> Result<()> {
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

    #[test]
    fn names_are_cleaned_for_the_signed_registration() {
        assert_eq!(clean_name("Bu|reau\n"), "Bureau");
        assert_eq!(clean_name("  |  "), "PC");
        assert_eq!(clean_name(&"x".repeat(100)).len(), 60);
    }
}
