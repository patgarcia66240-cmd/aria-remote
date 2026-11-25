//! WebRTC côté agent : le contrôleur (navigateur) propose l'offre et ouvre deux canaux de données ; l'agent répond. L'écran et les commandes
//! passent par ces canaux, chiffrés (DTLS), de pair à pair quand c'est possible (STUN) ou via un relais (TURN).
//!   « frames »  : agent -> contrôleur, une image JPEG par envoi, découpée en morceaux (voir `chunk_frame`) ;
//!   « control » : JSON dans les deux sens (infos écran, souris, clavier).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use bytes::{BufMut, Bytes, BytesMut};
use tokio::sync::mpsc::UnboundedSender;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_H264};
use webrtc::api::APIBuilder;
use webrtc::data_channel::RTCDataChannel;
use webrtc::interceptor::registry::Registry;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtcp::payload_feedbacks::full_intra_request::FullIntraRequest;
use webrtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use webrtc::rtcp::receiver_report::ReceiverReport;
use webrtc::rtp_transceiver::rtp_codec::{RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType};
use webrtc::rtp_transceiver::RTCPFeedback;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;
use webrtc::track::track_local::TrackLocal;

use super::signaling::{ClientMessage, IceServerConfig};
use crate::pipeline::RateControl;

/// Taille d'un morceau d'image : bien en dessous de la limite d'un message WebRTC, quel que soit le navigateur.
pub const CHUNK: usize = 16 * 1024;
// (La limite de ce qui peut attendre dans le canal d'envoi n'est plus fixe : voir `pipeline::Governor`.)

/// Découpe un JPEG. En-tête de 8 octets, grand-boutiste : numéro d'image (u32), rang (u16), nombre de morceaux (u16) — même format que
/// `FrameAssembler` (ui/src/remoteApi.js, desktop/web/session.js).
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

/// Piste vidéo H.264 de la session (docs/PLAN_H264.md) : les images encodées y sont écrites, le contrôleur la décode. Présente seulement si le
/// contrôleur l'a demandée dans son offre ET que cette machine sait encoder du H.264 ; sinon l'écran passe en JPEG par le canal « frames ».
#[derive(Clone)]
pub struct VideoOut {
    pub track: Arc<TrackLocalStaticSample>,
    /// Levé par le contrôleur (PLI/FIR : il a perdu des paquets et ne peut plus décoder) ; l'encodeur le lit et produit une image clé.
    pub keyframe_wanted: Arc<AtomicBool>,
    /// Débit de l'encodeur, réglé d'après les pertes que le contrôleur rapporte (RTCP « receiver report ») : voir `pipeline::RateControl`.
    pub rate: Arc<Mutex<RateControl>>,
}

impl VideoOut {
    pub fn new() -> Self {
        let track = Arc::new(TrackLocalStaticSample::new(
            RTCRtpCodecCapability { mime_type: MIME_TYPE_H264.to_owned(), ..Default::default() },
            "video".to_owned(),
            "screen".to_owned(),
        ));
        Self { track, keyframe_wanted: Arc::new(AtomicBool::new(true)), rate: Arc::new(Mutex::new(RateControl::new())) }
    }
}

/// Vrai si l'offre du contrôleur demande de la vidéo H.264 en paquétisation 1 (celle que les navigateurs décodent partout).
pub fn offer_wants_h264(sdp: &str) -> bool {
    let mut in_video = false;
    let (mut h264_payloads, mut mode_one) = (Vec::new(), Vec::new());
    for line in sdp.lines().map(str::trim) {
        if let Some(media) = line.strip_prefix("m=") {
            in_video = media.starts_with("video");
        } else if in_video {
            if let Some(rest) = line.strip_prefix("a=rtpmap:") {
                if let Some((payload, codec)) = rest.split_once(' ') {
                    if codec.to_ascii_uppercase().starts_with("H264/") {
                        h264_payloads.push(payload.to_string());
                    }
                }
            } else if let Some(rest) = line.strip_prefix("a=fmtp:") {
                if let Some((payload, params)) = rest.split_once(' ') {
                    if params.contains("packetization-mode=1") {
                        mode_one.push(payload.to_string());
                    }
                }
            }
        }
    }
    h264_payloads.iter().any(|payload| mode_one.contains(payload))
}

pub struct Peer {
    pc: Arc<RTCPeerConnection>,
    video: Option<VideoOut>,
}

impl Peer {
    /// `on_channel` reçoit chaque canal de données ouvert par le contrôleur ; les candidats ICE locaux partent vers le serveur par `out`.
    pub async fn new(
        ice: &[IceServerConfig],
        session_id: String,
        out: UnboundedSender<ClientMessage>,
        on_channel: Arc<dyn Fn(Arc<RTCDataChannel>) + Send + Sync>,
        on_state: Arc<dyn Fn(RTCPeerConnectionState) + Send + Sync>,
        with_video: bool,
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
        let mut media = MediaEngine::default();
        media.register_codec(
            RTCRtpCodecParameters {
                capability: RTCRtpCodecCapability {
                    mime_type: MIME_TYPE_H264.to_owned(),
                    clock_rate: 90_000,
                    channels: 0,
                    sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f".to_owned(),
                    rtcp_feedback: vec![
                        RTCPFeedback { typ: "nack".to_owned(), parameter: String::new() },
                        RTCPFeedback { typ: "nack".to_owned(), parameter: "pli".to_owned() },
                        RTCPFeedback { typ: "ccm".to_owned(), parameter: "fir".to_owned() },
                    ],
                },
                payload_type: 102,
                ..Default::default()
            },
            RTPCodecType::Video,
        )?;
        let registry = register_default_interceptors(Registry::new(), &mut media)?;
        let pc = Arc::new(APIBuilder::new().with_media_engine(media).with_interceptor_registry(registry).build().new_peer_connection(config).await?);

        let video = if with_video {
            let video = VideoOut::new();
            let sender = pc.add_track(Arc::clone(&video.track) as Arc<dyn TrackLocal + Send + Sync>).await?;
            let (wanted, rate) = (video.keyframe_wanted.clone(), video.rate.clone());
            tokio::spawn(async move {
                // RTCP du contrôleur : PLI / FIR = « envoie une image clé » ; rapport de réception = taux de perte, qui règle le débit.
                // (Il faut lire ce flux de toute façon : c'est ce qui fait tourner NACK.)
                while let Ok((packets, _)) = sender.read_rtcp().await {
                    for packet in &packets {
                        let any = packet.as_any();
                        if any.is::<PictureLossIndication>() || any.is::<FullIntraRequest>() {
                            wanted.store(true, Ordering::Relaxed);
                        } else if let Some(report) = any.downcast_ref::<ReceiverReport>() {
                            if let (Some(block), Ok(mut rate)) = (report.reports.first(), rate.lock()) {
                                rate.on_receiver_report(block.fraction_lost);
                            }
                        }
                    }
                }
            });
            Some(video)
        } else {
            None
        };

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
        Ok(Self { pc, video })
    }

    /// La piste vidéo H.264, si elle a été ajoutée.
    pub fn video(&self) -> Option<VideoOut> {
        self.video.clone()
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

    const CHROME_OFFER: &str = "v=0\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\nm=video 9 UDP/TLS/RTP/SAVPF 102 103 104\r\n\
        a=rtpmap:102 H264/90000\r\na=fmtp:102 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n\
        a=rtpmap:103 H264/90000\r\na=fmtp:103 level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=42e01f\r\n\
        a=rtpmap:104 VP8/90000\r\n";

    #[test]
    fn a_browser_offer_with_h264_packetization_one_asks_for_video() {
        assert!(offer_wants_h264(CHROME_OFFER));
    }

    #[test]
    fn offers_without_usable_h264_video_stay_on_jpeg() {
        assert!(!offer_wants_h264("v=0\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n"), "offre sans vidéo (contrôleurs actuels)");
        assert!(!offer_wants_h264("v=0\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\na=rtpmap:96 VP8/90000\r\n"), "VP8 seulement");
        assert!(!offer_wants_h264("v=0\r\nm=video 9 UDP/TLS/RTP/SAVPF 97\r\na=rtpmap:97 H264/90000\r\na=fmtp:97 packetization-mode=0\r\n"), "mode 0 seulement");
        assert!(!offer_wants_h264("v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=rtpmap:98 H264/90000\r\na=fmtp:98 packetization-mode=1\r\n"), "H.264 hors section vidéo");
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
