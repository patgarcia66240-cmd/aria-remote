//! « Cet appareil » : le moteur de l'agent (bibliothèque `remote_agent`) tourne DANS ce programme, sans second exécutable ni fenêtre Edge.
//! La fenêtre lit l'état (`agent_state`) et donne ses ordres (`agent_command`) ; l'accord pour chaque session passe toujours par la personne
//! devant l'appareil (carte « demande de contrôle » de la fenêtre, remontée au premier plan si elle était réduite).
use remote_agent::{launch, Interface, Launched, Settings, UiCommand};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Mutex;
use std::time::Duration;

#[derive(Default)]
pub struct Host {
    agent: Mutex<Option<Launched>>,
}

/// Ordres que la fenêtre peut donner à l'agent (le reste — arrêt — a sa propre commande).
#[derive(Deserialize, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentCommand {
    Consent { session_id: String, accept: bool },
    StopSession,
    SetAllow { mouse: bool, keyboard: bool },
    Configure { server: String, api_key: String },
    NewPairingCode,
}

impl From<AgentCommand> for UiCommand {
    fn from(command: AgentCommand) -> Self {
        match command {
            AgentCommand::Consent { session_id, accept } => UiCommand::Consent { session_id, accept },
            AgentCommand::StopSession => UiCommand::StopSession,
            AgentCommand::SetAllow { mouse, keyboard } => UiCommand::SetAllow { mouse, keyboard },
            AgentCommand::Configure { server, api_key } => UiCommand::Configure { server, api_key },
            AgentCommand::NewPairingCode => UiCommand::NewPairingCode,
        }
    }
}

impl Host {
    fn agent(&self) -> std::sync::MutexGuard<'_, Option<Launched>> {
        self.agent.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn running(&self) -> bool {
        self.agent().is_some()
    }

    pub fn state(&self) -> Option<Value> {
        self.agent().as_ref().and_then(|agent| agent.state()).and_then(|state| serde_json::to_value(state).ok())
    }

    pub fn send(&self, command: UiCommand) -> bool {
        self.agent().as_ref().is_some_and(|agent| agent.send(command))
    }

    pub async fn start(&self) -> Result<(), String> {
        if self.running() {
            return Ok(());
        }
        let agent = launch(Settings::new(Interface::Embedded)).await.map_err(|error| format!("{error:#}"))?;
        *self.agent() = Some(agent);
        Ok(())
    }

    pub async fn stop(&self) {
        let Some(agent) = self.agent().take() else { return };
        agent.send(UiCommand::Quit);
        let _ = tokio::time::timeout(Duration::from_secs(5), agent.wait()).await;
    }

    /// Arrêt sans attendre (fermeture du programme) : l'ordre part, le processus s'arrête de toute façon juste après.
    pub fn stop_now(&self) {
        if let Some(agent) = self.agent().take() {
            agent.send(UiCommand::Quit);
        }
    }
}

#[tauri::command]
pub async fn agent_start(host: tauri::State<'_, Host>) -> Result<(), String> {
    host.start().await
}

#[tauri::command]
pub async fn agent_stop(host: tauri::State<'_, Host>) -> Result<(), String> {
    host.stop().await;
    Ok(())
}

/// État de l'agent (lien avec le serveur, code d'appairage, demande d'accord, session) ; null s'il ne tourne pas.
#[tauri::command]
pub fn agent_state(host: tauri::State<'_, Host>) -> Option<Value> {
    host.state()
}

#[tauri::command]
pub fn agent_command(host: tauri::State<'_, Host>, command: AgentCommand) -> Result<(), String> {
    if host.send(command.into()) { Ok(()) } else { Err("L'agent ne tourne pas.".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_orders_map_to_agent_commands() {
        let consent: AgentCommand = serde_json::from_str(r#"{"type":"consent","session_id":"s1","accept":true}"#).unwrap();
        assert_eq!(UiCommand::from(consent), UiCommand::Consent { session_id: "s1".into(), accept: true });
        let allow: AgentCommand = serde_json::from_str(r#"{"type":"set_allow","mouse":true,"keyboard":false}"#).unwrap();
        assert_eq!(UiCommand::from(allow), UiCommand::SetAllow { mouse: true, keyboard: false });
        let stop: AgentCommand = serde_json::from_str(r#"{"type":"stop_session"}"#).unwrap();
        assert_eq!(UiCommand::from(stop), UiCommand::StopSession);
        assert!(serde_json::from_str::<AgentCommand>(r#"{"type":"quit"}"#).is_err(), "l'arrêt n'est pas un ordre de la fenêtre");
    }

    #[test]
    fn without_an_agent_there_is_no_state_and_orders_are_refused() {
        let host = Host::default();
        assert!(!host.running());
        assert!(host.state().is_none());
        assert!(!host.send(UiCommand::StopSession));
    }
}
