//! L'agent : garde la connexion au backend, demande le consentement de la personne devant l'appareil, puis ouvre les sessions.
//! Cycle : appairage (code affiché dans la fenêtre, saisi dans PC Assistant) -> WebSocket authentifié par la clé -> sessions acceptées une à une.
//! La fenêtre (ui.rs) affiche l'état et envoie ses ordres par `UiCommand` ; la console reste utilisable (« stop », « quit »).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use tokio::sync::mpsc;
use tokio::time::sleep;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;

use crate::config::{self, Config};
use crate::identity::{agent_auth_message, answer_message, Identity};
use crate::input::{InputSink, Permission};
use crate::network::signaling::{self, ClientMessage, IceServerConfig, ServerMessage, Socket};
use crate::network::webrtc::Peer;
use crate::session::{attach_channel, SharedInput, SourceFactory};
use crate::ui::{now_ms, ConsentInfo, SessionInfo, Ui, UiCommand};

/// Le serveur abandonne une demande restée sans réponse au bout de 60 s : au-delà, on la retire aussi ici.
const CONSENT_TIMEOUT: Duration = Duration::from_secs(65);

pub struct Options {
    pub server: String,
    pub api_key: String,
    pub allow: HashSet<Permission>,
    pub auto_accept: bool,
    pub source: SourceFactory,
    pub input: Box<dyn Fn() -> Result<Box<dyn InputSink>> + Send + Sync>,
    pub ui: Option<Ui>,
    pub config_path: PathBuf,
}

struct Plan {
    permissions: HashSet<Permission>,
    ice: Vec<IceServerConfig>,
    accepted: bool,
    decided: bool,
    asked_at: Instant,
}

struct Active {
    session_id: String,
    plan: Plan,
    peer: Option<Peer>,
}

enum Event {
    Consent { session_id: String, accepted: bool },
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    NotPaired,
    Refused,
    Closed,
    Quit,
}

/// Suite à donner après un ordre de la fenêtre ou de la console.
#[derive(Debug, PartialEq, Clone, Copy)]
enum Flow {
    Continue,
    Reconnect,
    Quit,
}

pub struct Agent {
    options: Options,
    config: Config,
    identity: Identity,
    commands: mpsc::UnboundedReceiver<UiCommand>,
    requests: HashMap<String, Plan>,
    active: Option<Active>,
    awaiting_console: Option<String>,
    input: Option<SharedInput>,
    pairing_until: Option<Instant>,
}

type Out = mpsc::UnboundedSender<ClientMessage>;

fn permission_names(set: &HashSet<Permission>) -> Vec<String> {
    let mut names: Vec<String> = set.iter().map(|p| p.as_str().to_string()).collect();
    names.sort_unstable();
    names
}

impl Agent {
    pub fn new(options: Options, config: Config, identity: Identity, commands: mpsc::UnboundedReceiver<UiCommand>) -> Self {
        Self { options, config, identity, commands, requests: HashMap::new(), active: None, awaiting_console: None, input: None, pairing_until: None }
    }

    fn shared_input(&mut self) -> Result<SharedInput> {
        if self.input.is_none() {
            self.input = Some(Arc::new(Mutex::new((self.options.input)()?)));
        }
        Ok(self.input.clone().expect("créé juste au-dessus"))
    }

    // -- fenêtre -------------------------------------------------------------------------------------------------------------
    fn ui(&self, change: impl FnOnce(&mut crate::ui::UiState)) {
        if let Some(ui) = &self.options.ui {
            ui.update(change);
        }
    }

    fn log(&self, line: impl Into<String>) {
        let line = line.into();
        println!("[agent] {line}");
        if let Some(ui) = &self.options.ui {
            ui.log(line);
        }
    }

    fn set_link(&self, link: &str, detail: impl Into<String>) {
        let (link, detail) = (link.to_string(), detail.into());
        if link == "error" {
            eprintln!("[agent] {detail}");
        }
        self.ui(move |s| {
            s.link = link;
            s.link_detail = detail;
        });
    }

    fn publish_allow(&self) {
        let (mouse, keyboard) = (self.options.allow.contains(&Permission::ControlMouse), self.options.allow.contains(&Permission::ControlKeyboard));
        self.ui(move |s| {
            s.allow_mouse = mouse;
            s.allow_keyboard = keyboard;
        });
    }

    fn save_config(&self) {
        if let Err(error) = config::save(&self.options.config_path, &self.config) {
            self.log(format!("configuration non sauvegardée : {error:#}"));
        }
    }

    // -- boucle principale ---------------------------------------------------------------------------------------------------
    /// Ne rend la main que sur « quit » (console ou fenêtre).
    pub async fn run(&mut self) -> Result<()> {
        let (line_tx, mut lines) = mpsc::unbounded_channel::<String>();
        std::thread::spawn(move || {
            let mut buffer = String::new();
            while std::io::stdin().read_line(&mut buffer).map(|n| n > 0).unwrap_or(false) {
                if line_tx.send(buffer.trim().to_string()).is_err() {
                    break;
                }
                buffer.clear();
            }
        });
        self.publish_allow();

        // Premier lancement avec la fenêtre : on attend que l'adresse de PC Assistant soit saisie.
        while self.options.server.is_empty() {
            self.set_link("setup", "");
            match self.commands.recv().await {
                Some(command) => {
                    if self.handle_command(command, None).await == Flow::Quit {
                        return Ok(());
                    }
                }
                None => bail!("aucune adresse de PC Assistant et aucune fenêtre pour la saisir"),
            }
        }

        loop {
            // Tant qu'un code d'appairage est affiché, chaque nouvelle tentative de connexion (toutes les 3 s) ne doit pas le faire clignoter.
            if self.pairing_until.map_or(true, |until| Instant::now() >= until) {
                self.set_link("connecting", "");
            }
            match signaling::connect(&self.options.server, &self.options.api_key, &self.config.device_id).await {
                Ok(socket) => match self.connection(socket, &mut lines).await {
                    Ok(Outcome::Quit) => return Ok(()),
                    Ok(Outcome::NotPaired) => self.show_pairing_code().await,
                    Ok(Outcome::Refused) => self.set_link("error", "PC Assistant a refusé l'identité de cet appareil. Supprime-le de la liste des appareils et recommence l'appairage."),
                    Ok(Outcome::Closed) => self.set_link("connecting", ""),
                    Err(error) => self.set_link("error", format!("{error:#}")),
                },
                Err(error) => self.set_link("error", format!("{error:#}")),
            }
            match self.pause(Duration::from_secs(3)).await {
                Flow::Quit => return Ok(()),
                Flow::Reconnect => self.pairing_until = None,
                Flow::Continue => {}
            }
        }
    }

    /// Appareil pas encore appairé : demande (ou renouvelle) un code et l'affiche dans la fenêtre et la console.
    async fn show_pairing_code(&mut self) {
        if self.pairing_until.map_or(true, |until| Instant::now() >= until) {
            match signaling::register(&self.options.server, &self.options.api_key, &self.config, &self.identity).await {
                Ok((code, ttl)) => {
                    self.pairing_until = Some(Instant::now() + Duration::from_secs(ttl.saturating_sub(10)));
                    println!("\n  Code d'appairage : {code}\n  Saisis-le dans PC Assistant (Ordinateur > Maintenance à distance). Il vaut {} minutes.\n", ttl / 60);
                    let expires = now_ms() + ttl * 1000;
                    self.ui(move |s| {
                        s.pairing_code = Some(code);
                        s.pairing_expires_ms = Some(expires);
                    });
                    self.set_link("unpaired", "");
                    self.log("en attente de l'appairage : saisis le code dans PC Assistant");
                }
                Err(error) => self.set_link("error", format!("{error:#}")),
            }
        } else {
            self.set_link("unpaired", "");
        }
    }

    /// Attente entre deux tentatives de connexion, sans rater un ordre de la fenêtre.
    async fn pause(&mut self, duration: Duration) -> Flow {
        let timer = sleep(duration);
        tokio::pin!(timer);
        loop {
            tokio::select! {
                _ = &mut timer => return Flow::Continue,
                Some(command) = self.commands.recv() => match self.handle_command(command, None).await {
                    Flow::Continue => {}
                    other => return other,
                },
            }
        }
    }

    async fn connection(&mut self, mut socket: Socket, lines: &mut mpsc::UnboundedReceiver<String>) -> Result<Outcome> {
        // 1. Défi : prouver qu'on détient la clé privée de l'appareil. Un appareil pas encore appairé est refusé avant même le défi.
        // Derrière certains relais HTTPS, la fermeture « appareil inconnu » met ~20 s à arriver : sans réponse en 8 s on demande déjà un code
        // (sans risque : demander un code à un appareil déjà appairé ne le désappaire pas).
        let first = tokio::time::timeout(Duration::from_secs(8), signaling::receive(&mut socket)).await.unwrap_or(None);
        let nonce = match first {
            Some(Ok(ServerMessage::Challenge { nonce })) => nonce,
            None => return Ok(Outcome::NotPaired),
            // Derrière un relais HTTPS (Cloudflare, proxy…), la fermeture « appareil inconnu » du serveur arrive parfois comme une coupure
            // sèche au lieu d'une trame de fermeture : même sens, on demande un code. Si le serveur est vraiment en panne, l'inscription échoue
            // et l'erreur s'affiche.
            Some(Err(_)) => return Ok(Outcome::NotPaired),
            other => bail!("réponse inattendue du serveur : {other:?}"),
        };
        let signature = self.identity.sign(&agent_auth_message(&self.config.device_id, &nonce));
        signaling::send(&mut socket, &ClientMessage::Auth { signature }).await?;
        match signaling::receive(&mut socket).await {
            Some(Ok(ServerMessage::Ready { .. })) => {}
            None => return Ok(Outcome::Refused),
            other => bail!("réponse inattendue du serveur : {other:?}"),
        }
        let (out, mut outgoing) = mpsc::unbounded_channel::<ClientMessage>();
        let _ = out.send(self.grant_message());
        println!("[agent] connecté à PC Assistant — « stop » coupe la session en cours, « quit » arrête l'agent.");
        self.pairing_until = None;
        self.ui(|s| {
            s.pairing_code = None;
            s.pairing_expires_ms = None;
        });
        self.set_link("online", "");
        self.log("connecté à PC Assistant");

        // Une session encore vivante après une coupure du serveur est reprise sans renégocier.
        if let Some(active) = &self.active {
            if active.peer.as_ref().is_some_and(|peer| peer.is_connected()) {
                let _ = out.send(ClientMessage::Resume { session_id: active.session_id.clone() });
            }
        }
        let (events_tx, mut events) = mpsc::unbounded_channel::<Event>();
        let mut keepalive = tokio::time::interval(Duration::from_secs(20));
        let mut housekeeping = tokio::time::interval(Duration::from_secs(1));

        loop {
            tokio::select! {
                message = signaling::receive(&mut socket) => match message {
                    None => return Ok(Outcome::Closed),
                    Some(Err(error)) => eprintln!("[agent] {error:#}"),
                    Some(Ok(message)) => self.on_server(message, &out, &events_tx).await,
                },
                Some(message) = outgoing.recv() => signaling::send(&mut socket, &message).await?,
                Some(Event::Consent { session_id, accepted }) = events.recv() => self.on_consent(session_id, accepted, &out),
                Some(command) = self.commands.recv() => match self.handle_command(command, Some(&out)).await {
                    Flow::Continue => {}
                    Flow::Reconnect => return Ok(Outcome::Closed),
                    Flow::Quit => return Ok(Outcome::Quit),
                },
                Some(line) = lines.recv() => {
                    if self.on_line(&line, &out, &events_tx).await { return Ok(Outcome::Quit); }
                },
                _ = keepalive.tick() => signaling::send(&mut socket, &ClientMessage::Ping).await?,
                _ = housekeeping.tick() => self.expire_requests(),
            }
        }
    }

    fn grant_message(&self) -> ClientMessage {
        ClientMessage::Grant { permissions: permission_names(&self.options.allow) }
    }

    // -- ordres de la fenêtre ------------------------------------------------------------------------------------------------
    async fn handle_command(&mut self, command: UiCommand, out: Option<&Out>) -> Flow {
        match command {
            UiCommand::Consent { session_id, accept } => {
                if let Some(out) = out {
                    self.on_consent(session_id, accept, out);
                }
            }
            UiCommand::StopSession => self.stop_active(out).await,
            UiCommand::SetAllow { mouse, keyboard } => {
                let mut allow = HashSet::from([Permission::ViewScreen]);
                if mouse { allow.insert(Permission::ControlMouse); }
                if keyboard { allow.insert(Permission::ControlKeyboard); }
                self.options.allow = allow;
                let saved: Vec<&str> = [(mouse, "mouse"), (keyboard, "keyboard")].into_iter().filter(|(on, _)| *on).map(|(_, name)| name).collect();
                self.config.allow = Some(saved.join(","));
                self.save_config();
                self.publish_allow();
                self.log(format!("permissions modifiées : souris {}, clavier {}", if mouse { "oui" } else { "non" }, if keyboard { "oui" } else { "non" }));
                if let Some(out) = out {
                    let _ = out.send(self.grant_message());
                }
            }
            UiCommand::Configure { server, api_key } => match config::normalize_server(&server) {
                Some(server) => {
                    self.options.server = server.clone();
                    self.options.api_key = api_key.trim().to_string();
                    self.config.server = Some(server.clone());
                    self.config.api_key = Some(self.options.api_key.clone());
                    self.save_config();
                    self.ui(move |s| s.server = server);
                    self.log("adresse de PC Assistant enregistrée");
                    return Flow::Reconnect;
                }
                None => self.log("adresse invalide : saisis par exemple 192.168.1.20:8000"),
            },
            UiCommand::Quit => {
                self.stop_active(out).await;
                return Flow::Quit;
            }
        }
        Flow::Continue
    }

    async fn stop_active(&mut self, out: Option<&Out>) {
        match self.active.take() {
            Some(active) => {
                self.log("session coupée par l'utilisateur de cet appareil");
                if let Some(out) = out {
                    let _ = out.send(ClientMessage::Bye { session_id: active.session_id });
                }
                if let Some(peer) = active.peer {
                    peer.close().await;
                }
                self.ui(|s| s.session = None);
            }
            None => self.log("aucune session en cours"),
        }
    }

    // -- messages du serveur -------------------------------------------------------------------------------------------------
    async fn on_server(&mut self, message: ServerMessage, out: &Out, events: &mpsc::UnboundedSender<Event>) {
        match message {
            ServerMessage::OpenSession { session_id, permissions, ice_servers } => self.on_open_session(session_id, permissions, ice_servers, out, events),
            ServerMessage::Offer { session_id, sdp } => {
                if let Err(error) = self.on_offer(&session_id, sdp, out).await {
                    self.log(format!("négociation impossible : {error:#}"));
                }
            }
            ServerMessage::Ice { session_id, candidate } => {
                if let Some(peer) = self.active.as_ref().filter(|a| a.session_id == session_id).and_then(|a| a.peer.as_ref()) {
                    if let Err(error) = peer.add_ice(&candidate).await {
                        eprintln!("[agent] {error:#}");
                    }
                }
            }
            ServerMessage::Reconnect { session_id } => {
                if let Some(active) = self.active.as_mut().filter(|a| a.session_id == session_id) {
                    if let Some(peer) = active.peer.take() {
                        peer.close().await;
                    }
                    self.log("lien perdu : en attente d'une nouvelle négociation");
                    self.set_session_state("disconnected");
                }
            }
            ServerMessage::CloseSession { session_id } => self.end_session(&session_id).await,
            ServerMessage::Error { detail } => self.log(format!("le serveur signale : {detail}")),
            ServerMessage::Challenge { .. } | ServerMessage::Ready { .. } => {}
        }
    }

    fn set_session_state(&self, state: &str) {
        let state = state.to_string();
        self.ui(move |s| {
            if let Some(session) = &mut s.session {
                session.state = state;
            }
        });
    }

    fn on_open_session(&mut self, session_id: String, permissions: Vec<String>, ice: Vec<IceServerConfig>, out: &Out, events: &mpsc::UnboundedSender<Event>) {
        let refuse = |reason: &str| {
            let _ = out.send(ClientMessage::SessionReply { session_id: session_id.clone(), accepted: false, reason: reason.to_string() });
        };
        let parsed: Option<HashSet<Permission>> = permissions.iter().map(|p| Permission::parse(p)).collect();
        let Some(requested) = parsed.filter(|set| !set.is_empty()) else { return refuse("Permissions demandées invalides.") };
        if self.active.is_some() || !self.requests.is_empty() {
            return refuse("Une session est déjà en cours sur cet appareil.");
        }
        if !requested.is_subset(&self.options.allow) {
            return refuse("Cet appareil n'autorise pas toutes les permissions demandées.");
        }
        let names = permission_names(&requested);
        self.requests.insert(session_id.clone(), Plan { permissions: requested, ice, accepted: false, decided: false, asked_at: Instant::now() });
        if self.options.auto_accept {
            self.log(format!("ATTENTION : --auto-accept, session acceptée sans demander ({}).", names.join(", ")));
            let _ = events.send(Event::Consent { session_id, accepted: true });
            return;
        }
        self.log(format!("demande de prise de contrôle ({})", names.join(", ")));
        // La demande apparaît TOUJOURS dans la fenêtre. Si la fenêtre n'est pas visible (réduite, fermée), une fenêtre système s'ouvre aussi :
        // c'est la première réponse qui compte.
        let ui_ready = self.options.ui.as_ref().is_some_and(|ui| ui.window_visible());
        let info = ConsentInfo { session_id: session_id.clone(), permissions: names.clone(), since_ms: now_ms() };
        self.ui(move |s| s.consent = Some(info));
        if ui_ready {
            return;
        }
        let question = format!("Un contrôleur demande la connexion à « {} » avec : {}.", self.config.name, names.join(", "));
        #[cfg(windows)]
        {
            let events = events.clone();
            std::thread::spawn(move || {
                let accepted = rfd::MessageDialog::new()
                    .set_title("PC Assistant — contrôle à distance")
                    .set_description(format!("{question}\n\nAutoriser ?"))
                    .set_level(rfd::MessageLevel::Warning)
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show()
                    == rfd::MessageDialogResult::Yes;
                let _ = events.send(Event::Consent { session_id, accepted });
            });
        }
        #[cfg(not(windows))]
        {
            let _ = events;
            println!("\n  {question}\n  Autoriser ? [o/N]");
            self.awaiting_console = Some(session_id);
        }
    }

    /// Première réponse gagnante : une seconde (fenêtre système puis fenêtre de l'agent) est ignorée.
    fn on_consent(&mut self, session_id: String, accepted: bool, out: &Out) {
        let Some(plan) = self.requests.get_mut(&session_id) else { return };
        if plan.decided {
            return;
        }
        plan.decided = true;
        plan.accepted = accepted;
        if !accepted {
            self.requests.remove(&session_id);
        }
        self.ui(|s| s.consent = None);
        self.awaiting_console = None;
        self.log(if accepted { "prise de contrôle autorisée" } else { "prise de contrôle refusée" });
        let reason = if accepted { "" } else { "L'utilisateur de l'appareil a refusé la connexion." };
        let _ = out.send(ClientMessage::SessionReply { session_id, accepted, reason: reason.to_string() });
    }

    /// Demande restée sans réponse : le serveur l'a abandonnée, on la retire (sinon l'appareil refuserait toute nouvelle session).
    fn expire_requests(&mut self) {
        let stale: Vec<String> = self.requests.iter().filter(|(_, plan)| plan.asked_at.elapsed() > CONSENT_TIMEOUT).map(|(id, _)| id.clone()).collect();
        for id in stale {
            self.requests.remove(&id);
            self.log("demande sans réponse : abandonnée");
        }
        if self.requests.is_empty() {
            self.awaiting_console = None;
            self.ui(|s| s.consent = None);
        }
    }

    /// Commandes tapées dans la console de l'agent. Renvoie true pour arrêter l'agent.
    async fn on_line(&mut self, line: &str, out: &Out, events: &mpsc::UnboundedSender<Event>) -> bool {
        if let Some(session_id) = self.awaiting_console.take() {
            let accepted = matches!(line.to_lowercase().as_str(), "o" | "oui" | "y" | "yes");
            let _ = events.send(Event::Consent { session_id, accepted });
            return false;
        }
        match line.to_lowercase().as_str() {
            "quit" | "exit" | "q" => {
                self.stop_active(Some(out)).await;
                return true;
            }
            "stop" => self.stop_active(Some(out)).await,
            "" => {}
            _ => println!("[agent] commandes : stop (couper la session), quit (arrêter l'agent)"),
        }
        false
    }

    async fn on_offer(&mut self, session_id: &str, sdp: String, out: &Out) -> Result<()> {
        let mut active = match self.active.take() {
            Some(active) if active.session_id == session_id => active, // renégociation après une coupure : mêmes permissions
            other => {
                self.active = other;
                let Some(plan) = self.requests.remove(session_id).filter(|plan| plan.accepted) else {
                    bail!("offre reçue pour une session non acceptée ({session_id})");
                };
                Active { session_id: session_id.to_string(), plan, peer: None }
            }
        };
        if let Some(old) = active.peer.take() {
            old.close().await;
        }

        let input = self.shared_input()?;
        let permissions = Arc::new(active.plan.permissions.clone());
        let source = self.options.source.clone();
        let on_channel: Arc<dyn Fn(Arc<webrtc::data_channel::RTCDataChannel>) + Send + Sync> =
            Arc::new(move |channel| attach_channel(channel, permissions.clone(), input.clone(), source.clone()));
        let ui = self.options.ui.clone();
        let on_state: Arc<dyn Fn(RTCPeerConnectionState) + Send + Sync> = Arc::new(move |state| {
            if let Some(ui) = &ui {
                let label = match state {
                    RTCPeerConnectionState::Connected => "connected",
                    RTCPeerConnectionState::Disconnected => "disconnected",
                    RTCPeerConnectionState::Failed => "failed",
                    RTCPeerConnectionState::Closed => "closed",
                    _ => "connecting",
                };
                ui.update(|s| {
                    if let Some(session) = &mut s.session {
                        session.state = label.to_string();
                    }
                });
            }
        });
        let peer = Peer::new(&active.plan.ice, session_id.to_string(), out.clone(), on_channel, on_state).await?;
        let answer = peer.answer(sdp.clone()).await?;
        // Signature de la réponse sur CETTE offre : seul le détenteur de la clé privée de l'appareil peut répondre au nom de cet appareil.
        let signature = self.identity.sign(&answer_message(session_id, &sdp, &answer));
        let _ = out.send(ClientMessage::Answer { session_id: session_id.to_string(), sdp: answer, signature });
        let info = SessionInfo {
            session_id: session_id.to_string(),
            permissions: permission_names(&active.plan.permissions),
            since_ms: now_ms(),
            state: "connecting".into(),
        };
        active.peer = Some(peer);
        self.active = Some(active);
        self.ui(move |s| s.session = Some(info));
        self.log(format!("session {session_id} : négociation envoyée"));
        Ok(())
    }

    async fn end_session(&mut self, session_id: &str) {
        self.requests.remove(session_id);
        if self.requests.is_empty() {
            self.ui(|s| s.consent = None);
        }
        if self.active.as_ref().is_some_and(|a| a.session_id == session_id) {
            if let Some(active) = self.active.take() {
                if let Some(peer) = active.peer {
                    peer.close().await;
                }
            }
            self.ui(|s| s.session = None);
            self.log("session terminée");
        }
    }
}
