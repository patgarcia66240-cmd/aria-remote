//! Clé du contrôleur (phase 4 de docs/SIGNATURE_CONTROLEUR.md) : ce bureau signe ses offres WebRTC pour que l'agent vérifie qui contrôle.
//! La clé privée reste dans le dossier de configuration de l'application ; elle n'est jamais envoyée à la page ni au serveur.
//! Réutilise l'identité et les messages de l'agent (crate remote-agent) : mêmes octets, donc la vérification côté agent accepte ces signatures.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use remote_agent::identity::{controller_offer_message, fingerprint, Identity};
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
}
