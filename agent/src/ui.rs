//! Interface graphique de l'agent (style TeamViewer) : une petite page web servie UNIQUEMENT sur cet ordinateur (127.0.0.1) et ouverte dans une
//! fenêtre d'application (Edge ou Chrome en mode « app », sans barre d'adresse). Elle montre le code d'appairage, l'état, les permissions, demande
//! l'accord pour chaque session et coupe la session en cours. Aucune dépendance graphique native : même interface sous Windows, Linux et macOS.
//!
//! Sécurité : la page peut AUTORISER une prise de contrôle, donc aucune autre page web du navigateur ne doit pouvoir l'actionner à ta place.
//! - écoute en 127.0.0.1 seulement, sur un port choisi au hasard ;
//! - chaque appel exige un jeton aléatoire propre à ce lancement (donné à la fenêtre dans l'adresse d'ouverture, jamais écrit sur le disque) ;
//! - l'en-tête Host doit être celui de l'agent (contre le « DNS rebinding »).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;

const PAGE: &str = include_str!("../ui/index.html");
const MAX_EVENTS: usize = 12;
/// La fenêtre est « vue » si elle a interrogé l'agent il y a moins de 3 s en se disant visible (les onglets cachés ralentissent leurs minuteries).
const VISIBLE_WITHIN: Duration = Duration::from_secs(3);

/// Ordres donnés par la fenêtre à l'agent.
#[derive(Debug, PartialEq)]
pub enum UiCommand {
    Consent { session_id: String, accept: bool },
    StopSession,
    SetAllow { mouse: bool, keyboard: bool },
    Configure { server: String, api_key: String },
    /// Demande un NOUVEAU code d'appairage, même si l'appareil est déjà appairé (autre contrôleur, appareil retiré de la liste...).
    NewPairingCode,
    Quit,
}

#[derive(Serialize, Clone, Default, PartialEq, Debug)]
pub struct ConsentInfo {
    pub session_id: String,
    pub permissions: Vec<String>,
    pub since_ms: u64,
}

#[derive(Serialize, Clone, Default, PartialEq, Debug)]
pub struct SessionInfo {
    pub session_id: String,
    pub permissions: Vec<String>,
    pub since_ms: u64,
    /// Etat du lien WebRTC : connecting, connected, disconnected, failed, closed.
    pub state: String,
}

/// Ce que la fenêtre affiche. `link` : setup | connecting | unpaired | online | error.
#[derive(Serialize, Clone, Default, Debug)]
pub struct UiState {
    pub version: String,
    pub device_name: String,
    pub device_id: String,
    pub fingerprint: String,
    pub server: String,
    pub link: String,
    pub link_detail: String,
    pub pairing_code: Option<String>,
    pub pairing_expires_ms: Option<u64>,
    pub allow_mouse: bool,
    pub allow_keyboard: bool,
    pub consent: Option<ConsentInfo>,
    pub session: Option<SessionInfo>,
    pub events: Vec<(u64, String)>,
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub struct UiShared {
    state: Mutex<UiState>,
    commands: UnboundedSender<UiCommand>,
    token: String,
    port: u16,
    last_visible: Mutex<Option<Instant>>,
}

pub type Ui = Arc<UiShared>;

impl UiShared {
    pub fn new(commands: UnboundedSender<UiCommand>, port: u16, initial: UiState) -> Ui {
        let mut raw = [0u8; 24];
        rand::rngs::OsRng.fill_bytes(&mut raw);
        Arc::new(Self { state: Mutex::new(initial), commands, token: hex::encode(raw), port, last_visible: Mutex::new(None) })
    }

    /// Modifie l'état affiché (le verrou est tenu le temps de la closure : pas d'await dedans).
    pub fn update(&self, change: impl FnOnce(&mut UiState)) {
        if let Ok(mut state) = self.state.lock() {
            change(&mut state);
        }
    }

    pub fn log(&self, line: impl Into<String>) {
        let line = line.into();
        self.update(|state| {
            state.events.push((now_ms(), line));
            if state.events.len() > MAX_EVENTS {
                state.events.remove(0);
            }
        });
    }

    pub fn snapshot(&self) -> UiState {
        self.state.lock().map(|s| UiState { version: env!("CARGO_PKG_VERSION").to_string(), ..s.clone() }).unwrap_or_default()
    }

    /// La fenêtre est-elle ouverte ET visible ? Si oui, c'est elle qui demande l'accord ; sinon l'accord passe par une fenêtre système.
    pub fn window_visible(&self) -> bool {
        self.last_visible.lock().ok().and_then(|seen| *seen).is_some_and(|seen| seen.elapsed() < VISIBLE_WITHIN)
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/?t={}", self.port, self.token)
    }

    fn send(&self, command: UiCommand) -> bool {
        self.commands.send(command).is_ok()
    }
}

// -- Contrôles d'accès (fonctions pures, testées) -------------------------------------------------------------------------------
pub fn host_allowed(host: Option<&str>, port: u16) -> bool {
    matches!(host, Some(h) if h == format!("127.0.0.1:{port}") || h == format!("localhost:{port}"))
}

pub fn token_matches(given: Option<&str>, expected: &str) -> bool {
    let Some(given) = given else { return false };
    given.len() == expected.len() && given.bytes().zip(expected.bytes()).fold(0u8, |diff, (a, b)| diff | (a ^ b)) == 0
}

async fn guard(State(ui): State<Ui>, request: Request, next: Next) -> Response {
    let headers: &HeaderMap = request.headers();
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    if !host_allowed(host, ui.port) {
        return StatusCode::FORBIDDEN.into_response();
    }
    // La page elle-même (GET /) est publique sur 127.0.0.1 : elle ne contient aucune donnée. Tout le reste exige le jeton.
    if request.uri().path() != "/" {
        let token = headers.get("x-token").and_then(|v| v.to_str().ok());
        if !token_matches(token, &ui.token) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }
    let mut response = next.run(request).await;
    response.headers_mut().insert(header::CACHE_CONTROL, "no-store".parse().expect("valeur statique"));
    response.headers_mut().insert(header::X_FRAME_OPTIONS, "DENY".parse().expect("valeur statique"));
    response
}

#[derive(Deserialize)]
struct StateQuery {
    #[serde(default)]
    v: u8,
}

async fn page() -> Html<&'static str> {
    Html(PAGE)
}

async fn state(State(ui): State<Ui>, axum::extract::Query(query): axum::extract::Query<StateQuery>) -> Json<UiState> {
    if query.v == 1 {
        if let Ok(mut seen) = ui.last_visible.lock() {
            *seen = Some(Instant::now());
        }
    }
    Json(ui.snapshot())
}

#[derive(Deserialize)]
struct ConsentBody {
    session_id: String,
    accept: bool,
}

#[derive(Deserialize)]
struct AllowBody {
    mouse: bool,
    keyboard: bool,
}

#[derive(Deserialize)]
struct SetupBody {
    server: String,
    #[serde(default)]
    api_key: String,
}

fn ack(sent: bool) -> StatusCode {
    if sent { StatusCode::NO_CONTENT } else { StatusCode::SERVICE_UNAVAILABLE }
}

async fn consent(State(ui): State<Ui>, Json(body): Json<ConsentBody>) -> StatusCode {
    ack(ui.send(UiCommand::Consent { session_id: body.session_id, accept: body.accept }))
}

async fn stop(State(ui): State<Ui>) -> StatusCode {
    ack(ui.send(UiCommand::StopSession))
}

async fn allow(State(ui): State<Ui>, Json(body): Json<AllowBody>) -> StatusCode {
    ack(ui.send(UiCommand::SetAllow { mouse: body.mouse, keyboard: body.keyboard }))
}

async fn setup(State(ui): State<Ui>, Json(body): Json<SetupBody>) -> StatusCode {
    ack(ui.send(UiCommand::Configure { server: body.server, api_key: body.api_key }))
}

async fn new_pairing_code(State(ui): State<Ui>) -> StatusCode {
    ack(ui.send(UiCommand::NewPairingCode))
}

async fn quit(State(ui): State<Ui>) -> StatusCode {
    ack(ui.send(UiCommand::Quit))
}

pub fn router(ui: Ui) -> Router {
    Router::new()
        .route("/", get(page))
        .route("/api/state", get(state))
        .route("/api/consent", post(consent))
        .route("/api/stop", post(stop))
        .route("/api/allow", post(allow))
        .route("/api/setup", post(setup))
        .route("/api/pairing-code", post(new_pairing_code))
        .route("/api/quit", post(quit))
        .layer(middleware::from_fn_with_state(ui.clone(), guard))
        .with_state(ui)
}

/// Démarre le serveur local sur un port libre ; renvoie l'interface partagée (état + adresse d'ouverture).
pub async fn start(commands: UnboundedSender<UiCommand>, initial: UiState) -> anyhow::Result<Ui> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let ui = UiShared::new(commands, port, initial);
    let app = router(ui.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok(ui)
}

/// Ouvre la fenêtre : Edge ou Chrome en mode application (fenêtre sans barre d'adresse, comme un vrai logiciel) ; sinon le navigateur par défaut.
pub fn open_window(url: &str) {
    use std::process::{Command, Stdio};
    let app_arg = format!("--app={url}");
    let size = "--window-size=480,800";
    let quiet = |mut command: Command| command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok();

    #[cfg(windows)]
    {
        let roots = ["ProgramFiles(x86)", "ProgramFiles", "LocalAppData"].map(|name| std::env::var(name).unwrap_or_default());
        let candidates = [r"Microsoft\Edge\Application\msedge.exe", r"Google\Chrome\Application\chrome.exe"];
        for relative in candidates {
            for root in roots.iter().filter(|r| !r.is_empty()) {
                let path = std::path::Path::new(root).join(relative);
                if path.exists() {
                    let mut command = Command::new(path);
                    command.args([app_arg.as_str(), size]);
                    if quiet(command) {
                        return;
                    }
                }
            }
        }
        let mut command = Command::new("explorer");
        command.arg(url);
        quiet(command);
    }
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("open");
        command.arg(url);
        quiet(command);
        let _ = (&app_arg, size);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        for browser in ["microsoft-edge", "google-chrome", "chromium", "chromium-browser"] {
            let mut command = Command::new(browser);
            command.args([app_arg.as_str(), size]);
            if quiet(command) {
                return;
            }
        }
        let mut command = Command::new("xdg-open");
        command.arg(url);
        quiet(command);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tokio::sync::mpsc;
    use tower::ServiceExt;

    fn setup() -> (Ui, mpsc::UnboundedReceiver<UiCommand>, Router) {
        let (tx, rx) = mpsc::unbounded_channel();
        let ui = UiShared::new(tx, 4242, UiState { device_name: "Salon".into(), link: "online".into(), ..Default::default() });
        let router = router(ui.clone());
        (ui, rx, router)
    }

    fn request(method: &str, path: &str, host: &str, token: Option<&str>, body: &str) -> axum::http::Request<Body> {
        let mut builder = axum::http::Request::builder().method(method).uri(path).header("host", host).header("content-type", "application/json");
        if let Some(token) = token {
            builder = builder.header("x-token", token);
        }
        builder.body(Body::from(body.to_string())).unwrap()
    }

    #[test]
    fn only_the_agents_own_address_is_accepted_as_host() {
        assert!(host_allowed(Some("127.0.0.1:4242"), 4242));
        assert!(host_allowed(Some("localhost:4242"), 4242));
        assert!(!host_allowed(Some("evil.example:4242"), 4242));      // DNS rebinding : un nom de domaine qui pointe sur 127.0.0.1
        assert!(!host_allowed(Some("127.0.0.1:9999"), 4242));
        assert!(!host_allowed(None, 4242));
    }

    #[test]
    fn the_token_comparison_is_exact() {
        assert!(token_matches(Some("abc"), "abc"));
        assert!(!token_matches(Some("abd"), "abc"));
        assert!(!token_matches(Some("ab"), "abc"));
        assert!(!token_matches(None, "abc"));
    }

    #[tokio::test]
    async fn every_api_call_needs_the_token_and_the_right_host() {
        let (ui, mut rx, app) = setup();
        let status = |response: Response| response.status();
        assert_eq!(status(app.clone().oneshot(request("GET", "/api/state", "127.0.0.1:4242", None, "")).await.unwrap()), StatusCode::UNAUTHORIZED);
        assert_eq!(status(app.clone().oneshot(request("GET", "/api/state", "127.0.0.1:4242", Some("faux"), "")).await.unwrap()), StatusCode::UNAUTHORIZED);
        assert_eq!(status(app.clone().oneshot(request("GET", "/api/state", "evil.example", Some(&ui.token), "")).await.unwrap()), StatusCode::FORBIDDEN);
        let consent = r#"{"session_id":"sess_x","accept":true}"#;
        assert_eq!(status(app.clone().oneshot(request("POST", "/api/consent", "127.0.0.1:4242", None, consent)).await.unwrap()), StatusCode::UNAUTHORIZED);
        assert!(rx.try_recv().is_err(), "aucun ordre ne doit passer sans le jeton");
        let ok = app.clone().oneshot(request("POST", "/api/consent", "127.0.0.1:4242", Some(&ui.token), consent)).await.unwrap();
        assert_eq!(ok.status(), StatusCode::NO_CONTENT);
        assert_eq!(rx.try_recv().unwrap(), UiCommand::Consent { session_id: "sess_x".into(), accept: true });
    }

    #[tokio::test]
    async fn the_page_is_served_without_data_and_the_state_with_the_token() {
        let (ui, _rx, app) = setup();
        let page = app.clone().oneshot(request("GET", "/", "127.0.0.1:4242", None, "")).await.unwrap();
        assert_eq!(page.status(), StatusCode::OK);
        assert_eq!(page.headers()["x-frame-options"], "DENY");
        let body = axum::body::to_bytes(page.into_body(), 1 << 20).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("<html") && !String::from_utf8_lossy(&body).contains(&ui.token));
        let state = app.oneshot(request("GET", "/api/state?v=1", "127.0.0.1:4242", Some(&ui.token), "")).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(state.into_body(), 1 << 20).await.unwrap()).unwrap();
        assert_eq!(json["device_name"], "Salon");
        assert!(ui.window_visible(), "interroger l'état en se disant visible marque la fenêtre comme vue");
    }

    #[tokio::test]
    async fn commands_reach_the_agent_with_their_content() {
        let (ui, mut rx, app) = setup();
        let post = |path: &'static str, body: &'static str| request("POST", path, "127.0.0.1:4242", Some(Box::leak(ui.token.clone().into_boxed_str())), body);
        for (path, body) in [("/api/stop", "{}"), ("/api/allow", r#"{"mouse":true,"keyboard":false}"#),
                             ("/api/setup", r#"{"server":"192.168.1.20:8000","api_key":"k"}"#), ("/api/pairing-code", "{}"), ("/api/quit", "{}")] {
            assert_eq!(app.clone().oneshot(post(path, body)).await.unwrap().status(), StatusCode::NO_CONTENT);
        }
        assert_eq!(rx.try_recv().unwrap(), UiCommand::StopSession);
        assert_eq!(rx.try_recv().unwrap(), UiCommand::SetAllow { mouse: true, keyboard: false });
        assert_eq!(rx.try_recv().unwrap(), UiCommand::Configure { server: "192.168.1.20:8000".into(), api_key: "k".into() });
        assert_eq!(rx.try_recv().unwrap(), UiCommand::NewPairingCode);
        assert_eq!(rx.try_recv().unwrap(), UiCommand::Quit);
        assert_eq!(app.oneshot(post("/api/consent", "pas du json")).await.unwrap().status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn the_event_log_keeps_only_the_latest_lines_and_visibility_expires() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let ui = UiShared::new(tx, 1, UiState::default());
        for i in 0..20 {
            ui.log(format!("ligne {i}"));
        }
        let events = ui.snapshot().events;
        assert_eq!((events.len(), events.last().unwrap().1.as_str()), (MAX_EVENTS, "ligne 19"));
        assert!(!ui.window_visible());
        *ui.last_visible.lock().unwrap() = Some(Instant::now() - Duration::from_secs(10));
        assert!(!ui.window_visible());
    }
}
