//! WebRTC côté agent : le contrôleur (navigateur) propose l'offre et ouvre deux canaux de données ; l'agent répond. L'écran et les commandes
//! passent par ces canaux, chiffrés (DTLS), de pair à pair quand c'est possible (STUN) ou via un relais (TURN).
//!   « frames »  : agent -> contrôleur, une image JPEG par envoi, découpée en morceaux (voir `chunk_frame`) ;
//!   « control » : JSON dans les deux sens (infos écran, souris, clavier).

use std::sync::Arc;

use anyhow::{Context, Result};
use bytes::{BufMut, Bytes, BytesMut};
use tokio::sync::mpsc::UnboundedSender;
use webrtc::api::APIBuilder;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;

use super::signaling::{ClientMessage, IceServerConfig};

/// Taille d'un morceau d'image : bien en dessous de la limite d'un message WebRTC, quel que soit le navigateur.
pub const CHUNK: usize = 16 * 1024;
/// Au-delà, le canal est saturé (réseau lent) : on saute des images plutôt que d'accumuler du retard.
pub const MAX_BUFFERED: usize = 1_000_000;

/// Découpe un JPEG. En-tête de 8 octets, grand-boutiste : numéro d'image (u32), rang (u16), nombre de morceaux (u16) — même format que
/// `FrameAssembler` dans frontend/src/components/remoteApi.js.
pub fn chunk_frame(frame_id: u32, jpeg: &[u8]) -> Vec<Bytes> {
    let count = jpeg.len().div_ceil(CHUNK);
    if count == 0 || count > usize::from(u16::MAX) {
        return Vec::new();
    }
    jpeg.chunks(CHUNK)
        .enumerate()
        .map(|(index, part)| {
            let mut message = BytesMut::with_capacity(8 + part.len());
            message.put_u32(frame_id);
            message.put_u16(index as u16);
            message.put_u16(count as u16);
            message.extend_from_slice(part);
            message.freeze()
        })
        .collect()
}

pub struct Peer {
    pc: Arc<RTCPeerConnection>,
}

impl Peer {
    /// `on_channel` reçoit chaque canal de données ouvert par le contrôleur ; les candidats ICE locaux partent vers le serveur par `out`.
    pub async fn new(
        ice: &[IceServerConfig],
        session_id: String,
        out: UnboundedSender<ClientMessage>,
        on_channel: Arc<dyn Fn(Arc<RTCDataChannel>) + Send + Sync>,
        on_state: Arc<dyn Fn(RTCPeerConnectionState) + Send + Sync>,
    ) -> Result<Self> {
        let config = RTCConfiguration {
            ice_servers: ice
                .iter()
                .map(|server| RTCIceServer {
                    urls: server.urls.clone(),
                    username: server.username.clone().unwrap_or_default(),
                    credential: server.credential.clone().unwrap_or_default(),
                })
                .collect(),
            ..Default::default()
        };
        let pc = Arc::new(APIBuilder::new().build().new_peer_connection(config).await?);

        pc.on_ice_candidate(Box::new(move |candidate| {
            let (out, session_id) = (out.clone(), session_id.clone());
            Box::pin(async move {
                if let Some(init) = candidate.and_then(|c| c.to_json().ok()) {
                    if let Ok(text) = serde_json::to_string(&init) {
                        let _ = out.send(ClientMessage::Ice { session_id, candidate: text });
                    }
                }
            })
        }));
        pc.on_data_channel(Box::new(move |channel| {
            on_channel(channel);
            Box::pin(async {})
        }));
        pc.on_peer_connection_state_change(Box::new(move |state| {
            eprintln!("[webrtc] état : {state}");
            on_state(state);
            Box::pin(async {})
        }));
        Ok(Self { pc })
    }

    /// Répond à l'offre ; renvoie le SDP EXACT à envoyer (et à signer).
    pub async fn answer(&self, offer_sdp: String) -> Result<String> {
        self.pc.set_remote_description(RTCSessionDescription::offer(offer_sdp)?).await?;
        let answer = self.pc.create_answer(None).await?;
        self.pc.set_local_description(answer).await?;
        let local = self.pc.local_description().await.context("pas de description locale")?;
        Ok(local.sdp)
    }

    pub async fn add_ice(&self, candidate_json: &str) -> Result<()> {
        let init: RTCIceCandidateInit = serde_json::from_str(candidate_json).context("candidat ICE illisible")?;
        self.pc.add_ice_candidate(init).await?;
        Ok(())
    }

    pub fn is_connected(&self) -> bool {
        self.pc.connection_state() == RTCPeerConnectionState::Connected
    }

    pub async fn close(&self) {
        let _ = self.pc.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reassemble(chunks: &[Bytes]) -> (u32, Vec<u8>) {
        let mut data = Vec::new();
        let mut id = 0;
        for (expected, chunk) in chunks.iter().enumerate() {
            id = u32::from_be_bytes(chunk[0..4].try_into().unwrap());
            assert_eq!(u16::from_be_bytes(chunk[4..6].try_into().unwrap()) as usize, expected);
            assert_eq!(u16::from_be_bytes(chunk[6..8].try_into().unwrap()) as usize, chunks.len());
            data.extend_from_slice(&chunk[8..]);
        }
        (id, data)
    }

    #[test]
    fn a_frame_is_cut_into_numbered_chunks_that_reassemble_exactly() {
        let jpeg: Vec<u8> = (0..(CHUNK * 2 + 5)).map(|i| (i % 251) as u8).collect();
        let chunks = chunk_frame(0xAABBCCDD, &jpeg);
        assert_eq!(chunks.len(), 3);
        assert!(chunks.iter().all(|c| c.len() <= 8 + CHUNK));
        assert_eq!(reassemble(&chunks), (0xAABBCCDD, jpeg));
    }

    #[test]
    fn small_and_empty_frames() {
        let one = chunk_frame(1, &[1, 2, 3]);
        assert_eq!(one.len(), 1);
        assert_eq!(&one[0][..], &[0, 0, 0, 1, 0, 0, 0, 1, 1, 2, 3]);
        assert!(chunk_frame(2, &[]).is_empty());
    }

    #[test]
    fn candidates_use_the_browser_json_shape() {
        let init: RTCIceCandidateInit = serde_json::from_str(r#"{"candidate":"candidate:1 1 udp 2122 192.168.1.2 5000 typ host","sdpMid":"0","sdpMLineIndex":0,"usernameFragment":null}"#).unwrap();
        assert_eq!(init.sdp_mid.as_deref(), Some("0"));
        assert_eq!(init.sdp_mline_index, Some(0));
        let text = serde_json::to_string(&init).unwrap();
        assert!(text.contains("sdpMLineIndex") && text.contains("sdpMid"));
    }
}
