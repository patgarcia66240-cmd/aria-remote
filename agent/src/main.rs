//! Agent Remote de PC Assistant (docs/REMOTE_ARCHITECTURE.md) : à installer sur le PC à contrôler.
//!
//! Il ne fonctionne jamais caché : il tourne dans une console visible, affiche le code d'appairage, demande l'accord de la personne devant
//! l'appareil pour CHAQUE session et s'arrête avec « quit ». Souris et clavier ne sont possibles que si `--allow` les autorise.

mod agent;
mod capture;
mod config;
mod identity;
mod input;
mod network;
mod session;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;

#[derive(Parser)]
#[command(version, about = "Agent Remote de PC Assistant")]
struct Cli {
    /// Adresse du backend PC Assistant (celle de l'API, ex. http://192.168.1.20:8000).
    #[arg(long, env = "REMOTE_SERVER", default_value = "http://127.0.0.1:8000")]
    server: String,
    /// Clé d'API du backend (API_AUTH_TOKEN).
    #[arg(long, env = "REMOTE_API_KEY", default_value = "")]
    api_key: String,
    /// Nom affiché dans PC Assistant (par défaut : nom de l'ordinateur).
    #[arg(long)]
    name: Option<String>,
    /// Permissions accordées EN PLUS de la vue de l'écran : mouse, keyboard (séparées par des virgules). Par défaut : regarder seulement.
    #[arg(long, default_value = "")]
    allow: String,
    /// Accepte les sessions sans demander (développement uniquement).
    #[arg(long)]
    auto_accept: bool,
    /// Fichier de configuration (identité de l'appareil, clé privée comprise).
    #[arg(long)]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let path = cli.config.unwrap_or_else(config::default_path);
    let (config, identity) = config::load_or_create(&path, cli.name.as_deref())?;
    println!("Agent Remote — appareil « {} » ({})", config.name, config.device_id);
    println!("Serveur : {}", cli.server);
    println!("Empreinte de la clé : {}", &identity::sha256_hex(&identity.public_key())[..16]);

    let options = agent::Options {
        server: cli.server,
        api_key: cli.api_key,
        allow: input::parse_allow(&cli.allow),
        auto_accept: cli.auto_accept,
        source: Arc::new(capture::screen::default_source),
        input: Box::new(default_input),
    };
    agent::Agent::new(options, config, identity).run().await
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
