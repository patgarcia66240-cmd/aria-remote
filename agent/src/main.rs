//! Agent Remote de PC Assistant (docs/REMOTE_ARCHITECTURE.md) : à installer sur le PC à contrôler.
//!
//! Il ne fonctionne jamais caché : une fenêtre montre son état et le code d'appairage, demande l'accord de la personne devant l'appareil pour
//! CHAQUE session et permet de la couper. Souris et clavier ne sont possibles que si l'utilisateur de l'appareil les autorise (dans la fenêtre).

mod agent;
mod capture;
mod config;
mod identity;
mod input;
mod network;
mod session;
mod ui;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use tokio::sync::mpsc;

#[derive(Parser)]
#[command(version, about = "Agent Remote de PC Assistant")]
struct Cli {
    /// Adresse du serveur : celle du serveur de rendez-vous (accès par Internet, ex. https://rendezvous.exemple.fr) ou du backend PC Assistant
    /// (même réseau, ex. http://192.168.1.20:8000). Retenue après le premier lancement ; sans elle, la fenêtre la demande.
    #[arg(long, env = "REMOTE_SERVER")]
    server: Option<String>,
    /// Clé d'accès de l'agent à ce serveur (RENDEZVOUS_AGENT_KEY, ou API_AUTH_TOKEN du backend). Retenue avec l'adresse.
    #[arg(long, env = "REMOTE_API_KEY")]
    api_key: Option<String>,
    /// Nom affiché dans PC Assistant (par défaut : nom de l'ordinateur).
    #[arg(long)]
    name: Option<String>,
    /// Permissions accordées EN PLUS de la vue de l'écran : mouse, keyboard (séparées par des virgules). Retenues ; modifiables dans la fenêtre.
    /// Par défaut : regarder seulement.
    #[arg(long)]
    allow: Option<String>,
    /// Accepte les sessions sans demander (développement uniquement).
    #[arg(long)]
    auto_accept: bool,
    /// Pas de fenêtre ni de serveur local : tout se passe dans la console.
    #[arg(long)]
    no_gui: bool,
    /// Garde la fenêtre disponible mais ne l'ouvre pas (affiche son adresse) : tests, ou ouverture à la main.
    #[arg(long)]
    no_window: bool,
    /// Fichier de configuration (identité de l'appareil, clé privée comprise).
    #[arg(long)]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let path = cli.config.unwrap_or_else(config::default_path);
    let (mut config, identity) = config::load_or_create(&path, cli.name.as_deref())?;
    println!("Agent Remote — appareil « {} » ({})", config.name, config.device_id);

    // Adresse et clé : ligne de commande > valeurs retenues > fenêtre (ou console avec --no-gui) > défaut local.
    let gui = !cli.no_gui;
    let unknown_server = cli.server.as_deref().and_then(config::normalize_server).is_none() && config.server.is_none();
    let (server, api_key, mut changed) = if gui && unknown_server {
        (String::new(), cli.api_key.clone().unwrap_or_default(), false) // la fenêtre la demandera
    } else {
        config::resolve_connection(cli.server, cli.api_key, &mut config, &mut ask_connection)
    };
    if let Some(allow) = cli.allow {
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
    let ui = if gui {
        let initial = ui::UiState {
            device_name: config.name.clone(),
            device_id: config.device_id.clone(),
            fingerprint: fingerprint.clone(),
            server: server.clone(),
            link: if server.is_empty() { "setup".into() } else { "connecting".into() },
            ..Default::default()
        };
        let ui = ui::start(commands_tx.clone(), initial).await?;
        println!("Fenêtre de l'agent : {}", ui.url());
        if !cli.no_window {
            ui::open_window(&ui.url());
        }
        Some(ui)
    } else {
        if !server.is_empty() {
            println!("Serveur : {server}");
        }
        None
    };

    let options = agent::Options {
        server,
        api_key,
        allow,
        auto_accept: cli.auto_accept,
        source: Arc::new(capture::screen::default_source),
        input: Box::new(default_input),
        ui,
        config_path: path,
    };
    let result = agent::Agent::new(options, config, identity, commands_rx).run().await;
    drop(commands_tx);
    result
}

/// Mode console (--no-gui), premier lancement : demande où se trouve le serveur. None hors terminal (service, tâche planifiée).
fn ask_connection() -> Option<(String, String)> {
    use std::io::{BufRead, IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        return None;
    }
    let ask = |question: &str| -> String {
        print!("{question}");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        line.trim().to_string()
    };
    println!("\nPremier lancement sur cet appareil.");
    println!("Entre l'adresse du serveur : rendez-vous (Internet) ou PC Assistant (même réseau, ex. 192.168.1.20:8000).");
    let server = ask("Adresse du serveur : ");
    let key = ask("Clé d'accès (vide s'il n'y en a pas) : ");
    Some((server, key))
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
