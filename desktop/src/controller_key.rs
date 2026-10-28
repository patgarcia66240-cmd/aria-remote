//! Clé du contrôleur (phase 4 de docs/SIGNATURE_CONTROLEUR.md) : ce bureau signe ses offres WebRTC pour que l'agent vérifie qui contrôle.
//! La clé privée reste dans le dossier de configuration de l'application ; elle n'est jamais envoyée à la page ni au serveur.
//! Réutilise l'identité et les messages de l'agent (crate remote-agent) : mêmes octets, donc la vérification côté agent accepte ces signatures.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use std::collections::HashMap;

use base64::Engine;
use remote_agent::identity::{answer_is_signed, controller_offer_message, fingerprint, Identity};
use serde::Serialize;

/// Évite deux clés créées en même temps si deux commandes arrivent ensemble.
static CREATE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Serialize, Debug, PartialEq)]
pub struct ControllerIdentity {
    pub public_key: String,
    pub fingerprint: String,
}

pub fn path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("aria-remote-desktop").join("controller.seed")
}

/// Charge la clé du contrôleur, ou la crée au premier usage.
pub fn load_or_create_at(path: &Path) -> Result<Identity, String> {
    let _guard = CREATE_LOCK.lock().map_err(|_| "verrou de la clé indisponible".to_string())?;
    match std::fs::read_to_string(path) {
        Ok(seed) => return Identity::from_seed(seed.trim()).map_err(|error| format!("clé du contrôleur illisible : {error}")),
        // Seule l'absence de fichier autorise une nouvelle clé : une erreur de lecture ne doit jamais écraser la clé existante.
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(format!("lecture de la clé du contrôleur : {error}")),
        Err(_) => {}
    }
    let identity = Identity::generate();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    }
    std::fs::write(path, identity.seed()).map_err(|error| format!("écriture de la clé du contrôleur : {error}"))?;
    Ok(identity)
}

pub fn identity_info(path: &Path) -> Result<ControllerIdentity, String> {
    let identity = load_or_create_at(path)?;
    let public_key = identity.public_key();
    let fingerprint = fingerprint(&public_key).map_err(|error| error.to_string())?;
    Ok(ControllerIdentity { public_key, fingerprint })
}

/// Signature de l'offre : liée à la session et au contenu exact du SDP (voir controller_offer_message côté agent).
pub fn sign_offer(path: &Path, session_id: &str, sdp: &str) -> Result<String, String> {
    let identity = load_or_create_at(path)?;
    Ok(identity.sign(&controller_offer_message(session_id, sdp)))
}

/// Clés publiques des appareils appairés, épinglées à l'appairage (point 2 de l'audit) : « appareil -> clé ».
pub fn pins_path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("aria-remote-desktop").join("pinned-devices.json")
}

fn read_pins(path: &Path) -> Result<HashMap<String, String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|error| format!("registre des appareils illisible : {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
        // Une erreur de lecture ne doit jamais effacer les clés épinglées : on refuse plutôt que de réécrire un registre vide.
        Err(error) => Err(format!("lecture du registre des appareils : {error}")),
    }
}

/// Épingle la clé publique d'un appareil (32 octets en base64) et renvoie son empreinte à comparer avec la fenêtre de l'agent.
pub fn pin_at(path: &Path, device_id: &str, public_key: &str) -> Result<String, String> {
    let raw = base64::engine::general_purpose::STANDARD.decode(public_key.trim()).map_err(|_| "clé de l'appareil illisible".to_string())?;
    if raw.len() != 32 {
        return Err("clé de l'appareil invalide (32 octets attendus)".to_string());
    }
    let mut pins = read_pins(path)?;
    pins.insert(device_id.to_string(), public_key.trim().to_string());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    }
    std::fs::write(path, serde_json::to_string_pretty(&pins).map_err(|error| error.to_string())?)
        .map_err(|error| format!("écriture du registre des appareils : {error}"))?;
    fingerprint(public_key.trim()).map_err(|error| error.to_string())
}

/// Vraie seulement si la réponse est signée par la clé ÉPINGLÉE de cet appareil. Pas de clé épinglée = refus.
pub fn verify_answer_at(path: &Path, device_id: &str, session_id: &str, offer: &str, answer: &str, signature: &str) -> Result<bool, String> {
    match read_pins(path)?.get(device_id) {
        Some(key) => Ok(answer_is_signed(key, session_id, offer, answer, signature)),
        None => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use remote_agent::identity::controller_offer_is_signed;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aria-remote-ctl-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("controller.seed")
    }

    #[test]
    fn the_key_is_created_once_and_then_kept() {
        let path = scratch("kept");
        let first = identity_info(&path).unwrap();
        let second = identity_info(&path).unwrap();
        assert_eq!(first, second, "la même clé doit revenir au lancement suivant");
        assert_eq!(first.fingerprint.len(), 19);
    }

    #[test]
    fn an_offer_signed_here_is_accepted_by_the_agent_and_no_other_offer_is() {
        let path = scratch("signed");
        let info = identity_info(&path).unwrap();
        let sdp = "v=0 offre du bureau";
        let signature = sign_offer(&path, "sess_9", sdp).unwrap();
        assert!(controller_offer_is_signed(&info.public_key, "sess_9", sdp, &signature));
        assert!(!controller_offer_is_signed(&info.public_key, "sess_9", "v=0 autre", &signature), "offre modifiée");
        assert!(!controller_offer_is_signed(&info.public_key, "sess_10", sdp, &signature), "rejouée sur une autre session");
    }

    #[test]
    fn an_answer_is_accepted_only_from_the_device_pinned_at_pairing() {
        let dir = std::env::temp_dir().join(format!("aria-remote-pins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("pinned-devices.json");
        let device = Identity::generate();
        let intruder = Identity::generate();
        let (offer, answer) = ("v=0 offre", "v=0 reponse");
        let signature = device.sign(&remote_agent::identity::answer_message("sess_1", offer, answer)).to_string();

        assert_eq!(verify_answer_at(&path, "dev_1", "sess_1", offer, answer, &signature), Ok(false), "rien d'épinglé : refus");
        let shown = pin_at(&path, "dev_1", &device.public_key()).unwrap();
        assert_eq!(shown, fingerprint(&device.public_key()).unwrap(), "l'empreinte épinglée est celle de l'agent");
        assert_eq!(verify_answer_at(&path, "dev_1", "sess_1", offer, answer, &signature), Ok(true));
        assert_eq!(verify_answer_at(&path, "dev_1", "sess_1", offer, "v=0 modifiee", &signature), Ok(false), "réponse modifiée");
        assert_eq!(verify_answer_at(&path, "dev_2", "sess_1", offer, answer, &signature), Ok(false), "autre appareil");

        pin_at(&path, "dev_1", &intruder.public_key()).unwrap();
        assert_eq!(verify_answer_at(&path, "dev_1", "sess_1", offer, answer, &signature), Ok(false), "un nouvel appairage remplace la clé");
    }

    #[test]
    fn a_key_of_the_wrong_size_is_refused_and_an_unreadable_registry_is_never_overwritten() {
        let dir = std::env::temp_dir().join(format!("aria-remote-pins-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("pinned-devices.json");
        assert!(pin_at(&path, "dev_1", "AAAA").is_err(), "trop court");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "{ pas du json").unwrap();
        assert!(verify_answer_at(&path, "dev_1", "s", "o", "a", "x").is_err(), "registre illisible : erreur");
        assert!(pin_at(&path, "dev_1", &Identity::generate().public_key()).is_err(), "le registre illisible n'est pas écrasé");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ pas du json");
    }
}
