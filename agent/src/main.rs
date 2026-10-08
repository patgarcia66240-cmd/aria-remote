//! Agent Remote de PC Assistant (docs/REMOTE_ARCHITECTURE.md) : à installer sur le PC à contrôler.
//!
//! Il ne fonctionne jamais caché : une fenêtre montre son état et le code d'appairage, demande l'accord de la personne devant l'appareil pour
//! CHAQUE session et permet de la couper. Souris et clavier ne sont possibles que si l'utilisateur de l'appareil les autorise (dans la fenêtre).
//! Le moteur est dans la bibliothèque (lib.rs) ; ce programme n'ajoute que la ligne de commande et l'ouverture de la fenêtre.

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use remote_agent::{launch, ui, Interface, Settings};

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
    let mut settings = Settings::new(if cli.no_gui { Interface::Console } else { Interface::Web });
    settings.config_path = cli.config;
    settings.name = cli.name;
    settings.server = cli.server;
    settings.api_key = cli.api_key;
    settings.allow = cli.allow;
    settings.auto_accept = cli.auto_accept;
    settings.ask_connection = Some(ask_connection);
    let agent = launch(settings).await?;
    if let (false, Some(url)) = (cli.no_window, agent.url()) {
        ui::open_window(url);
    }
    agent.wait().await
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
