"""Serveur de rendez-vous (remote-rendezvous/) : clés séparées agent / contrôleur, parcours complet par WebSocket, et mode relais du plugin."""
import importlib.util
import threading
import time
from pathlib import Path

import httpx
import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient

import security
from plugins.remote import adapters, chat_handler, identity, proxy
from plugins.remote.identity import ReplayGuard
from plugins.remote.pairing import PairingBook
from plugins.remote.gateway import hub
from plugins.remote.ports import PORT_AUDIT, PORT_STORE
from plugin_sdk.ports import ports
from plugins.remote.service import service
from tests.test_remote_plugin import DEVICE, Agent

APP_PATH = Path(__file__).resolve().parent.parent.parent / "remote-rendezvous" / "app.py"
AGENT_KEY = "a" * 40
CONTROLLER_KEY = "c" * 40


def load_module():
    spec = importlib.util.spec_from_file_location("rendezvous_app_under_test", APP_PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@pytest.fixture
def rendezvous(monkeypatch):
    """Application de rendez-vous en mémoire : appareils et journal en mémoire, aucun agent connecté."""
    module = load_module()
    monkeypatch.setattr(service, "sessions", {})
    monkeypatch.setattr(service, "pairing", PairingBook())
    monkeypatch.setattr(service, "replay", ReplayGuard())
    monkeypatch.setattr(security.settings, "REMOTE_PAIR_RATE_LIMIT", 50, raising=False)
    security.reset_rate_limits()
    hub.connections.clear()
    with ports.override(PORT_STORE, adapters.MemoryDeviceStore()), ports.override(PORT_AUDIT, adapters.MemoryAuditSink()):
        yield module
    hub.connections.clear()


def clients(module):
    app = module.create_app(AGENT_KEY, CONTROLLER_KEY)
    return TestClient(app, headers={"x-api-key": AGENT_KEY}), TestClient(app, headers={"x-api-key": CONTROLLER_KEY}), TestClient(app)


# -- configuration -------------------------------------------------------------------------------------------------------------
def test_the_server_refuses_to_start_with_missing_short_or_identical_keys(rendezvous):
    for agent, controller in (("", CONTROLLER_KEY), (AGENT_KEY, "court"), (AGENT_KEY, AGENT_KEY)):
        with pytest.raises(rendezvous.ConfigurationError):
            rendezvous.create_app(agent, controller)
    rendezvous.create_app(AGENT_KEY, CONTROLLER_KEY)


def test_health_is_open_and_nothing_else_is_advertised(rendezvous):
    _, _, anonymous = clients(rendezvous)
    assert anonymous.get("/health").json() == {"status": "ok"}
    for path in ("/docs", "/redoc", "/openapi.json", "/"):
        assert anonymous.get(path).status_code == 404
    assert anonymous.get("/api/remote/devices").status_code == 401
    assert anonymous.post("/api/remote/pair", json={"code": "123456"}).status_code == 401


# -- clés séparées -------------------------------------------------------------------------------------------------------------
def test_an_agent_key_cannot_do_what_only_the_controller_may_and_the_reverse(rendezvous):
    agent_client, controller_client, _ = clients(rendezvous)
    agent = Agent()
    body = agent.register_args()
    assert controller_client.post("/api/remote/devices/register", json=body).status_code == 403     # le contrôleur n'enregistre pas d'appareil
    registered = agent_client.post("/api/remote/devices/register", json=agent.register_args())
    assert registered.status_code == 200
    for method, path in (("get", "/api/remote/devices"), ("get", "/api/remote/status"), ("get", "/api/remote/ice-servers")):
        assert getattr(agent_client, method)(path).status_code == 403, path                             # une clé d'agent qui fuit ne liste rien
    assert agent_client.post("/api/remote/pair", json={"code": registered.json()["pairing_code"]}).status_code == 403
    assert agent_client.post("/api/remote/sessions", json={"device_id": DEVICE}).status_code == 403
    devices = controller_client.get("/api/remote/devices")
    assert devices.status_code == 200 and devices.json()["devices"][0]["name"] == "Bureau"
    paired = controller_client.post("/api/remote/pair", json={"code": registered.json()["pairing_code"]})
    assert paired.json()["device"]["paired"] is True
    assert TestClient(controller_client.app, headers={"x-api-key": "x" * 40}).get("/api/remote/devices").status_code == 403


def test_the_agent_websocket_needs_the_agent_key_not_the_controller_key(rendezvous):
    agent_client, controller_client, _ = clients(rendezvous)
    agent = Agent()
    code = agent_client.post("/api/remote/devices/register", json=agent.register_args()).json()["pairing_code"]
    controller_client.post("/api/remote/pair", json={"code": code})
    url = f"/api/remote/agent/ws?device_id={agent.device_id}"
    with pytest.raises(Exception):
        with controller_client.websocket_connect(url):
            pass
    with agent_client.websocket_connect(url) as ws:
        assert ws.receive_json()["type"] == "challenge"


# -- parcours complet par le serveur de rendez-vous ----------------------------------------------------------------------------
def test_a_full_session_through_the_rendezvous_server(rendezvous):
    module = rendezvous
    app = module.create_app(AGENT_KEY, CONTROLLER_KEY)
    agent = Agent()
    with TestClient(app) as http:
        agent_headers, controller_headers = {"x-api-key": AGENT_KEY}, {"x-api-key": CONTROLLER_KEY}
        code = http.post("/api/remote/devices/register", json=agent.register_args(), headers=agent_headers).json()["pairing_code"]
        assert http.post("/api/remote/pair", json={"code": code}, headers=controller_headers).status_code == 200
        with http.websocket_connect(f"/api/remote/agent/ws?device_id={agent.device_id}", headers=agent_headers) as ws:
            challenge = ws.receive_json()
            ws.send_json({"type": "auth", "signature": agent.sign(identity.agent_auth_message(agent.device_id, challenge["nonce"]))})
            assert ws.receive_json()["type"] == "ready"
            assert http.get("/api/remote/devices", headers=controller_headers).json()["devices"][0]["online"] is True
            result: dict = {}
            opener = threading.Thread(target=lambda: result.update(http.post("/api/remote/sessions", json={"device_id": agent.device_id},
                                                                             headers=controller_headers).json()))
            opener.start()
            request = ws.receive_json()
            assert request["type"] == "open_session"
            sid = request["session_id"]
            ws.send_json({"type": "session_reply", "session_id": sid, "accepted": True})
            opener.join(5)
            assert result["state"] == "CONNECTING"
            http.post("/api/remote/signaling/offer", json={"session_id": sid, "sdp": "v=0 offer"}, headers=controller_headers)
            assert ws.receive_json() == {"type": "offer", "session_id": sid, "sdp": "v=0 offer"}
            session = service.session(sid)
            ws.send_json({"type": "answer", "session_id": sid, "sdp": "v=0 answer", "signature": agent.answer(session)})
            for _ in range(50):
                view = http.get(f"/api/remote/sessions/{sid}/signaling", headers=controller_headers).json()
                if view["state"] == "CONNECTED":
                    break
                time.sleep(0.05)
            assert view["state"] == "CONNECTED" and view["answer"] == "v=0 answer"


# -- mode relais du plugin (accès par Internet) --------------------------------------------------------------------------------
@pytest.fixture
def relay(rendezvous, monkeypatch):
    """Plugin Remote de PC Assistant en mode relais, relié en mémoire à l'application de rendez-vous."""
    app = rendezvous.create_app(AGENT_KEY, CONTROLLER_KEY)
    monkeypatch.setattr(security.settings, "API_AUTH_TOKEN", "cle-locale", raising=False)
    monkeypatch.setattr(proxy.settings, "REMOTE_RENDEZVOUS_URL", "http://rendezvous.test/", raising=False)
    monkeypatch.setattr(proxy.settings, "REMOTE_RENDEZVOUS_KEY", CONTROLLER_KEY, raising=False)
    proxy.set_client(httpx.AsyncClient(transport=httpx.ASGITransport(app=app), timeout=10))
    local = FastAPI()
    local.include_router(proxy.build_router(), prefix="/api/remote")
    with TestClient(local, headers={"x-api-key": "cle-locale"}) as http:
        yield type("Relay", (), {"http": http, "rendezvous": TestClient(app, headers={"x-api-key": AGENT_KEY}), "app": app})
    proxy.set_client(None)


def test_the_relay_serves_the_same_routes_and_keeps_the_server_key_server_side(relay):
    assert TestClient(relay.http.app).get("/api/remote/devices").status_code == 401            # la clé LOCALE reste exigée
    agent = Agent()
    code = relay.rendezvous.post("/api/remote/devices/register", json=agent.register_args()).json()["pairing_code"]
    paired = relay.http.post("/api/remote/pair", json={"code": code})
    assert paired.status_code == 200 and paired.json()["device"]["paired"] is True
    assert relay.http.get("/api/remote/devices").json()["devices"][0]["name"] == "Bureau"
    status = relay.http.get("/api/remote/status").json()
    assert status["mode"] == "rendezvous" and status["server"] == "http://rendezvous.test"
    assert relay.http.get("/api/remote/devices/" + agent.device_id).json()["paired"] is True
    assert CONTROLLER_KEY not in relay.http.get("/api/remote/status").text


def test_the_relay_never_forwards_agent_routes(relay):
    agent = Agent()
    assert relay.http.post("/api/remote/devices/register", json=agent.register_args()).status_code == 404
    assert relay.http.get("/api/remote/agent/ws").status_code == 404
    assert not relay.rendezvous.get("/api/remote/devices").is_success       # rien n'a été enregistré : la clé d'agent ne liste pas, et rien n'est passé


def test_the_relay_passes_server_errors_through_and_reports_an_unreachable_server(relay, monkeypatch):
    assert relay.http.post("/api/remote/pair", json={"code": "000000"}).status_code == 400
    assert "invalide" in relay.http.post("/api/remote/pair", json={"code": "000000"}).json()["detail"]
    monkeypatch.setattr(proxy.settings, "REMOTE_RENDEZVOUS_KEY", "z" * 40, raising=False)
    assert relay.http.get("/api/remote/devices").status_code == 403                  # mauvaise clé de contrôleur : l'erreur du serveur est relayée

    def refuse(request):
        raise httpx.ConnectError("refusé")
    proxy.set_client(httpx.AsyncClient(transport=httpx.MockTransport(refuse)))
    down = relay.http.get("/api/remote/devices")
    assert down.status_code == 503 and "injoignable" in down.json()["detail"]


def test_chat_works_through_the_rendezvous_server(relay):
    import asyncio

    ask = lambda text: asyncio.run(chat_handler.handle(text, {}))["response"]      # noqa: E731
    assert "Aucun appareil appairé" in ask("quels pc distants")
    agent = Agent()
    code = relay.rendezvous.post("/api/remote/devices/register", json=agent.register_args()).json()["pairing_code"]
    relay.http.post("/api/remote/pair", json={"code": code})
    assert "Bureau (hors ligne)" in ask("statut remote")
    assert "Je ne connais pas d'appareil nommé « salon »" in ask("connecte-moi au PC du salon")
    assert "hors ligne" in ask("connecte-moi au PC du bureau")                      # appareil connu mais son agent n'est pas connecté
    assert "Aucune session" in ask("déconnecte la session remote")
