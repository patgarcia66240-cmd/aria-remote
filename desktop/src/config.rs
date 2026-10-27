//! Réglages mémorisés : adresse du serveur et clé du contrôleur, dans le dossier de configuration de l'utilisateur.
//! La clé n'est jamais renvoyée à l'interface : elle ne sert qu'aux appels faits par ce programme.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Config {
    #[serde(default)]
    pub server: String,
    #[serde(default)]
    pub key: String,
    /// Ce que fait ce PC : « control » (prendre le contrôle, par défaut), « host » (être contrôlé) ou « both ».
    #[serde(default = "default_mode")]
    pub mode: String,
    /// Démarrer avec Windows (réduit près de l'horloge quand ce PC peut être contrôlé).
    #[serde(default)]
    pub autostart: bool,
}

pub const MODES: [&str; 3] = ["control", "host", "both"];

fn default_mode() -> String {
    "control".into()
}

impl Default for Config {
    fn default() -> Self {
        Self { server: String::new(), key: String::new(), mode: default_mode(), autostart: false }
    }
}

impl Config {
    pub fn configured(&self) -> bool {
        !self.server.is_empty() && !self.key.is_empty()
    }

    /// Ce PC peut-il être contrôlé (l'agent tourne dans ce programme) ?
    pub fn hosts(&self) -> bool {
        self.mode == "host" || self.mode == "both"
    }
}

pub fn path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("aria-remote-desktop").join("config.json")
}

pub fn load_from(path: &std::path::Path) -> Config {
    std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

/// Écrit la configuration sans jamais laisser la clé dans un fichier lisible par d'autres : le contenu part dans un fichier
/// temporaire dont les droits sont restreints AVANT l'écriture, puis il remplace le fichier final. En cas d'échec, rien n'est laissé.
pub fn save_to(path: &std::path::Path, config: &Config) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Dossier de configuration impossible : {e}"))?;
    }
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    let _ = std::fs::remove_file(&tmp);
    let result = (|| {
        std::fs::File::create(&tmp).map_err(|e| format!("Enregistrement impossible : {e}"))?;
        restrict_to_current_user(&tmp)?;
        std::fs::write(&tmp, text).map_err(|e| format!("Enregistrement impossible : {e}"))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("Enregistrement impossible : {e}"))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Restreint le fichier à l'utilisateur courant : 0600 sous Unix ; sous Windows, retire l'héritage des droits
/// et ne donne l'accès qu'à `USERNAME`. Échoue si les droits ne peuvent pas être restreints.
fn restrict_to_current_user(path: &std::path::Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("Droits du fichier de configuration impossibles : {e}"))
    }
    #[cfg(windows)]
    {
        let user = std::env::var("USERNAME").map_err(|_| "Droits du fichier de configuration : utilisateur inconnu.".to_string())?;
        let status = std::process::Command::new("icacls")
            .arg(path)
            .args(["/inheritance:r", "/grant:r"])
            .arg(format!("{user}:F"))
            .status()
            .map_err(|e| format!("Droits du fichier de configuration impossibles : {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err("Droits du fichier de configuration impossibles à restreindre.".into())
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Ok(())
    }
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
        let config = Config { server: "https://rv.exemple.fr".into(), key: "k".into(), mode: "both".into(), autostart: true };
        save_to(&file, &config).unwrap();
        assert_eq!(load_from(&file), config);
        assert!(config.configured() && config.hosts());
        assert!(!Config::default().hosts(), "par défaut : contrôler seulement (comme avant)");
        assert_eq!(serde_json::from_str::<Config>(r#"{"server":"s","key":"k"}"#).unwrap().mode, "control", "une ancienne configuration reste en mode contrôle");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn saving_replaces_the_file_and_leaves_no_temporary_copy_behind() {
        let dir = std::env::temp_dir().join(format!("aria-remote-desktop-atomic-{}", std::process::id()));
        let file = dir.join("config.json");
        save_to(&file, &Config { server: "a".into(), key: "k1".into(), ..Config::default() }).unwrap();
        save_to(&file, &Config { server: "b".into(), key: "k2".into(), ..Config::default() }).unwrap();
        assert_eq!(load_from(&file).key, "k2", "la nouvelle configuration remplace l'ancienne");
        assert!(!dir.join("config.json.tmp").exists(), "aucune copie temporaire ne reste");
        std::fs::remove_dir_all(dir).ok();
    }
}
