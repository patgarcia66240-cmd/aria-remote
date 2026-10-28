//! Signaling avec le backend (plugin remote) : enregistrement HTTP signé, puis WebSocket /api/remote/agent/ws. Le serveur ne voit passer que
//! du texte de négociation (SDP, ICE) ; jamais l'écran ni les commandes.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest, tungstenite::Message, MaybeTlsStream, WebSocketStream};

use crate::config::Config;
use crate::identity::{register_message, Identity};

pub type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Délai maximal pour l'enregistrement et la poignée de main WebSocket : sans lui, un réseau muet bloque l'agent sans rien afficher.
const NETWORK_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct IceServerConfig {
    pub urls: Vec<String>,
    pub username: Option<String>,
    pub credential: Option<String>,
}

/// Messages du serveur vers l'agent.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Challenge { nonce: String },
    Ready { #[serde(default)] ice_servers: Vec<IceServerConfig> },
    OpenSession { session_id: String, permissions: Vec<String>, #[serde(default)] ice_servers: Vec<IceServerConfig> },
    /// `signature` : signature du contrôleur sur son offre (facultative jusqu'à la phase 6 de docs/SIGNATURE_CONTROLEUR.md).
    Offer { session_id: String, sdp: String, #[serde(default)] signature: Option<String> },
    Ice { session_id: String, candidate: String },
    Reconnect { session_id: String },
    CloseSession { session_id: String },
    Error { detail: String },
}

/// Messages de l'agent vers le serveur.
#[derive(Debug, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Auth { signature: String },
    /// Ce que l'appareil autorise, et le type de sa machine (« windows », « windows-laptop »...) : absent chez les anciens agents, ignoré par les anciens serveurs.
    Grant { permissions: Vec<String>, #[serde(skip_serializing_if = "String::is_empty")] platform: String },
    SessionReply { session_id: String, accepted: bool, reason: String },
    Answer { session_id: String, sdp: String, signature: String },
    Ice { session_id: String, candidate: String },
    Bye { session_id: String },
    /// L'agent revient après une coupure du WebSocket et sa session WebRTC est toujours vivante : on la reprend sans renégocier.
    Resume { session_id: String },
    Ping,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn http_base(server: &str) -> String {
    server.trim_end_matches('/').to_string()
}

pub fn websocket_url(server: &str, device_id: &str) -> String {
    let base = http_base(server);
    let base = if let Some(rest) = base.strip_prefix("https://") { format!("wss://{rest}") } else if let Some(rest) = base.strip_prefix("http://") { format!("ws://{rest}") } else { base };
    format!("{base}/api/remote/agent/ws?device_id={device_id}")
}

#[derive(Deserialize)]
struct Registered {
    pairing_code: String,
    expires_in: u64,
}

/// Annonce l'appareil (demande SIGNÉE : preuve de possession de la clé privée, anti-rejeu par horodatage et nonce) et renvoie le code à saisir.
/// Défense en profondeur : la clé d'API ne part jamais vers un serveur non sûr (http hors réseau local, ou schéma inconnu), même si l'appelant
/// a oublié de normaliser l'adresse. Même règle que config::normalize_server.
fn ensure_safe_server(server: &str) -> Result<()> {
    match crate::config::normalize_server(server) {
        Some(normalized) if normalized == server.trim().trim_end_matches('/') => Ok(()),
        _ => bail!("adresse de serveur refusée (https, ou http sur le réseau local seulement)"),
    }
}

pub async fn register(server: &str, api_key: &str, config: &Config, identity: &Identity, platform: &str) -> Result<(String, u64)> {
    ensure_safe_server(server)?;
    let mut raw = [0u8; 18];
    rand::rngs::OsRng.fill_bytes(&mut raw);
    let nonce = base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, raw);
    let timestamp = now();
    let body = serde_json::json!({
        "device_id": config.device_id,
        "name": config.name,
        "public_key": identity.public_key(),
        "platform": platform,
        "timestamp": timestamp,
        "nonce": nonce,
        "signature": identity.sign(&register_message(&config.device_id, &config.name, timestamp, &nonce)),
    });
    let http = reqwest::Client::builder().timeout(NETWORK_TIMEOUT).build().context("client HTTP impossible")?;
    let response = http
        .post(format!("{}/api/remote/devices/register", http_base(server)))
        .header("x-api-key", api_key)
        .json(&body)
        .send()
        .await
        .context("serveur injoignable ou délai dépassé")?;
    let status = response.status();
    if !status.is_success() {
        let detail = response.text().await.unwrap_or_default();
        bail!("enregistrement refusé ({status}) : {detail}");
    }
    let registered: Registered = response.json().await.context("réponse inattendue")?;
    Ok((registered.pairing_code, registered.expires_in))
}

pub async fn connect(server: &str, api_key: &str, device_id: &str) -> Result<Socket> {
    ensure_safe_server(server)?;
    let mut request = websocket_url(server, device_id).into_client_request()?;
    request.headers_mut().insert("x-api-key", api_key.parse().context("clé d'API invalide")?);
    let attempt = match tokio::time::timeout(NETWORK_TIMEOUT, connect_async(request)).await {
        Ok(attempt) => attempt,
        Err(_) => bail!("connexion sans réponse après {} s : vérifie l'adresse et le pare-feu", NETWORK_TIMEOUT.as_secs()),
    };
    match attempt {
        Ok((socket, _)) => Ok(socket),
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) if response.status().as_u16() == 401 => {
            bail!("clé d'API refusée par PC Assistant (401) : vérifie la clé saisie")
        }
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) if response.status().as_u16() == 403 => {
            bail!("clé d'API incorrecte (403) : vérifie la clé saisie")
        }
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) if response.status().as_u16() == 404 => {
            bail!("le plugin « Contrôle à distance » n'est pas activé dans PC Assistant (404) : active-le dans la page Plugins")
        }
        Err(error) => Err(error).context("connexion impossible (PC Assistant éteint, mauvaise adresse ou pare-feu ?)"),
    }
}

pub async fn send(socket: &mut Socket, message: &ClientMessage) -> Result<()> {
    socket.send(Message::Text(serde_json::to_string(message)?)).await?;
    Ok(())
}

/// Prochain message du serveur ; None si la connexion est fermée. Les messages illisibles sont ignorés (journalisés par l'appelant).
pub async fn receive(socket: &mut Socket) -> Option<Result<ServerMessage>> {
    loop {
        match socket.next().await? {
            Ok(Message::Text(text)) => return Some(serde_json::from_str(&text).with_context(|| format!("message inconnu : {text}"))),
            Ok(Message::Close(_)) => return None,
            Ok(_) => continue,
            Err(error) => return Some(Err(error.into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_websocket_url_follows_the_server_scheme() {
        assert_eq!(websocket_url("http://192.168.1.10:8000/", "dev_a"), "ws://192.168.1.10:8000/api/remote/agent/ws?device_id=dev_a");
        assert_eq!(websocket_url("https://aria.example", "dev_a"), "wss://aria.example/api/remote/agent/ws?device_id=dev_a");
    }

    #[test]
    fn server_messages_parse_like_the_backend_sends_them() {
        let open: ServerMessage = serde_json::from_str(r#"{"type":"open_session","session_id":"sess_1","permissions":["view_screen"],"ice_servers":[{"urls":["stun:s:3478"]}]}"#).unwrap();
        assert_eq!(open, ServerMessage::OpenSession { session_id: "sess_1".into(), permissions: vec!["view_screen".into()],
                   ice_servers: vec![IceServerConfig { urls: vec!["stun:s:3478".into()], username: None, credential: None }] });
        let ready: ServerMessage = serde_json::from_str(r#"{"type":"ready"}"#).unwrap();
        assert_eq!(ready, ServerMessage::Ready { ice_servers: vec![] });
        assert!(serde_json::from_str::<ServerMessage>(r#"{"type":"inconnu"}"#).is_err());
    }

    #[test]
    fn client_messages_serialize_with_the_type_the_backend_expects() {
        let reply = serde_json::to_value(ClientMessage::SessionReply { session_id: "s".into(), accepted: false, reason: "non".into() }).unwrap();
        assert_eq!(reply, serde_json::json!({"type": "session_reply", "session_id": "s", "accepted": false, "reason": "non"}));
        assert_eq!(serde_json::to_value(ClientMessage::Ping).unwrap(), serde_json::json!({"type": "ping"}));
        assert_eq!(serde_json::to_value(ClientMessage::Grant { permissions: vec!["view_screen".into()], platform: String::new() }).unwrap()["type"], "grant");
        // le type de machine n'est envoyé que s'il est connu : un ancien serveur reçoit exactement le message d'avant
        assert!(serde_json::to_value(ClientMessage::Grant { permissions: vec![], platform: String::new() }).unwrap().get("platform").is_none());
        assert_eq!(serde_json::to_value(ClientMessage::Grant { permissions: vec![], platform: "windows-laptop".into() }).unwrap()["platform"], "windows-laptop");
    }
}

#[cfg(test)]
mod safety_tests {
    use super::ensure_safe_server;

    #[test]
    fn the_api_key_only_travels_to_https_or_to_the_local_network_over_http() {
        assert!(ensure_safe_server("https://rendezvous.exemple.fr").is_ok());
        assert!(ensure_safe_server("http://192.168.1.20:8000").is_ok());
        assert!(ensure_safe_server("http://127.0.0.1:8000").is_ok());
        assert!(ensure_safe_server("http://attaquant.exemple").is_err(), "http vers Internet : la clé partirait en clair");
        assert!(ensure_safe_server("ftp://192.168.1.20").is_err());
        assert!(ensure_safe_server("").is_err());
    }
}
