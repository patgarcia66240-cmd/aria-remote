//! Une session de contrôle : branche les canaux de données du contrôleur sur la capture d'écran (« frames ») et la saisie (« control »).
//! Les permissions de la session sont appliquées ICI à chaque commande, même si le serveur et le contrôleur sont en règle.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use tokio::sync::mpsc;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::RTCDataChannel;

use crate::capture::screen::{encode_jpeg, JPEG_QUALITY, MAX_WIDTH};
use crate::capture::ScreenSource;
use crate::input::{self, Control, InputSink, Permission};
use crate::network::webrtc::{chunk_frame, MAX_BUFFERED};

pub const FPS: u64 = 10;

pub type SourceFactory = Arc<dyn Fn() -> Result<Box<dyn ScreenSource>> + Send + Sync>;
pub type SharedInput = Arc<Mutex<Box<dyn InputSink>>>;

/// Branche un canal de données ouvert par le contrôleur selon son nom ; un canal inconnu est ignoré.
pub fn attach_channel(channel: Arc<RTCDataChannel>, permissions: Arc<HashSet<Permission>>, input: SharedInput, source: SourceFactory) {
    match channel.label() {
        "frames" => {
            if permissions.contains(&Permission::ViewScreen) {
                let channel = channel.clone();
                let on_open = channel.clone();
                on_open.on_open(Box::new(move || {
                    start_frames(channel, source);
                    Box::pin(async {})
                }));
            }
        }
        "control" => attach_control(channel, permissions, input),
        other => eprintln!("[session] canal inconnu ignoré : {other}"),
    }
}

fn attach_control(channel: Arc<RTCDataChannel>, permissions: Arc<HashSet<Permission>>, input: SharedInput) {
    let info_channel = channel.clone();
    let info_permissions = permissions.clone();
    let info_input = input.clone();
    channel.on_open(Box::new(move || {
        let (width, height) = info_input.lock().map(|sink| sink.display_size()).unwrap_or((0, 0));
        let mut names: Vec<&str> = info_permissions.iter().map(|p| p.as_str()).collect();
        names.sort_unstable();
        let info = serde_json::json!({"t": "info", "width": width, "height": height, "permissions": names}).to_string();
        Box::pin(async move { let _ = info_channel.send_text(info).await; })
    }));

    let reply = channel.clone();
    channel.on_message(Box::new(move |message: DataChannelMessage| {
        let outcome = match serde_json::from_slice::<Control>(&message.data) {
            Ok(control) => match input.lock() {
                Ok(mut sink) => input::apply(&control, &permissions, sink.as_mut()),
                Err(_) => Err("saisie indisponible"),
            },
            Err(_) => Err("commande illisible"),
        };
        let reply = reply.clone();
        Box::pin(async move {
            if let Err(reason) = outcome {
                eprintln!("[session] commande refusée : {reason}");
                let _ = reply.send_text(serde_json::json!({"t": "denied", "reason": reason}).to_string()).await;
            }
        })
    }));
}

/// Capture dans un thread dédié (la source d'écran y est créée et y reste), envoi asynchrone avec contrôle de saturation.
fn start_frames(channel: Arc<RTCDataChannel>, source: SourceFactory) {
    let (frames, mut ready) = mpsc::channel::<Vec<u8>>(2);
    std::thread::spawn(move || {
        let mut screen = match source() {
            Ok(screen) => screen,
            Err(error) => {
                eprintln!("[capture] écran indisponible : {error:#}");
                return;
            }
        };
        let period = Duration::from_millis(1000 / FPS);
        loop {
            let started = Instant::now();
            match screen.capture().and_then(|frame| encode_jpeg(frame, MAX_WIDTH, JPEG_QUALITY)) {
                Ok(jpeg) => {
                    if frames.blocking_send(jpeg).is_err() {
                        break;      // la session est terminée : plus personne pour recevoir
                    }
                }
                Err(error) => {
                    eprintln!("[capture] {error:#}");
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
            if let Some(rest) = period.checked_sub(started.elapsed()) {
                std::thread::sleep(rest);
            }
        }
    });
    tokio::spawn(async move {
        let mut frame_id = 0u32;
        while let Some(jpeg) = ready.recv().await {
            if channel.buffered_amount().await > MAX_BUFFERED {
                continue;
            }
            for chunk in chunk_frame(frame_id, &jpeg) {
                if channel.send(&chunk).await.is_err() {
                    return;
                }
            }
            frame_id = frame_id.wrapping_add(1);
        }
    });
}
