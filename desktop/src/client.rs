//! Client du serveur de rendez-vous, côté contrôleur : mêmes routes `/api/remote/*` et même clé (`x-api-key`) que le plugin d'ARIA.
//! Aucune dépendance à Tauri : tout ce qui est testable est ici.
use serde_json::Value;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(75); // la confirmation sur l'appareil contrôlé peut prendre jusqu'à 60 s

/// Adresse « propre » du serveur : https par défaut, sans barre finale ni `/api/remote`. L'http n'est accepté que sur un réseau local :
/// la clé du contrôleur ne doit jamais voyager en clair sur Internet.
pub fn normalize_server(raw: &str) -> Result<String, String> {
    let text = raw.trim();
    if text.is_empty() {
        return Err("Saisis l'adresse du serveur de rendez-vous.".into());
    }
    let (scheme, rest) = match text.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") => (scheme.to_ascii_lowercase(), rest),
        Some(_) => return Err("L'adresse doit commencer par https:// (ou http:// sur un réseau local).".into()),
        None => ("https".to_string(), text),
    };
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix("/api/remote").unwrap_or(rest);
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_only = host.rsplit('@').next().unwrap_or("").split(':').next().unwrap_or("");
    if host_only.is_empty() || rest.contains(char::is_whitespace) {
        return Err("Adresse invalide : il manque le nom du serveur.".into());
    }
    if scheme == "http" && !is_local_host(host_only) {
        return Err("Un serveur sur Internet doit utiliser https:// : la clé ne doit pas circuler en clair.".into());
    }
    Ok(format!("{scheme}://{rest}"))
}

fn is_local_host(host: &str) -> bool {
    if host == "localhost" || host == "127.0.0.1" || host.ends_with(".local") {
        return true;
    }
    let parts: Vec<u8> = host.split('.').filter_map(|p| p.parse().ok()).collect();
    parts.len() == 4 && host.split('.').count() == 4 && match (parts[0], parts[1]) {
        (10, _) | (192, 168) => true,
        (172, second) => (16..=31).contains(&second),
        _ => false,
    }
}

/// Le code d'appairage : six chiffres, tout le reste (espaces, tirets) est ignoré.
#[allow(dead_code)] // même règle que la saisie dans la fenêtre (web/app.js) ; gardée ici pour les tests
pub fn pairing_code(raw: &str) -> Option<String> {
    let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
    (digits.len() == 6).then_some(digits)
}

/// Message lisible pour une réponse en erreur : le « detail » du serveur s'il y en a un, sinon un texte selon le code HTTP.
pub fn error_text(status: u16, body: &str) -> String {
    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(body) {
        if let Some(Value::String(detail)) = map.get("detail") {
            return detail.clone();
        }
    }
    match status {
        401 | 403 => "Clé du contrôleur refusée par le serveur.".into(),
        404 => "Ce serveur ne répond pas comme un serveur de rendez-vous (adresse incorrecte ?).".into(),
        429 => "Trop d'essais : patiente quelques instants.".into(),
        502..=504 => "Le serveur de rendez-vous est indisponible.".into(),
        other => format!("Erreur du serveur (code {other})."),
    }
}

pub struct Client {
    http: reqwest::Client,
    base: String,
    key: String,
}

impl Client {
    pub fn new(server: &str, key: &str) -> Result<Self, String> {
        let base = normalize_server(server)?;
        let key = key.trim().to_string();
        if key.is_empty() {
            return Err("Saisis la clé du contrôleur.".into());
        }
        let http = reqwest::Client::builder().timeout(TIMEOUT).build().map_err(|e| format!("Client HTTP impossible : {e}"))?;
        Ok(Self { http, base, key })
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<Value, String> {
        let response = request.header("x-api-key", &self.key).send().await.map_err(|e| {
            if e.is_timeout() { "Le serveur ne répond pas (délai dépassé).".to_string() } else { format!("Serveur injoignable ({}).", self.base) }
        })?;
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        if !(200..300).contains(&status) {
            return Err(error_text(status, &body));
        }
        serde_json::from_str(&body).map_err(|_| "Réponse inattendue du serveur.".to_string())
    }

    pub async fn status(&self) -> Result<Value, String> {
        self.send(self.http.get(format!("{}/api/remote/status", self.base))).await
    }

    /// Un appel du contrôleur vers `/api/remote<path>`. Seules les routes du contrôleur passent (voir `allowed`) : la fenêtre ne peut jamais agir en agent.
    pub async fn request(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
        if !allowed(method, path) {
            return Err("Appel non autorisé.".into());
        }
        let url = format!("{}/api/remote{}", self.base, path);
        let builder = match method {
            "GET" => self.http.get(url),
            "POST" => self.http.post(url).json(&body.unwrap_or_else(|| serde_json::json!({}))),
            _ => self.http.delete(url),
        };
        self.send(builder).await
    }
}

/// Routes que la fenêtre peut appeler (côté contrôleur) : appareils, appairage, sessions, signaling, serveurs STUN/TURN.
/// Jamais les routes d'agent (`/devices/register`, `/agent/...`).
pub fn allowed(method: &str, path: &str) -> bool {
    let (route, query) = path.split_once('?').unwrap_or((path, ""));
    let safe = |text: &str| text.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/' | '=' | '&'));
    if !route.starts_with('/') || route.contains("..") || route.contains("//") || !safe(route) || !safe(query) {
        return false;
    }
    let parts: Vec<&str> = route.trim_start_matches('/').split('/').collect();
    match (method, parts.as_slice()) {
        ("GET", ["status"] | ["devices"] | ["ice-servers"]) => true,
        ("GET", ["devices", id]) => *id != "register",
        ("POST", ["pair"] | ["sessions"] | ["signaling", "offer" | "answer" | "ice"]) => true,
        ("GET" | "DELETE", ["sessions", _]) => true,
        ("POST", ["sessions", _, "reconnect"]) => true,
        ("GET", ["sessions", _, "signaling"]) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn server_address_is_cleaned_up() {
        assert_eq!(normalize_server(" rendezvous.exemple.fr/ ").unwrap(), "https://rendezvous.exemple.fr");
        assert_eq!(normalize_server("https://rv.exemple.fr/api/remote").unwrap(), "https://rv.exemple.fr");
        assert_eq!(normalize_server("HTTP://192.168.1.20:8080").unwrap(), "http://192.168.1.20:8080");
        assert_eq!(normalize_server("http://localhost:8080/").unwrap(), "http://localhost:8080");
    }

    #[test]
    fn plain_http_is_refused_outside_the_local_network() {
        assert!(normalize_server("http://rendezvous.exemple.fr").is_err());
        assert!(normalize_server("http://8.8.8.8").is_err());
        assert!(normalize_server("http://172.40.0.1").is_err());
        assert!(normalize_server("ftp://x").is_err());
        assert!(normalize_server("   ").is_err());
        assert!(normalize_server("https://").is_err());
    }

    #[test]
    fn pairing_code_keeps_only_six_digits() {
        assert_eq!(pairing_code(" 085 201 ").as_deref(), Some("085201"));
        assert_eq!(pairing_code("08-52-01").as_deref(), Some("085201"));
        assert_eq!(pairing_code("12345"), None);
        assert_eq!(pairing_code("1234567"), None);
        assert_eq!(pairing_code("abc"), None);
    }

    #[test]
    fn errors_are_readable() {
        assert_eq!(error_text(400, r#"{"detail":"Code invalide ou expiré."}"#), "Code invalide ou expiré.");
        assert!(error_text(401, "").contains("Clé"));
        assert!(error_text(404, "<html>").contains("rendez-vous"));
        assert!(error_text(500, "{}").contains("500"));
    }

    #[test]
    fn a_key_is_required() {
        assert!(Client::new("https://rv.exemple.fr", "  ").is_err());
    }

    /// Mini serveur HTTP : répond une fois avec `reply` et renvoie la requête reçue (en-têtes et corps).
    async fn serve_once(reply: &'static str) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut received = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let n = socket.read(&mut buffer).await.unwrap();
                received.extend_from_slice(&buffer[..n]);
                let text = String::from_utf8_lossy(&received).to_string();
                if let Some(split) = text.find("\r\n\r\n") {
                    let length = text[..split].lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                    if received.len() >= split + 4 + length { break; }
                }
                if n == 0 { break; }
            }
            socket.write_all(reply.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&received).to_string()
        });
        (address, task)
    }

    #[tokio::test]
    async fn devices_are_listed_with_the_controller_key() {
        let body = r#"{"devices":[{"device_id":"dev_1","name":"Bureau","paired":true,"online":true}]}"#;
        let reply: &'static str = Box::leak(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}", body.len(), body).into_boxed_str());
        let (address, server) = serve_once(reply).await;
        let client = Client::new(&address, "une-cle-de-controleur-assez-longue").unwrap();
        let devices = client.request("GET", "/devices", None).await.unwrap();
        assert_eq!(devices["devices"][0]["name"], "Bureau");
        let request = server.await.unwrap().to_ascii_lowercase();
        assert!(request.starts_with("get /api/remote/devices "));
        assert!(request.contains("x-api-key: une-cle-de-controleur-assez-longue"));
    }

    #[tokio::test]
    async fn a_post_sends_its_body_and_shows_the_server_error() {
        let body = r#"{"detail":"Code invalide ou expiré."}"#;
        let reply: &'static str = Box::leak(format!("HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}", body.len(), body).into_boxed_str());
        let (address, server) = serve_once(reply).await;
        let client = Client::new(&address, "cle").unwrap();
        let error = client.request("POST", "/pair", Some(serde_json::json!({ "code": "085201" }))).await.unwrap_err();
        assert_eq!(error, "Code invalide ou expiré.");
        let request = server.await.unwrap();
        assert!(request.starts_with("POST /api/remote/pair "));
        assert!(request.ends_with(r#"{"code":"085201"}"#));
    }

    #[test]
    fn only_controller_routes_are_allowed() {
        for (method, path) in [("GET", "/devices"), ("GET", "/devices/dev_bureau_0001"), ("POST", "/pair"), ("POST", "/sessions"), ("GET", "/sessions/sess_1"),
                               ("DELETE", "/sessions/sess_1"), ("POST", "/sessions/sess_1/reconnect"), ("GET", "/sessions/sess_1/signaling?after=3"),
                               ("POST", "/signaling/offer"), ("POST", "/signaling/ice"), ("GET", "/ice-servers?session_id=sess_1"), ("GET", "/status")] {
            assert!(allowed(method, path), "{method} {path} devrait passer");
        }
        for (method, path) in [("POST", "/devices/register"), ("GET", "/devices/register"), ("GET", "/agent/ws"), ("POST", "/agent/ws"), ("DELETE", "/devices"),
                               ("GET", "/sessions/../agent/ws"), ("GET", "//agent"), ("GET", "devices"), ("PUT", "/sessions"), ("GET", "/sessions/s 1"),
                               ("GET", "/devices?x=%00"), ("POST", "/signaling/other"), ("GET", "/")] {
            assert!(!allowed(method, path), "{method} {path} devrait être refusé");
        }
    }

    #[tokio::test]
    async fn a_forbidden_call_never_reaches_the_network() {
        let client = Client::new("http://127.0.0.1:9", "cle").unwrap();
        assert_eq!(client.request("POST", "/devices/register", None).await.unwrap_err(), "Appel non autorisé.");
    }

    #[tokio::test]
    async fn a_closed_server_gives_a_clear_message() {
        let client = Client::new("http://127.0.0.1:9", "cle").unwrap();
        assert!(client.status().await.unwrap_err().contains("injoignable"));
    }
}
