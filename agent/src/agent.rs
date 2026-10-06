//! L'agent : garde la connexion au backend, demande le consentement de la personne devant l'appareil, puis ouvre les sessions.
//! Cycle : appairage (code affiché ici, saisi dans PC Assistant) -> WebSocket authentifié par la clé -> sessions acceptées une à une.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use tokio::sync::mpsc;
use tokio::time::sleep;

use crate::config::Config;
use crate::identity::{agent_auth_message, answer_message, Identity};
use crate::input::{InputSink, Permission};
use crate::network::signaling::{self, ClientMessage, IceServerConfig, ServerMessage, Socket};
use crate::network::webrtc::Peer;
use crate::session::{attach_channel, SharedInput, SourceFactory};

pub struct Options {
    pub server: String,
    pub api_key: String,
    pub allow: HashSet<Permission>,
    pub auto_accept: bool,
    pub source: SourceFactory,
    pub input: Box<dyn Fn() -> Result<Box<dyn InputSink>> + Send + Sync>,
}

struct Plan {
    permissions: HashSet<Permission>,
    ice: Vec<IceServerConfig>,
    accepted: bool,
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

pub struct Agent {
    options: Options,
    config: Config,
    identity: Identity,
    requests: HashMap<String, Plan>,
    active: Option<Active>,
    awaiting_console: Option<String>,
    input: Option<SharedInput>,
}

impl Agent {
    pub fn new(options: Options, config: Config, identity: Identity) -> Self {
        Self { options, config, identity, requests: HashMap::new(), active: None, awaiting_console: None, input: None }
    }

    fn shared_input(&mut self) -> Result<SharedInput> {
        if self.input.is_none() {
            self.input = Some(Arc::new(Mutex::new((self.options.input)()?)));
        }
        Ok(self.input.clone().expect("créé juste au-dessus"))
    }

    /// Boucle principale : ne rend la main que sur « quit ».
    pub async fn run(&mut self) -> Result<()> {
        let (line_tx, mut lines) = mpsc::unbounded_channel::<String>();
        std::thread::spawn(move || {
            let mut buffer = String::new();
            while std::io::stdin().read_line(&mut buffer).map(|n| n > 0).unwrap_or(false) {
                if line_tx.send(buffer.trim().to_string()).is_err() { break; }
                buffer.clear();
            }
        });
        let mut code_valid_until: Option<Instant> = None;
        loop {
            match signaling::connect(&self.options.server, &self.options.api_key, &self.config.device_id).await {
                Ok(socket) => match self.connection(socket, &mut lines).await {
                    Ok(Outcome::Quit) => return Ok(()),
                    Ok(Outcome::NotPaired) => {
                        if code_valid_until.map_or(true, |until| Instant::now() >= until) {
                            match signaling::register(&self.options.server, &self.options.api_key, &self.config, &self.identity).await {
                                Ok((code, ttl)) => {
                                    code_valid_until = Some(Instant::now() + Duration::from_secs(ttl.saturating_sub(10)));
                                    println!("\n  Code d'appairage : {code}\n  Saisis-le dans PC Assistant (Ordinateur > Maintenance à distance). Il vaut {} minutes.\n", ttl / 60);
                                }
                                Err(error) => eprintln!("[agent] {error:#}"),
                            }
                        }
                    }
                    Ok(Outcome::Refused) => eprintln!("[agent] authentification refusée par le serveur : l'identité de l'appareil ne correspond pas."),
                    Ok(Outcome::Closed) => eprintln!("[agent] connexion fermée, nouvelle tentative…"),
                    Err(error) => eprintln!("[agent] {error:#}"),
                },
                Err(error) => eprintln!("[agent] {error:#}"),
            }
            sleep(Duration::from_secs(3)).await;
        }
    }

    async fn connection(&mut self, mut socket: Socket, lines: &mut mpsc::UnboundedReceiver<String>) -> Result<Outcome> {
        // 1. Défi : prouver qu'on détient la clé privée de l'appareil. Un appareil pas encore appairé est refusé avant même le défi.
        let nonce = match signaling::receive(&mut socket).await {
            Some(Ok(ServerMessage::Challenge { nonce })) => nonce,
            None => return Ok(Outcome::NotPaired),
            other => bail!("réponse inattendue du serveur : {other:?}"),
        };
        let signature = self.identity.sign(&agent_auth_message(&self.config.device_id, &nonce));
        signaling::send(&mut socket, &ClientMessage::Auth { signature }).await?;
        match signaling::receive(&mut socket).await {
            Some(Ok(ServerMessage::Ready { .. })) => {}
            None => return Ok(Outcome::Refused),
            other => bail!("réponse inattendue du serveur : {other:?}"),
        }
        let mut granted: Vec<&str> = self.options.allow.iter().map(|p| p.as_str()).collect();
        granted.sort_unstable();
        signaling::send(&mut socket, &ClientMessage::Grant { permissions: granted.iter().map(|s| s.to_string()).collect() }).await?;
        println!("[agent] connecté à PC Assistant — « stop » coupe la session en cours, « quit » arrête l'agent.");

        // Une session encore vivante après une coupure du serveur est reprise sans renégocier.
        let (out, mut outgoing) = mpsc::unbounded_channel::<ClientMessage>();
        if let Some(active) = &self.active {
            if active.peer.as_ref().is_some_and(|peer| peer.is_connected()) {
                let _ = out.send(ClientMessage::Resume { session_id: active.session_id.clone() });
            }
        }
        let (events_tx, mut events) = mpsc::unbounded_channel::<Event>();
        let mut keepalive = tokio::time::interval(Duration::from_secs(20));

        loop {
            tokio::select! {
                message = signaling::receive(&mut socket) => match message {
                    None => return Ok(Outcome::Closed),
                    Some(Err(error)) => eprintln!("[agent] {error:#}"),
                    Some(Ok(message)) => self.on_server(message, &out, &events_tx).await,
                },
                Some(message) = outgoing.recv() => signaling::send(&mut socket, &message).await?,
                Some(Event::Consent { session_id, accepted }) = events.recv() => self.on_consent(session_id, accepted, &out),
                Some(line) = lines.recv() => {
                    if self.on_line(&line, &out, &events_tx).await { return Ok(Outcome::Quit); }
                },
                _ = keepalive.tick() => signaling::send(&mut socket, &ClientMessage::Ping).await?,
            }
        }
    }

    async fn on_server(&mut self, message: ServerMessage, out: &mpsc::UnboundedSender<ClientMessage>, events: &mpsc::UnboundedSender<Event>) {
        match message {
            ServerMessage::OpenSession { session_id, permissions, ice_servers } => self.on_open_session(session_id, permissions, ice_servers, out, events),
            ServerMessage::Offer { session_id, sdp } => {
                if let Err(error) = self.on_offer(&session_id, sdp, out).await {
                    eprintln!("[agent] négociation impossible : {error:#}");
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
                    if let Some(peer) = active.peer.take() { peer.close().await; }
                    println!("[agent] lien perdu : en attente d'une nouvelle négociation (session {session_id})");
                }
            }
            ServerMessage::CloseSession { session_id } => self.end_session(&session_id).await,
            ServerMessage::Error { detail } => eprintln!("[agent] le serveur signale : {detail}"),
            ServerMessage::Challenge { .. } | ServerMessage::Ready { .. } => {}
        }
    }

    fn on_open_session(&mut self, session_id: String, permissions: Vec<String>, ice: Vec<IceServerConfig>, out: &mpsc::UnboundedSender<ClientMessage>,
                       events: &mpsc::UnboundedSender<Event>) {
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
        let mut names: Vec<&str> = requested.iter().map(|p| p.as_str()).collect();
        names.sort_unstable();
        self.requests.insert(session_id.clone(), Plan { permissions: requested, ice, accepted: false });
        if self.options.auto_accept {
            eprintln!("[agent] ATTENTION : --auto-accept, session acceptée sans demander ({}).", names.join(", "));
            let _ = events.send(Event::Consent { session_id, accepted: true });
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

    fn on_consent(&mut self, session_id: String, accepted: bool, out: &mpsc::UnboundedSender<ClientMessage>) {
        let Some(plan) = self.requests.get_mut(&session_id) else { return };
        plan.accepted = accepted;
        if !accepted {
            self.requests.remove(&session_id);
        }
        let reason = if accepted { "" } else { "L'utilisateur de l'appareil a refusé la connexion." };
        let _ = out.send(ClientMessage::SessionReply { session_id, accepted, reason: reason.to_string() });
    }

    /// Commandes tapées dans la console de l'agent. Renvoie true pour arrêter l'agent.
    async fn on_line(&mut self, line: &str, out: &mpsc::UnboundedSender<ClientMessage>, events: &mpsc::UnboundedSender<Event>) -> bool {
        if let Some(session_id) = self.awaiting_console.take() {
            let accepted = matches!(line.to_lowercase().as_str(), "o" | "oui" | "y" | "yes");
            let _ = events.send(Event::Consent { session_id, accepted });
            return false;
        }
        match line.to_lowercase().as_str() {
            "quit" | "exit" | "q" => {
                if let Some(active) = self.active.take() {
                    let _ = out.send(ClientMessage::Bye { session_id: active.session_id });
                    if let Some(peer) = active.peer { peer.close().await; }
                }
                return true;
            }
            "stop" => match self.active.take() {
                Some(active) => {
                    println!("[agent] session coupée.");
                    let _ = out.send(ClientMessage::Bye { session_id: active.session_id });
                    if let Some(peer) = active.peer { peer.close().await; }
                }
                None => println!("[agent] aucune session en cours."),
            },
            "" => {}
            _ => println!("[agent] commandes : stop (couper la session), quit (arrêter l'agent)"),
        }
        false
    }

    async fn on_offer(&mut self, session_id: &str, sdp: String, out: &mpsc::UnboundedSender<ClientMessage>) -> Result<()> {
        let mut active = match self.active.take() {
            Some(active) if active.session_id == session_id => active,       // renégociation après une coupure : mêmes permissions
            other => {
                self.active = other;
                let Some(plan) = self.requests.remove(session_id).filter(|plan| plan.accepted) else {
                    bail!("offre reçue pour une session non acceptée ({session_id})");
                };
                Active { session_id: session_id.to_string(), plan, peer: None }
            }
        };
        if let Some(old) = active.peer.take() { old.close().await; }

        let input = self.shared_input()?;
        let permissions = Arc::new(active.plan.permissions.clone());
        let source = self.options.source.clone();
        let on_channel: Arc<dyn Fn(Arc<webrtc::data_channel::RTCDataChannel>) + Send + Sync> =
            Arc::new(move |channel| attach_channel(channel, permissions.clone(), input.clone(), source.clone()));
        let peer = Peer::new(&active.plan.ice, session_id.to_string(), out.clone(), on_channel).await?;
        let answer = peer.answer(sdp.clone()).await?;
        // Signature de la réponse sur CETTE offre : seul le détenteur de la clé privée de l'appareil peut répondre au nom de cet appareil.
        let signature = self.identity.sign(&answer_message(session_id, &sdp, &answer));
        let _ = out.send(ClientMessage::Answer { session_id: session_id.to_string(), sdp: answer, signature });
        active.peer = Some(peer);
        self.active = Some(active);
        println!("[agent] session {session_id} : négociation envoyée.");
        Ok(())
    }

    async fn end_session(&mut self, session_id: &str) {
        self.requests.remove(session_id);
        if self.active.as_ref().is_some_and(|a| a.session_id == session_id) {
            if let Some(active) = self.active.take() {
                if let Some(peer) = active.peer { peer.close().await; }
            }
            println!("[agent] session {session_id} terminée.");
        }
    }
}
