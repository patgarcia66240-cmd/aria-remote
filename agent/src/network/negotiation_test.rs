//! Négociation de bout en bout, dans un seul processus : un VRAI `Peer` d'agent (capture synthétique, encodeur réel sous Windows) face à un
//! « contrôleur » écrit avec la même bibliothèque, avec de vraies connexions WebRTC sur cette machine.
//! Vérifie ce que docs/PLAN_H264.md demande à l'étape 3 : la vidéo H.264 n'apparaît que si le contrôleur la demande ; sinon (contrôleurs actuels) rien
//! ne change et l'écran reste en JPEG ; et le repli JPEG, demandé en cours de route, marche.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use tokio::sync::mpsc;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_H264};
use webrtc::api::APIBuilder;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtp_transceiver::rtp_codec::{RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType};
use webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection;
use webrtc::rtp_transceiver::{RTCRtpTransceiverInit, RTCPFeedback};

use super::signaling::ClientMessage;
use super::webrtc::{offer_wants_h264, Peer};
use crate::capture::h264::H264Encoder;
use crate::capture::screen::synthetic_backend;
use crate::clipboard::MemoryClipboard;
use crate::input::{LogInput, Permission};
use crate::session::{attach_channel, Session};

struct Controller {
    pc: Arc<RTCPeerConnection>,
    control: Arc<RTCDataChannel>,
    rtp_packets: Arc<AtomicUsize>,
    jpeg_messages: Arc<AtomicUsize>,
    texts: Arc<Mutex<Vec<String>>>,
    _peer: Peer,
}

impl Controller {
    fn saw(&self, needle: &str) -> bool {
        self.texts.lock().unwrap().iter().any(|text| text.contains(needle))
    }
}

async fn eventually(what: &str, mut condition: impl FnMut() -> bool) {
    for _ in 0..200 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("délai dépassé : {what}");
}

/// Établit une session complète. `with_video` : le contrôleur met de la vidéo H.264 dans son offre (comme le fera le nouveau contrôleur).
async fn connect(with_video: bool) -> Controller {
    // Côté contrôleur.
    let mut media = MediaEngine::default();
    media
        .register_codec(
            RTCRtpCodecParameters {
                capability: RTCRtpCodecCapability {
                    mime_type: MIME_TYPE_H264.to_owned(),
                    clock_rate: 90_000,
                    channels: 0,
                    sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f".to_owned(),
                    rtcp_feedback: vec![
                        RTCPFeedback { typ: "nack".to_owned(), parameter: String::new() },
                        RTCPFeedback { typ: "nack".to_owned(), parameter: "pli".to_owned() },
                    ],
                },
                payload_type: 102,
                ..Default::default()
            },
            RTPCodecType::Video,
        )
        .unwrap();
    let registry = register_default_interceptors(Registry::new(), &mut media).unwrap();
    let api = APIBuilder::new().with_media_engine(media).with_interceptor_registry(registry).build();
    let pc = Arc::new(api.new_peer_connection(RTCConfiguration::default()).await.unwrap());

    let (rtp_packets, jpeg_messages, texts) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)), Arc::new(Mutex::new(Vec::new())));
    let frames = pc.create_data_channel("frames", None).await.unwrap();
    let counter = jpeg_messages.clone();
    frames.on_message(Box::new(move |_: DataChannelMessage| {
        counter.fetch_add(1, Ordering::Relaxed);
        Box::pin(async {})
    }));
    let control = pc.create_data_channel("control", None).await.unwrap();
    let sink = texts.clone();
    control.on_message(Box::new(move |message: DataChannelMessage| {
        sink.lock().unwrap().push(String::from_utf8_lossy(&message.data).into_owned());
        Box::pin(async {})
    }));
    if with_video {
        pc.add_transceiver_from_kind(RTPCodecType::Video, Some(RTCRtpTransceiverInit { direction: RTCRtpTransceiverDirection::Recvonly, send_encodings: vec![] }))
            .await
            .unwrap();
    }
    let counter = rtp_packets.clone();
    pc.on_track(Box::new(move |track, _, _| {
        let counter = counter.clone();
        tokio::spawn(async move {
            while track.read_rtp().await.is_ok() {
                counter.fetch_add(1, Ordering::Relaxed);
            }
        });
        Box::pin(async {})
    }));

    let offer = pc.create_offer(None).await.unwrap();
    let mut gathered = pc.gathering_complete_promise().await;
    pc.set_local_description(offer).await.unwrap();
    let _ = gathered.recv().await;
    let offer_sdp = pc.local_description().await.unwrap().sdp;

    // Côté agent : même enchaînement que `Agent::on_offer`.
    let session = Session::new(
        [Permission::ViewScreen].into_iter().collect(),
        Arc::new(Mutex::new(Box::new(LogInput::new()))),
        synthetic_backend(),
        Arc::new(Mutex::new(Box::new(MemoryClipboard(None)))),
    );
    let handle = session.clone();
    let on_channel: Arc<dyn Fn(Arc<RTCDataChannel>) + Send + Sync> = Arc::new(move |channel| attach_channel(channel, session.clone()));
    let on_state: Arc<dyn Fn(RTCPeerConnectionState) + Send + Sync> = Arc::new(|_| {});
    let (out, mut signals) = mpsc::unbounded_channel();
    let wants_video = offer_wants_h264(&offer_sdp) && H264Encoder::available();
    let peer = Peer::new(&[], "test".into(), out, on_channel, on_state, wants_video).await.unwrap();
    if let Some(video) = peer.video() {
        handle.set_video(video);
    }
    let answer = peer.answer(offer_sdp).await.unwrap();
    pc.set_remote_description(RTCSessionDescription::answer(answer).unwrap()).await.unwrap();

    // Les candidats ICE de l'agent arrivent par « signaling » : on les passe au contrôleur.
    let remote = pc.clone();
    tokio::spawn(async move {
        while let Some(message) = signals.recv().await {
            if let ClientMessage::Ice { candidate, .. } = message {
                if let Ok(init) = serde_json::from_str::<RTCIceCandidateInit>(&candidate) {
                    let _ = remote.add_ice_candidate(init).await;
                }
            }
        }
    });
    Controller { pc, control, rtp_packets, jpeg_messages, texts, _peer: peer }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_controller_without_video_keeps_getting_jpeg_exactly_as_before() {
    let controller = connect(false).await;
    eventually("des images JPEG sur le canal « frames »", || controller.jpeg_messages.load(Ordering::Relaxed) >= 3).await;
    eventually("le message info", || controller.saw("\"t\":\"info\"")).await;
    assert!(!controller.saw("h264"), "un contrôleur qui n'a pas demandé la vidéo ne doit rien voir de H.264 : {:?}", controller.texts.lock().unwrap());
    assert_eq!(controller.rtp_packets.load(Ordering::Relaxed), 0, "aucune piste vidéo sans demande");
    let _ = controller.pc.close().await;
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_controller_that_asks_for_h264_gets_video_and_can_fall_back_to_jpeg() {
    assert!(H264Encoder::available(), "pas d'encodeur H.264 sur cette machine");
    let controller = connect(true).await;
    eventually("la fonction h264 dans info", || controller.saw("h264")).await;
    eventually("des paquets vidéo RTP", || controller.rtp_packets.load(Ordering::Relaxed) >= 20).await;
    assert_eq!(controller.jpeg_messages.load(Ordering::Relaxed), 0, "en H.264, le canal JPEG reste muet");

    // Repli demandé en cours de route : le JPEG reprend par « frames », le changement est annoncé.
    controller.control.send(&Bytes::from_static(br#"{"t":"codec","name":"jpeg"}"#)).await.unwrap();
    eventually("le retour des images JPEG", || controller.jpeg_messages.load(Ordering::Relaxed) >= 3).await;
    eventually("l'annonce du codec jpeg", || controller.saw(r#""t":"codec""#) && controller.saw(r#""name":"jpeg""#)).await;

    // Et retour au H.264.
    let before = controller.rtp_packets.load(Ordering::Relaxed);
    controller.control.send(&Bytes::from_static(br#"{"t":"codec","name":"h264"}"#)).await.unwrap();
    eventually("la reprise de la vidéo", || controller.rtp_packets.load(Ordering::Relaxed) > before + 20).await;
    let _ = controller.pc.close().await;
}
