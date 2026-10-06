//! Identité de l'appareil : clé Ed25519 générée ici, dont seule la partie publique quitte l'appareil. Les messages signés sont les MÊMES octets
//! que ceux du backend (backend/plugins/remote/identity.py) : champs joints par « | », préfixés par la version et le type.

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};

pub const PREFIX: &str = "pc-assistant-remote-v1";

pub struct Identity {
    key: SigningKey,
}

impl Identity {
    pub fn generate() -> Self {
        Self { key: SigningKey::generate(&mut OsRng) }
    }

    /// Reconstruit l'identité depuis la graine (32 octets en base64) conservée dans la configuration.
    pub fn from_seed(seed_b64: &str) -> Result<Self> {
        let bytes = STANDARD.decode(seed_b64.trim()).context("graine illisible")?;
        let seed: [u8; 32] = bytes.try_into().map_err(|_| anyhow::anyhow!("la graine doit faire 32 octets"))?;
        Ok(Self { key: SigningKey::from_bytes(&seed) })
    }

    pub fn seed(&self) -> String {
        STANDARD.encode(self.key.to_bytes())
    }

    pub fn public_key(&self) -> String {
        STANDARD.encode(self.key.verifying_key().to_bytes())
    }

    pub fn sign(&self, message: &[u8]) -> String {
        STANDARD.encode(self.key.sign(message).to_bytes())
    }
}

pub fn sha256_hex(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

pub fn register_message(device_id: &str, name: &str, timestamp: u64, nonce: &str) -> Vec<u8> {
    format!("{PREFIX}|register|{device_id}|{name}|{timestamp}|{nonce}").into_bytes()
}

/// Réponse de signaling liée à CETTE offre et à son contenu (empreinte DTLS comprise).
pub fn answer_message(session_id: &str, offer: &str, answer: &str) -> Vec<u8> {
    format!("{PREFIX}|answer|{session_id}|{}|{}", sha256_hex(offer), sha256_hex(answer)).into_bytes()
}

pub fn agent_auth_message(device_id: &str, nonce: &str) -> Vec<u8> {
    format!("{PREFIX}|agent|{device_id}|{nonce}").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signature, Verifier};

    #[test]
    fn the_seed_round_trips_and_gives_the_same_public_key() {
        let identity = Identity::generate();
        let again = Identity::from_seed(&identity.seed()).unwrap();
        assert_eq!(identity.public_key(), again.public_key());
        assert!(Identity::from_seed("pas-du-base64!").is_err());
        assert!(Identity::from_seed(&STANDARD.encode([1u8; 8])).is_err());
    }

    #[test]
    fn a_signature_verifies_with_the_public_key_only() {
        let identity = Identity::from_seed(&STANDARD.encode([7u8; 32])).unwrap();
        let message = agent_auth_message("dev_bureau_0001", "nonce");
        let signature = Signature::from_slice(&STANDARD.decode(identity.sign(&message)).unwrap()).unwrap();
        let public = ed25519_dalek::VerifyingKey::from_bytes(&STANDARD.decode(identity.public_key()).unwrap().try_into().unwrap()).unwrap();
        assert!(public.verify(&message, &signature).is_ok());
        assert!(public.verify(b"autre message", &signature).is_err());
    }

    /// Les mêmes octets que le backend Python : voir backend/tests/test_remote_plugin.py (test_messages_match_the_agent_wire_format).
    #[test]
    fn messages_have_the_exact_wire_format_of_the_backend() {
        assert_eq!(register_message("dev_bureau_0001", "Bureau", 1700000000, "abcdefghijklmnop"),
                   b"pc-assistant-remote-v1|register|dev_bureau_0001|Bureau|1700000000|abcdefghijklmnop");
        assert_eq!(agent_auth_message("dev_bureau_0001", "n0nce"), b"pc-assistant-remote-v1|agent|dev_bureau_0001|n0nce");
        assert_eq!(sha256_hex("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let answer = String::from_utf8(answer_message("sess_x", "offre", "reponse")).unwrap();
        assert_eq!(answer, format!("pc-assistant-remote-v1|answer|sess_x|{}|{}", sha256_hex("offre"), sha256_hex("reponse")));
    }
}
