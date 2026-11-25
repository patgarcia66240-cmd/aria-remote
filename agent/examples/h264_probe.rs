//! Sonde de transport H.264 (étape 2 du plan docs/PLAN_H264.md) : envoie un fichier H.264 (Annex B) en boucle sur une VRAIE piste vidéo WebRTC
//! vers une page de navigateur, pour vérifier ce que l'agent devra gérer : décodage chez le navigateur, NACK, PLI, rapports de réception.
//! Ce n'est pas l'agent : pas de capture ni d'encodeur ici (le fichier vient de ffmpeg), seulement le transport.
//!
//!   cargo run --release --example h264_probe -- clip.h264 [--fps 10] [--port 8099] [--loss 2] [--delay 40] [--jitter 10]
//! --loss : % de paquets RTP perdus avant le réseau (les retransmissions NACK y sont soumises aussi) ; --delay/--jitter : ms ajoutées à chaque paquet.
//! puis ouvrir http://127.0.0.1:8099/ . Les compteurs RTCP de l'ÉMETTEUR sortent sur la console toutes les 2 s.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::extract::State;
use axum::response::Html;
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::{Bytes, BytesMut};
use serde::{Deserialize, Serialize};
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_H264};
use webrtc::api::APIBuilder;
use webrtc::interceptor::noop::NoOp;
use webrtc::interceptor::registry::Registry;
use webrtc::interceptor::stream_info::StreamInfo;
use webrtc::interceptor::{Attributes, Interceptor, InterceptorBuilder, RTCPReader, RTCPWriter, RTPReader, RTPWriter};
use webrtc::media::Sample;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtcp::payload_feedbacks::full_intra_request::FullIntraRequest;
use webrtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use webrtc::rtcp::receiver_report::ReceiverReport;
use webrtc::rtcp::transport_feedbacks::transport_layer_nack::TransportLayerNack;
use webrtc::rtp_transceiver::rtp_codec::{RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType};
use webrtc::rtp_transceiver::RTCPFeedback;
use webrtc::util::MarshalSize;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;
use webrtc::track::track_local::TrackLocal;

const PAGE: &str = include_str!("h264_probe.html");
const START_CODE: [u8; 4] = [0, 0, 0, 1];

/// Dégradations simulées sur l'envoi RTP : perte aléatoire, délai fixe + gigue. Posé AVANT l'intercepteur NACK dans le registre, donc au plus
/// près du réseau : les paquets retransmis sur NACK sont perdus ou retardés comme les autres.
#[derive(Clone, Copy)]
struct Impairment {
    loss: f64,       // 0..1
    delay_ms: u64,
    jitter_ms: u64,
}

impl InterceptorBuilder for Impairment {
    fn build(&self, _id: &str) -> Result<Arc<dyn Interceptor + Send + Sync>, webrtc::interceptor::Error> {
        Ok(Arc::new(Impaired { settings: *self, base: NoOp }))
    }
}

struct Impaired {
    settings: Impairment,
    base: NoOp,
}

#[async_trait::async_trait]
impl Interceptor for Impaired {
    async fn bind_rtcp_reader(&self, reader: Arc<dyn RTCPReader + Send + Sync>) -> Arc<dyn RTCPReader + Send + Sync> {
        self.base.bind_rtcp_reader(reader).await
    }
    async fn bind_rtcp_writer(&self, writer: Arc<dyn RTCPWriter + Send + Sync>) -> Arc<dyn RTCPWriter + Send + Sync> {
        self.base.bind_rtcp_writer(writer).await
    }
    async fn bind_local_stream(&self, _info: &StreamInfo, writer: Arc<dyn RTPWriter + Send + Sync>) -> Arc<dyn RTPWriter + Send + Sync> {
        Arc::new(ImpairedWriter { settings: self.settings, next: writer })
    }
    async fn unbind_local_stream(&self, info: &StreamInfo) {
        self.base.unbind_local_stream(info).await
    }
    async fn bind_remote_stream(&self, info: &StreamInfo, reader: Arc<dyn RTPReader + Send + Sync>) -> Arc<dyn RTPReader + Send + Sync> {
        self.base.bind_remote_stream(info, reader).await
    }
    async fn unbind_remote_stream(&self, info: &StreamInfo) {
        self.base.unbind_remote_stream(info).await
    }
    async fn close(&self) -> Result<(), webrtc::interceptor::Error> {
        self.base.close().await
    }
}

struct ImpairedWriter {
    settings: Impairment,
    next: Arc<dyn RTPWriter + Send + Sync>,
}

#[async_trait::async_trait]
impl RTPWriter for ImpairedWriter {
    async fn write(&self, pkt: &webrtc::rtp::packet::Packet, attributes: &Attributes) -> Result<usize, webrtc::interceptor::Error> {
        let Impairment { loss, delay_ms, jitter_ms } = self.settings;
        let (lost, jitter) = (rand::random::<f64>() < loss, if jitter_ms > 0 { rand::random::<u64>() % (jitter_ms + 1) } else { 0 });
        let size = pkt.marshal_size();
        if lost {
            return Ok(size);   // « envoyé » du point de vue de l'émetteur, jamais arrivé
        }
        if delay_ms + jitter == 0 {
            return self.next.write(pkt, attributes).await;
        }
        let (next, pkt, attributes) = (self.next.clone(), pkt.clone(), attributes.clone());
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(delay_ms + jitter)).await;
            let _ = next.write(&pkt, &attributes).await;
        });
        Ok(size)
    }
}

#[derive(Default)]
struct Counters {
    nack_packets: AtomicU64,   // paquets réclamés par des NACK
    plis: AtomicU64,           // demandes d'image clé (PLI + FIR)
    frames: AtomicU64,
    bytes: AtomicU64,
    lost_fraction: AtomicU64,  // dernier « fraction lost » des rapports du récepteur, sur 256
}

struct App {
    access_units: Arc<Vec<Bytes>>,
    fps: u64,
    impairment: Impairment,
    counters: Arc<Counters>,
}

#[derive(Deserialize, Serialize)]
struct Sdp {
    sdp: String,
}

/// Découpe le fichier en images (une image = ses NAL de tête SPS/PPS/SEI + sa tranche), au format Annex B. Découpage à la main sur les codes de
/// départ 00 00 01 : `H264Reader` de webrtc-media coupait ce flux en 14 morceaux par image.
fn access_units(path: &str) -> Result<Vec<Bytes>> {
    let data = std::fs::read(path).with_context(|| format!("lecture de {path}"))?;
    let mut starts = Vec::new();   // position du premier octet APRÈS chaque code de départ
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let (mut units, mut current) = (Vec::new(), BytesMut::new());
    for (n, &begin) in starts.iter().enumerate() {
        let mut end = starts.get(n + 1).map_or(data.len(), |next| next - 3);
        while end > begin && data[end - 1] == 0 {
            end -= 1;   // zéros de bourrage / début d'un code de départ sur 4 octets
        }
        if end == begin {
            continue;
        }
        let nal = &data[begin..end];
        current.extend_from_slice(&START_CODE);
        current.extend_from_slice(nal);
        if matches!(nal[0] & 0x1f, 1 | 5) {
            units.push(current.split().freeze());   // tranche d'image : l'image est complète (une tranche par image)
        }
    }
    anyhow::ensure!(!units.is_empty(), "aucune image dans {path}");
    Ok(units)
}

async fn offer(State(app): State<Arc<App>>, Json(body): Json<Sdp>) -> Result<Json<Sdp>, String> {
    answer(&app, body.sdp).await.map(|sdp| Json(Sdp { sdp })).map_err(|error| format!("{error:#}"))
}

async fn answer(app: &Arc<App>, sdp: String) -> Result<String> {
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
    let mut registry = Registry::new();
    registry.add(Box::new(app.impairment));   // en premier = au plus près du réseau (voir `Impairment`)
    let registry = register_default_interceptors(registry, &mut media)?;
    let api = APIBuilder::new().with_media_engine(media).with_interceptor_registry(registry).build();
    let pc = Arc::new(api.new_peer_connection(RTCConfiguration::default()).await?);

    let track = Arc::new(TrackLocalStaticSample::new(
        RTCRtpCodecCapability { mime_type: MIME_TYPE_H264.to_owned(), ..Default::default() },
        "video".to_owned(),
        "probe".to_owned(),
    ));
    let sender = pc.add_track(Arc::clone(&track) as Arc<dyn TrackLocal + Send + Sync>).await?;

    // RTCP reçu du navigateur : ce que l'agent devra interpréter pour piloter l'encodeur.
    let counters = app.counters.clone();
    tokio::spawn(async move {
        while let Ok((packets, _)) = sender.read_rtcp().await {
            for packet in packets {
                let any = packet.as_any();
                if let Some(nack) = any.downcast_ref::<TransportLayerNack>() {
                    let lost: u64 = nack.nacks.iter().map(|pair| 1 + u64::from(pair.lost_packets.count_ones())).sum();
                    counters.nack_packets.fetch_add(lost, Ordering::Relaxed);
                } else if any.is::<PictureLossIndication>() || any.is::<FullIntraRequest>() {
                    counters.plis.fetch_add(1, Ordering::Relaxed);
                } else if let Some(report) = any.downcast_ref::<ReceiverReport>() {
                    if let Some(block) = report.reports.first() {
                        counters.lost_fraction.store(u64::from(block.fraction_lost), Ordering::Relaxed);
                    }
                }
            }
        }
    });

    let (units, fps, counters) = (app.access_units.clone(), app.fps, app.counters.clone());
    let writer = track.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(1000 / fps));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        for unit in units.iter().cycle() {
            tick.tick().await;
            let sample = Sample { data: unit.clone(), duration: Duration::from_millis(1000 / fps), ..Default::default() };
            if writer.write_sample(&sample).await.is_err() {
                break;
            }
            counters.frames.fetch_add(1, Ordering::Relaxed);
            counters.bytes.fetch_add(unit.len() as u64, Ordering::Relaxed);
        }
    });

    pc.set_remote_description(RTCSessionDescription::offer(sdp)?).await?;
    let answer = pc.create_answer(None).await?;
    let mut gathered = pc.gathering_complete_promise().await;
    pc.set_local_description(answer).await?;
    let _ = gathered.recv().await;
    Ok(pc.local_description().await.context("pas de description locale")?.sdp)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str, default: u64| args.windows(2).find(|w| w[0] == name).and_then(|w| w[1].parse().ok()).unwrap_or(default);
    let path = args.first().filter(|a| !a.starts_with("--")).context("usage : h264_probe <fichier.h264> [--fps 10] [--port 8099]")?;
    let (fps, port) = (flag("--fps", 10).max(1), flag("--port", 8099));
    let impairment = Impairment { loss: flag("--loss", 0) as f64 / 100.0, delay_ms: flag("--delay", 0), jitter_ms: flag("--jitter", 0) };
    let app = Arc::new(App { access_units: Arc::new(access_units(path)?), fps, impairment, counters: Arc::new(Counters::default()) });
    println!("{} images, {fps} images/s, perte {} %, délai {} ms + gigue {} ms : http://127.0.0.1:{port}/", app.access_units.len(), impairment.loss * 100.0, impairment.delay_ms, impairment.jitter_ms);

    let counters = app.counters.clone();
    tokio::spawn(async move {
        let mut last = (0u64, 0u64, 0u64);
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let now = (counters.frames.load(Ordering::Relaxed), counters.bytes.load(Ordering::Relaxed), counters.nack_packets.load(Ordering::Relaxed));
            println!(
                "[emetteur] {} images, {} kbit/s | NACK : {} paquets (+{}) | PLI/FIR : {} | perte rapportée : {:.1} %",
                now.0, (now.1 - last.1) * 8 / 2000, now.2, now.2 - last.2, counters.plis.load(Ordering::Relaxed),
                counters.lost_fraction.load(Ordering::Relaxed) as f64 * 100.0 / 256.0
            );
            last = now;
        }
    });

    let router = Router::new().route("/", get(|| async { Html(PAGE) })).route("/offer", post(offer)).with_state(app);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port as u16)).await?;
    axum::serve(listener, router).await?;
    Ok(())
}
