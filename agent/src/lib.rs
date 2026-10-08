//! Agent Remote de PC Assistant (docs/REMOTE_ARCHITECTURE.md) : à installer sur le PC à contrôler.
//!
//! Il ne fonctionne jamais caché : une interface montre son état et le code d'appairage, demande l'accord de la personne devant l'appareil pour
//! CHAQUE session et permet de la couper. Souris et clavier ne sont possibles que si l'utilisateur de l'appareil les autorise.
//!
//! Cette bibliothèque est le moteur. Deux programmes s'en servent : `remote-agent.exe` (interface web locale, ou console) et ARIA Remote Desktop
//! (interface intégrée : voir [`Interface::Embedded`]).

pub mod agent;
pub mod capture;
pub mod clipboard;
pub mod config;
pub mod identity;
pub mod input;
mod instance;
pub mod network;
pub mod pipeline;
pub mod platform;
pub mod session;
pub mod ui;

use std::path::PathBuf;
use std::thread::JoinHandle;

use anyhow::{anyhow, Result};
use tokio::sync::mpsc;

pub use ui::{ConsentInfo, SessionInfo, UiCommand, UiState};

/// Où l'agent montre son état et prend les ordres de la personne devant l'appareil.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Interface {
    /// Aucune interface : tout se passe dans la console.
    Console,
    /// Petite page web servie sur 127.0.0.1 (jeton et contrôle du Host), que le programme ouvre dans une fenêtre : `Launched::url`.
    Web,
    /// Pas de serveur : le programme qui héberge l'agent lit `Launched::state` et envoie ses ordres avec `Launched::send`.
    Embedded,
}

pub struct Settings {
    /// Fichier de configuration (identité de l'appareil, clé privée comprise) ; par défaut `config::default_path()`.
    pub config_path: Option<PathBuf>,
    pub name: Option<String>,
    /// Adresse et clé du serveur : prioritaires sur ce qui est retenu. Retenues ensuite : plus rien à saisir la fois suivante.
    pub server: Option<String>,
    pub api_key: Option<String>,
    /// Permissions accordées EN PLUS de la vue de l'écran : « mouse », « keyboard » (séparées par des virgules).
    pub allow: Option<String>,
    /// Accepte les sessions sans demander (développement uniquement).
    pub auto_accept: bool,
    pub interface: Interface,
    /// Premier lancement sans adresse : pose la question (mode console). None, ou résultat vide : pas de question.
    pub ask_connection: Option<fn() -> Option<(String, String)>>,
}

impl Settings {
    pub fn new(interface: Interface) -> Self {
        Self { config_path: None, name: None, server: None, api_key: None, allow: None, auto_accept: false, interface, ask_connection: None }
    }
}

/// L'agent en marche. Le lâcher ne l'arrête pas : envoyer `UiCommand::Quit`, puis `wait`.
pub struct Launched {
    ui: Option<ui::Ui>,
    commands: mpsc::UnboundedSender<UiCommand>,
    url: Option<String>,
    worker: JoinHandle<Result<()>>,
    _lock: instance::Lock,
}

impl Launched {
    /// Adresse de la page web de l'agent (`Interface::Web` seulement).
    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }

    /// État affiché (lien avec le serveur, code d'appairage, demande d'accord, session en cours...). None en mode console.
    pub fn state(&self) -> Option<UiState> {
        self.ui.as_ref().map(|ui| ui.snapshot())
    }

    /// Donne un ordre à l'agent ; `false` s'il est déjà arrêté.
    pub fn send(&self, command: UiCommand) -> bool {
        self.commands.send(command).is_ok()
    }

    /// Attend la fin de l'agent (ordre `Quit`, ou erreur).
    pub async fn wait(self) -> Result<()> {
        let Self { worker, commands, ui, _lock, .. } = self;
        drop((commands, ui));
        tokio::task::spawn_blocking(move || worker.join())
            .await?
            .map_err(|_| anyhow!("l'agent s'est arrêté brutalement"))?
    }
}

#[cfg(windows)]
fn default_input() -> Result<Box<dyn input::InputSink>> {
    Ok(Box::new(input::native::NativeInput::new()?))
}

#[cfg(not(windows))]
fn default_input() -> Result<Box<dyn input::InputSink>> {
    eprintln!("[agent] hors Windows : la saisie est seulement journalisée (aucune action réelle).");
    Ok(Box::new(input::LogInput::echoing()))
}

/// Prépare puis démarre l'agent dans son propre fil (son propre moteur asynchrone) : le programme appelant garde le sien.
/// Refuse de démarrer si un autre agent tourne déjà sur cet appareil.
pub async fn launch(settings: Settings) -> Result<Launched> {
    let Settings { config_path, name, server: cli_server, api_key: cli_key, allow: cli_allow, auto_accept, interface, ask_connection } = settings;
    let path = config_path.unwrap_or_else(config::default_path);
    let lock = instance::acquire(&path)?;
    let (mut config, identity) = config::load_or_create(&path, name.as_deref())?;
    println!("Agent Remote — appareil « {} » ({})", config.name, config.device_id);

    // Adresse et clé : paramètres > valeurs retenues > interface (ou console) > défaut local.
    let gui = interface != Interface::Console;
    let unknown_server = cli_server.as_deref().and_then(config::normalize_server).is_none() && config.server.is_none();
    let (server, api_key, mut changed) = if gui && unknown_server {
        (String::new(), cli_key.clone().unwrap_or_default(), false) // l'interface la demandera
    } else {
        let mut ask = || ask_connection.and_then(|ask| ask());
        config::resolve_connection(cli_server, cli_key, &mut config, &mut ask)
    };
    if let Some(allow) = cli_allow {
        if config.allow.as_ref() != Some(&allow) {
            config.allow = Some(allow);
            changed = true;
        }
    }
    if changed {
        config::save(&path, &config)?;
        println!("Réglages retenus dans {} : la prochaine fois, plus rien à saisir.", path.display());
    }
    let allow = input::parse_allow(config.allow.as_deref().unwrap_or(""));
    let fingerprint = identity::sha256_hex(&identity.public_key())[..16].to_string();
    println!("Empreinte de la clé : {fingerprint}");

    let (commands_tx, commands_rx) = mpsc::unbounded_channel();
    let initial = || ui::UiState {
        device_name: config.name.clone(),
        device_id: config.device_id.clone(),
        fingerprint: fingerprint.clone(),
        server: server.clone(),
        link: if server.is_empty() { "setup".into() } else { "connecting".into() },
        ..Default::default()
    };
    let (ui, url) = match interface {
        Interface::Console => {
            if !server.is_empty() {
                println!("Serveur : {server}");
            }
            (None, None)
        }
        Interface::Web => {
            let ui = ui::start(commands_tx.clone(), initial()).await?;
            let url = ui.url();
            println!("Fenêtre de l'agent : {url}");
            (Some(ui), Some(url))
        }
        Interface::Embedded => (Some(ui::UiShared::new(commands_tx.clone(), 0, initial())), None),
    };

    let options = agent::Options {
        server,
        api_key,
        allow,
        auto_accept,
        screens: capture::screen::default_screens(),
        input: Box::new(default_input),
        ui: ui.clone(),
        config_path: path,
    };
    let worker = std::thread::Builder::new().name("agent".into()).spawn(move || -> Result<()> {
        tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(agent::Agent::new(options, config, identity, commands_rx).run())
    })?;
    Ok(Launched { ui, commands: commands_tx, url, worker, _lock: lock })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_embedded_agent_exposes_its_state_refuses_a_second_instance_and_stops_on_quit() {
        let dir = std::env::temp_dir().join(format!("aria-remote-embedded-{}", std::process::id()));
        let config = dir.join("config.json");
        let mut settings = Settings::new(Interface::Embedded);
        settings.config_path = Some(config.clone());
        settings.name = Some("Test".into());
        settings.server = Some("http://127.0.0.1:9".into());      // personne n'écoute : l'agent réessaie en boucle, sans rien casser
        let agent = launch(settings).await.expect("démarrage");

        let state = agent.state().expect("état disponible en mode intégré");
        assert_eq!(state.device_name, "Test");
        assert_eq!(state.link, "connecting");
        assert!(agent.url().is_none(), "aucun serveur web en mode intégré");

        let mut second = Settings::new(Interface::Embedded);
        second.config_path = Some(config);
        let refused = launch(second).await.err().expect("seconde instance refusée").to_string();
        assert!(refused.contains("tourne déjà"), "{refused}");

        assert!(agent.send(UiCommand::Quit));
        tokio::time::timeout(std::time::Duration::from_secs(10), agent.wait()).await.expect("arrêt dans les 10 s").expect("arrêt propre");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
