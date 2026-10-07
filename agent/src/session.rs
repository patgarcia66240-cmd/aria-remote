//! Une session de contrôle : branche les canaux de données du contrôleur sur la capture d'écran (« frames ») et la saisie (« control »).
//! Les permissions de la session sont appliquées ICI à chaque commande, même si le serveur et le contrôleur sont en règle.
//!
//! Messages du canal « control » (JSON) :
//!   contrôleur -> agent : souris et clavier (input.rs), puis
//!     ping{id}                    : mesure de latence (réponse pong{id}) ;
//!     stream{fps?, quality?, width?} : réglages de l'image (vue de l'écran) ;
//!     screen{index}               : écran à afficher et à piloter (vue de l'écran) ;
//!     clip{text}                  : texte à mettre dans le presse-papiers de l'appareil (permission clavier).
//!   agent -> contrôleur : info{width, height, permissions, screens, screen, features}, cursor{x, y}, pong{id}, clip{text}, denied{reason},
//!     stats{fps, kbps, capture_ms, encode_ms, dropped, idle} toutes les 2 s (voir pipeline.rs).
//! Canaux : « frames » (images JPEG découpées, le contrôleur le crée non ordonné à fiabilité partielle : une image perdue ne retarde pas la suivante),
//!   « control » (fiable et ordonné) et « pointer » (non fiable : seulement les déplacements de souris, la dernière position suffit).

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::RTCDataChannel;

use crate::capture::screen::{JpegEncoder, JPEG_QUALITY, MAX_WIDTH};
use crate::capture::{Frame, Screen, Screens};
use crate::clipboard::{self, Clipboard};
use crate::input::{self, Area, Control, InputSink, Permission};
use crate::network::webrtc::chunk_frame;
use crate::pipeline::{fingerprint, Counters, Governor, Slot};

const CURSOR_HZ: u64 = 20;
const CLIPBOARD_POLL: Duration = Duration::from_secs(1);

/// Ce que cet agent sait faire, annoncé au contrôleur dans « info » : un contrôleur n'utilise que ce qui est listé (les anciens agents n'ont pas la liste).
pub const FEATURES: [&str; 6] = ["ping", "stream", "screens", "clipboard", "pointer", "stats"];

/// Un écran qui ne change pas n'est pas renvoyé… sauf toutes les 1,5 s : une image perdue en route (canal à fiabilité partielle) est ainsi
/// toujours remplacée, même devant un écran fixe.
const HEARTBEAT: Duration = Duration::from_millis(1500);
const STATS_EVERY: Duration = Duration::from_secs(2);

pub type SharedInput = Arc<Mutex<Box<dyn InputSink>>>;
pub type SharedClipboard = Arc<Mutex<Box<dyn Clipboard>>>;

/// Réglages de l'image, modifiables en cours de session. Les bornes protègent la machine (et la connexion) d'un contrôleur trop gourmand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamSettings {
    pub fps: u32,
    pub quality: u8,
    pub width: u32,
    pub screen: usize,
}

impl StreamSettings {
    pub const FPS: (u32, u32) = (2, 30);
    pub const QUALITY: (u8, u8) = (20, 90);
    pub const WIDTH: (u32, u32) = (640, 3840);

    pub fn initial(screen: usize) -> Self {
        Self { fps: 10, quality: JPEG_QUALITY, width: MAX_WIDTH, screen }
    }

    /// Applique les valeurs demandées, bornées ; ce qui n'est pas demandé ne change pas.
    pub fn tuned(self, fps: Option<u32>, quality: Option<u8>, width: Option<u32>) -> Self {
        Self {
            fps: fps.map_or(self.fps, |v| v.clamp(Self::FPS.0, Self::FPS.1)),
            quality: quality.map_or(self.quality, |v| v.clamp(Self::QUALITY.0, Self::QUALITY.1)),
            width: width.map_or(self.width, |v| v.clamp(Self::WIDTH.0, Self::WIDTH.1)),
            screen: self.screen,
        }
    }
}

/// Messages du contrôleur qui ne sont pas de la saisie.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum Message {
    Ping { id: u64 },
    Stream { fps: Option<u32>, quality: Option<u8>, width: Option<u32> },
    Screen { index: usize },
    Clip { text: String },
}

/// Tout ce qu'une session partage entre ses canaux : droits, saisie, écrans, réglages d'image, presse-papiers.
pub struct Session {
    permissions: HashSet<Permission>,
    input: SharedInput,
    screens: Screens,
    clipboard: SharedClipboard,
    stream: Mutex<StreamSettings>,
    sync: Mutex<clipboard::Sync>,
    control: Mutex<Option<Arc<RTCDataChannel>>>,
    /// Passe à vrai dès que le contrôleur emploie un message récent (ping, stream, screen) : il comprend alors « stats ». Un contrôleur plus ancien
    /// (l'onglet d'ARIA, par exemple) prendrait ce message pour les informations de l'écran et fausserait son affichage : il n'en reçoit donc pas.
    stats_wanted: AtomicBool,
}

impl Session {
    pub fn new(permissions: HashSet<Permission>, input: SharedInput, screens: Screens, clipboard: SharedClipboard) -> Arc<Self> {
        let primary = screens.primary();
        let initial = clipboard.lock().ok().and_then(|mut c| c.get_text());
        let session = Arc::new(Self { permissions, input, screens, clipboard, stream: Mutex::new(StreamSettings::initial(primary)), sync: Mutex::new(clipboard::Sync::new(initial)), control: Mutex::new(None), stats_wanted: AtomicBool::new(false) });
        session.point_input_at(primary);
        session
    }

    fn list(&self) -> Vec<Screen> {
        (self.screens.list)()
    }

    /// La saisie vise l'écran choisi (sinon la souris atterrirait toujours sur l'écran principal).
    fn point_input_at(&self, index: usize) {
        if let (Some(screen), Ok(mut sink)) = (self.list().get(index).cloned(), self.input.lock()) {
            sink.set_area(Area { x: screen.x, y: screen.y, width: screen.width, height: screen.height });
        }
    }

    fn settings(&self) -> StreamSettings {
        self.stream.lock().map(|s| *s).unwrap_or(StreamSettings::initial(0))
    }

    /// Message « info » : taille de l'écran affiché, droits, liste des écrans et fonctions disponibles.
    pub fn info(&self) -> Value {
        let screens = self.list();
        let selected = self.settings().screen;
        let (width, height) = screens.get(selected).map(|s| (s.width, s.height)).or_else(|| self.input.lock().ok().map(|s| s.display_size())).unwrap_or((0, 0));
        let mut names: Vec<&str> = self.permissions.iter().map(|p| p.as_str()).collect();
        names.sort_unstable();
        let features: Vec<&str> = FEATURES.iter().copied().filter(|f| *f != "clipboard" || self.permissions.contains(&Permission::ControlKeyboard)).collect();
        json!({"t": "info", "width": width, "height": height, "permissions": names, "screens": screens, "screen": selected, "features": features})
    }

    /// Canal « pointer » : seuls les déplacements de souris y sont acceptés (un clic ou une touche doit passer par le canal fiable « control »).
    pub fn process_pointer(&self, data: &[u8]) {
        if matches!(serde_json::from_slice::<Control>(data), Ok(Control::Move { .. })) {
            let _ = self.process(data);
        }
    }

    /// Envoie un message JSON au contrôleur sur le canal « control » (s'il est ouvert).
    async fn send_control(&self, value: Value) {
        let channel = self.control.lock().ok().and_then(|c| c.clone());
        if let Some(channel) = channel {
            let _ = channel.send_text(value.to_string()).await;
        }
    }

    /// Traite un message du canal « control » et renvoie les réponses à envoyer. Aucune entrée/sortie réseau ici : tout est testable.
    pub fn process(&self, data: &[u8]) -> Vec<Value> {
        let denied = |reason: &str| vec![json!({"t": "denied", "reason": reason})];
        if let Ok(control) = serde_json::from_slice::<Control>(data) {
            return match self.input.lock() {
                Ok(mut sink) => match input::apply(&control, &self.permissions, sink.as_mut()) {
                    Ok(()) => vec![],
                    Err(reason) => denied(reason),
                },
                Err(_) => denied("saisie indisponible"),
            };
        }
        let Ok(message) = serde_json::from_slice::<Message>(data) else { return denied("commande illisible") };
        if matches!(message, Message::Ping { .. } | Message::Stream { .. } | Message::Screen { .. }) {
            self.stats_wanted.store(true, Ordering::Relaxed);
        }
        match message {
            Message::Ping { id } => vec![json!({"t": "pong", "id": id})],
            Message::Stream { fps, quality, width } => {
                if !self.permissions.contains(&Permission::ViewScreen) {
                    return denied("permission refusée");
                }
                if let Ok(mut stream) = self.stream.lock() {
                    *stream = stream.tuned(fps, quality, width);
                }
                vec![]
            }
            Message::Screen { index } => {
                if !self.permissions.contains(&Permission::ViewScreen) {
                    return denied("permission refusée");
                }
                if index >= self.list().len() {
                    return denied("écran inconnu");
                }
                if let Ok(mut stream) = self.stream.lock() {
                    stream.screen = index;
                }
                self.point_input_at(index);
                vec![self.info()]
            }
            Message::Clip { text } => {
                if !self.permissions.contains(&Permission::ControlKeyboard) {
                    return denied("permission refusée");
                }
                let (Ok(mut sync), Ok(mut clipboard)) = (self.sync.lock(), self.clipboard.lock()) else { return denied("presse-papiers indisponible") };
                match sync.apply(&text, clipboard.as_mut()) {
                    Ok(()) => vec![],
                    Err(reason) => denied(reason),
                }
            }
        }
    }

    /// Texte à envoyer au contrôleur si le presse-papiers de l'appareil a changé (permission clavier requise).
    fn clipboard_change(&self) -> Option<String> {
        if !self.permissions.contains(&Permission::ControlKeyboard) {
            return None;
        }
        let current = self.clipboard.lock().ok().and_then(|mut c| c.get_text());
        self.sync.lock().ok().and_then(|mut sync| sync.changed(current))
    }

    /// Position du pointeur rapportée à l'écran affiché (0..1) ; None si le pointeur est inconnu.
    fn cursor_position(&self) -> Option<(f64, f64)> {
        let sink = self.input.lock().ok()?;
        let (x, y) = sink.cursor()?;
        let area = sink.area();
        let relative = |value: i32, origin: i32, size: u32| (f64::from(value) - f64::from(origin)) / f64::from(size.max(1));
        Some((relative(x, area.x, area.width).clamp(0.0, 1.0), relative(y, area.y, area.height).clamp(0.0, 1.0)))
    }
}

/// Branche un canal de données ouvert par le contrôleur selon son nom ; un canal inconnu est ignoré.
pub fn attach_channel(channel: Arc<RTCDataChannel>, session: Arc<Session>) {
    match channel.label() {
        "frames" => {
            if session.permissions.contains(&Permission::ViewScreen) {
                let on_open = channel.clone();
                on_open.on_open(Box::new(move || {
                    start_frames(channel, session);
                    Box::pin(async {})
                }));
            }
        }
        "control" => attach_control(channel, session),
        "pointer" => channel.on_message(Box::new(move |message: DataChannelMessage| {
            session.process_pointer(&message.data);
            Box::pin(async {})
        })),
        other => eprintln!("[session] canal inconnu ignoré : {other}"),
    }
}

fn attach_control(channel: Arc<RTCDataChannel>, session: Arc<Session>) {
    let open_channel = channel.clone();
    let open_session = session.clone();
    channel.on_open(Box::new(move || {
        let channel = open_channel.clone();
        let session = open_session.clone();
        Box::pin(async move {
            if let Ok(mut holder) = session.control.lock() {
                *holder = Some(channel.clone());
            }
            let _ = channel.send_text(session.info().to_string()).await;
            tokio::spawn(share_cursor(channel.clone(), session.clone()));
            if session.permissions.contains(&Permission::ControlKeyboard) {
                tokio::spawn(share_clipboard(channel, session));
            }
        })
    }));

    let reply = channel.clone();
    channel.on_message(Box::new(move |message: DataChannelMessage| {
        let replies = session.process(&message.data);
        let reply = reply.clone();
        Box::pin(async move {
            for value in replies {
                if value["t"] == "denied" {
                    eprintln!("[session] commande refusée : {}", value["reason"]);
                }
                let _ = reply.send_text(value.to_string()).await;
            }
        })
    }));
}

/// Envoie la position du pointeur (normalisée 0..1) quand elle change : la capture d'écran ne contient pas le curseur, le contrôleur le dessine.
async fn share_cursor(channel: Arc<RTCDataChannel>, session: Arc<Session>) {
    let mut last = None;
    loop {
        tokio::time::sleep(Duration::from_millis(1000 / CURSOR_HZ)).await;
        if let Some((x, y)) = session.cursor_position() {
            let key = ((x * 10_000.0) as i64, (y * 10_000.0) as i64);
            if last != Some(key) {
                last = Some(key);
                if channel.send_text(json!({"t": "cursor", "x": x, "y": y}).to_string()).await.is_err() {
                    return;     // le canal est fermé : la session est terminée
                }
            }
        }
    }
}

/// Envoie le texte copié sur l'appareil au contrôleur (une fois par changement).
async fn share_clipboard(channel: Arc<RTCDataChannel>, session: Arc<Session>) {
    loop {
        tokio::time::sleep(CLIPBOARD_POLL).await;
        if let Some(text) = session.clipboard_change() {
            if channel.send_text(json!({"t": "clip", "text": text}).to_string()).await.is_err() {
                return;
            }
        }
    }
}

/// Flux d'images en trois étapes qui tournent en parallèle, reliées par des boîtes aux lettres « dernière valeur » :
///   capture (thread) -> encodage (thread) -> envoi (tâche tokio).
/// Une étape lente ne fait jamais grossir de file : la suivante prend toujours l'image la plus récente. On n'encode ni n'envoie un écran qui n'a pas
/// changé, et on s'arrête de capturer quand le canal d'envoi est saturé (inutile d'encoder des images qu'on jettera). Les réglages (images par
/// seconde, qualité, largeur, écran) sont relus à chaque image : un changement s'applique tout de suite.
fn start_frames(channel: Arc<RTCDataChannel>, session: Arc<Session>) {
    let raw: Arc<Slot<Frame>> = Slot::new();
    let jpegs: Arc<Slot<Vec<u8>>> = Slot::new();
    let counters = Arc::new(Counters::default());
    let pressure = Arc::new(AtomicBool::new(false));

    // 1. Capture : la source d'écran est créée dans ce thread et y reste.
    {
        let (raw, counters, pressure, session) = (raw.clone(), counters.clone(), pressure.clone(), session.clone());
        std::thread::spawn(move || {
            let mut current = session.settings().screen;
            let mut screen = match (session.screens.open)(current) {
                Ok(screen) => screen,
                Err(error) => {
                    eprintln!("[capture] écran indisponible : {error:#}");
                    raw.close();
                    return;
                }
            };
            let mut last: Option<(u64, u32, u32)> = None;
            let mut last_sent = Instant::now();
            while !raw.is_closed() {
                let started = Instant::now();
                let settings = session.settings();
                if settings.screen != current {
                    match (session.screens.open)(settings.screen) {
                        Ok(next) => { screen = next; current = settings.screen; last = None; }
                        Err(error) => {
                            eprintln!("[capture] changement d'écran impossible : {error:#}");
                            if let Ok(mut stream) = session.stream.lock() { stream.screen = current; }   // on reste sur l'écran qui marche
                        }
                    }
                }
                if pressure.load(Ordering::Relaxed) {
                    Counters::add(&counters.throttled, 1);        // canal saturé : on ne capture pas pour jeter
                } else {
                    match screen.capture() {
                        Ok(frame) => {
                            Counters::add(&counters.captured, 1);
                            Counters::add(&counters.capture_us, started.elapsed().as_micros() as u64);
                            let print = (fingerprint(&frame.rgba), frame.width, frame.height);
                            if last == Some(print) && last_sent.elapsed() < HEARTBEAT {
                                Counters::add(&counters.unchanged, 1);
                            } else {
                                last = Some(print);
                                last_sent = Instant::now();
                                raw.put(frame);
                            }
                        }
                        Err(error) => {
                            eprintln!("[capture] {error:#}");
                            std::thread::sleep(Duration::from_secs(1));
                        }
                    }
                }
                if let Some(rest) = Duration::from_millis(1000 / u64::from(settings.fps.max(1))).checked_sub(started.elapsed()) {
                    std::thread::sleep(rest);
                }
            }
            raw.close();
        });
    }

    // 2. Encodage : ne prend que la dernière image capturée.
    {
        let (raw, jpegs, counters, session) = (raw.clone(), jpegs.clone(), counters.clone(), session.clone());
        std::thread::spawn(move || {
            let mut encoder = JpegEncoder::new();
            while let Some(frame) = raw.take_blocking() {
                let settings = session.settings();
                let started = Instant::now();
                match encoder.encode(frame, settings.width, settings.quality) {
                    Ok(jpeg) => {
                        Counters::add(&counters.encode_us, started.elapsed().as_micros() as u64);
                        Counters::add(&counters.encoded, 1);
                        jpegs.put(jpeg);
                    }
                    Err(error) => eprintln!("[capture] {error:#}"),
                }
            }
            jpegs.close();
        });
    }

    // 3. Envoi : saute l'image si le canal est déjà chargé (le retard ne s'accumule jamais), et rapporte les mesures toutes les 2 s.
    tokio::spawn(async move {
        let mut governor = Governor::new();
        let mut frame_id = 0u32;
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        let mut window = Instant::now();
        loop {
            tokio::select! {
                next = jpegs.take_async() => {
                    let Some(jpeg) = next else { break };
                    if channel.buffered_amount().await > governor.limit() {
                        Counters::add(&counters.dropped, 1);
                        pressure.store(true, Ordering::Relaxed);
                        continue;
                    }
                    governor.observe(jpeg.len());
                    let mut failed = false;
                    for chunk in chunk_frame(frame_id, &jpeg) {
                        if channel.send(&chunk).await.is_err() { failed = true; break; }
                    }
                    if failed { break; }
                    frame_id = frame_id.wrapping_add(1);
                    Counters::add(&counters.sent, 1);
                    Counters::add(&counters.bytes, jpeg.len() as u64);
                }
                _ = tick.tick() => {
                    // Indépendamment des images : la pression retombe dès que le canal se vide (sinon la capture, arrêtée, ne reprendrait jamais).
                    pressure.store(channel.buffered_amount().await > governor.limit() / 2, Ordering::Relaxed);
                    if window.elapsed() >= STATS_EVERY {
                        let report = counters.report(window.elapsed().as_millis() as u64);
                        window = Instant::now();
                        if !session.stats_wanted.load(Ordering::Relaxed) { continue; }
                        session.send_control(json!({"t": "stats", "fps": report.fps, "kbps": report.kbps, "capture_ms": report.capture_ms, "encode_ms": report.encode_ms, "dropped": report.dropped, "idle": report.idle})).await;
                    }
                }
            }
        }
        raw.close();
        jpegs.close();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::screen::{synthetic_backend, synthetic_screens};
    use crate::clipboard::MemoryClipboard;
    use crate::input::LogInput;

    fn session(permissions: &[Permission], clipboard: Option<&str>) -> (Arc<Session>, SharedClipboard) {
        let input: SharedInput = Arc::new(Mutex::new(Box::new(LogInput::new())));
        let clip: SharedClipboard = Arc::new(Mutex::new(Box::new(MemoryClipboard(clipboard.map(String::from)))));
        (Session::new(permissions.iter().copied().collect(), input, synthetic_backend(), clip.clone()), clip)
    }

    const VIEW: [Permission; 1] = [Permission::ViewScreen];
    const ALL: [Permission; 3] = [Permission::ViewScreen, Permission::ControlMouse, Permission::ControlKeyboard];

    fn run(session: &Session, text: &str) -> Vec<Value> {
        session.process(text.as_bytes())
    }

    #[test]
    fn stream_settings_are_clamped_and_partial_updates_keep_the_rest() {
        let base = StreamSettings::initial(0);
        assert_eq!(base.tuned(Some(1000), Some(255), Some(99_999)), StreamSettings { fps: 30, quality: 90, width: 3840, screen: 0 });
        assert_eq!(base.tuned(Some(0), Some(0), Some(0)), StreamSettings { fps: 2, quality: 20, width: 640, screen: 0 });
        assert_eq!(base.tuned(Some(15), None, None), StreamSettings { fps: 15, ..base });
        assert_eq!(base.tuned(None, None, None), base);
    }

    #[test]
    fn a_ping_is_answered_for_any_session_and_the_id_is_echoed() {
        let (view, _) = session(&VIEW, None);
        assert_eq!(run(&view, r#"{"t":"ping","id":42}"#), vec![json!({"t": "pong", "id": 42})]);
        assert_eq!(run(&view, r#"{"t":"ping"}"#)[0]["t"], "denied", "un ping sans identifiant est illisible");
    }

    #[test]
    fn info_lists_the_screens_and_only_announces_the_clipboard_with_the_keyboard_permission() {
        let (view, _) = session(&VIEW, None);
        let info = view.info();
        assert_eq!((info["width"].as_u64(), info["height"].as_u64(), info["screen"].as_u64()), (Some(1280), Some(720), Some(0)));
        assert_eq!(info["screens"].as_array().unwrap().len(), 2);
        assert_eq!(info["screens"][1]["x"], 1280);
        assert_eq!(info["features"], json!(["ping", "stream", "screens", "pointer", "stats"]));
        let (all, _) = session(&ALL, None);
        assert_eq!(all.info()["features"], json!(["ping", "stream", "screens", "clipboard", "pointer", "stats"]));
        assert_eq!(all.info()["permissions"], json!(["control_keyboard", "control_mouse", "view_screen"]));
    }

    #[test]
    fn choosing_a_screen_changes_the_capture_the_info_and_where_the_mouse_lands() {
        let (all, _) = session(&ALL, None);
        let replies = run(&all, r#"{"t":"screen","index":1}"#);
        assert_eq!(replies.len(), 1);
        assert_eq!((replies[0]["t"].as_str(), replies[0]["width"].as_u64(), replies[0]["screen"].as_u64()), (Some("info"), Some(800), Some(1)));
        assert_eq!(all.settings().screen, 1);
        assert!(run(&all, r#"{"t":"move","x":0,"y":0}"#).is_empty());
        let area = all.input.lock().unwrap().area();
        assert_eq!(area, Area { x: 1280, y: 0, width: 800, height: 600 });
        assert_eq!(run(&all, r#"{"t":"screen","index":7}"#), vec![json!({"t": "denied", "reason": "écran inconnu"})]);
        assert_eq!(all.settings().screen, 1, "un écran inconnu ne change rien");
    }

    #[test]
    fn the_cursor_is_reported_relative_to_the_displayed_screen() {
        let (all, _) = session(&ALL, None);
        run(&all, r#"{"t":"screen","index":1}"#);
        run(&all, r#"{"t":"move","x":0.5,"y":0.5}"#);
        let (x, y) = all.cursor_position().unwrap();
        assert!((x - 0.5).abs() < 0.01 && (y - 0.5).abs() < 0.01, "{x},{y}");
        run(&all, r#"{"t":"screen","index":0}"#);
        let (x, _) = all.cursor_position().unwrap();
        assert_eq!(x, 1.0, "le pointeur est sur l'écran de droite : au bord de l'écran de gauche");
    }

    #[test]
    fn stream_and_screen_need_the_view_permission_and_clip_needs_the_keyboard() {
        let none: Session = {
            let input: SharedInput = Arc::new(Mutex::new(Box::new(LogInput::new())));
            let clip: SharedClipboard = Arc::new(Mutex::new(Box::new(MemoryClipboard(None))));
            Arc::try_unwrap(Session::new(HashSet::new(), input, synthetic_backend(), clip)).ok().unwrap()
        };
        for text in [r#"{"t":"stream","fps":5}"#, r#"{"t":"screen","index":1}"#, r#"{"t":"clip","text":"x"}"#] {
            assert_eq!(run(&none, text), vec![json!({"t": "denied", "reason": "permission refusée"})], "{text}");
        }
        let (mouse_only, clip) = session(&[Permission::ViewScreen, Permission::ControlMouse], Some("avant"));
        assert_eq!(run(&mouse_only, r#"{"t":"clip","text":"intrus"}"#)[0]["reason"], "permission refusée");
        assert_eq!(clip.lock().unwrap().get_text().as_deref(), Some("avant"), "le presse-papiers n'est pas touché sans la permission clavier");
        assert_eq!(mouse_only.clipboard_change(), None, "et rien n'est lu non plus");
    }

    #[test]
    fn stats_are_only_wanted_by_controllers_that_speak_the_recent_messages() {
        let (view, _) = session(&VIEW, None);
        assert!(!view.stats_wanted.load(Ordering::Relaxed), "un contrôleur qui n'a rien dit de récent ne reçoit pas de statistiques");
        run(&view, r#"{"t":"move","x":0.5,"y":0.5}"#);
        run(&view, "pas du json");
        assert!(!view.stats_wanted.load(Ordering::Relaxed));
        run(&view, r#"{"t":"ping","id":1}"#);
        assert!(view.stats_wanted.load(Ordering::Relaxed));
        let (other, _) = session(&VIEW, None);
        run(&other, r#"{"t":"stream","fps":5}"#);
        assert!(other.stats_wanted.load(Ordering::Relaxed));
    }

    #[test]
    fn stream_updates_apply_with_the_view_permission() {
        let (view, _) = session(&VIEW, None);
        assert!(run(&view, r#"{"t":"stream","fps":20,"quality":80}"#).is_empty());
        assert_eq!(view.settings(), StreamSettings { fps: 20, quality: 80, width: 1600, screen: 0 });
        assert!(run(&view, r#"{"t":"stream","fps":"beaucoup"}"#)[0]["t"] == "denied");
    }

    #[test]
    fn clipboard_text_goes_both_ways_without_echo() {
        let (all, clip) = session(&ALL, Some("déjà là"));
        assert_eq!(all.clipboard_change(), None, "le contenu d'avant la session n'est pas partagé");
        assert!(run(&all, r#"{"t":"clip","text":"venu du contrôleur"}"#).is_empty());
        assert_eq!(clip.lock().unwrap().get_text().as_deref(), Some("venu du contrôleur"));
        assert_eq!(all.clipboard_change(), None, "pas d'écho vers le contrôleur");
        clip.lock().unwrap().set_text("copié sur l'appareil");
        assert_eq!(all.clipboard_change().as_deref(), Some("copié sur l'appareil"));
        assert_eq!(all.clipboard_change(), None);
        let big = "x".repeat(clipboard::MAX_BYTES + 1);
        assert_eq!(run(&all, &json!({"t": "clip", "text": big}).to_string())[0]["reason"], "texte trop gros pour le presse-papiers");
    }

    #[test]
    fn input_and_unknown_messages_behave_as_before() {
        let (view, _) = session(&VIEW, None);
        assert_eq!(run(&view, r#"{"t":"move","x":0.5,"y":0.5}"#), vec![json!({"t": "denied", "reason": "permission refusée"})]);
        assert_eq!(run(&view, r#"{"t":"shell","cmd":"del *"}"#), vec![json!({"t": "denied", "reason": "commande illisible"})]);
        assert_eq!(run(&view, "pas du json"), vec![json!({"t": "denied", "reason": "commande illisible"})]);
        let (all, _) = session(&ALL, None);
        assert!(run(&all, r#"{"t":"move","x":0.5,"y":0.5}"#).is_empty());
    }

    #[test]
    fn the_pointer_channel_only_moves_the_mouse_and_still_needs_the_permission() {
        let (all, _) = session(&ALL, None);
        all.process_pointer(br#"{"t":"move","x":0.5,"y":0.5}"#);
        assert_eq!(all.cursor_position().map(|(x, _)| (x * 100.0).round()), Some(50.0));
        all.process_pointer(br#"{"t":"down","x":0.9,"y":0.9,"button":0}"#);
        all.process_pointer(br#"{"t":"key","down":true,"code":"KeyA","key":"a"}"#);
        all.process_pointer(br#"{"t":"clip","text":"intrus"}"#);
        all.process_pointer(b"pas du json");
        assert_eq!(all.cursor_position().map(|(x, _)| (x * 100.0).round()), Some(50.0), "ni clic ni touche ne passent par ce canal non fiable");
        let (view, _) = session(&VIEW, None);
        view.process_pointer(br#"{"t":"move","x":0.9,"y":0.9}"#);
        assert_eq!(view.cursor_position(), None, "sans la permission souris, rien ne bouge");
    }

    #[test]
    fn the_synthetic_screens_are_the_two_test_screens() {
        assert_eq!(synthetic_screens().len(), 2);
        assert_eq!((synthetic_backend().list)().len(), 2);
        assert_eq!(synthetic_backend().primary(), 0);
        assert!((synthetic_backend().open)(1).is_ok() && (synthetic_backend().open)(2).is_err());
    }
}
