"""Plugin Remote (squelette) : machine d'état, permissions, appairage, sessions via les ports, API protégée et chat. Aucun vrai agent : LoopbackAgentGateway."""
import asyncio
import json
from pathlib import Path

import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient

import security
from plugin_sdk.manifest import validate
from plugin_sdk.ports import ports
from plugin_sdk.router import unprotected_routes
from plugins.remote import adapters, chat_handler, permissions, states
from plugins.remote.errors import AgentUnavailable, InvalidState, NotFound, PairingFailed, PairingLocked, PermissionDenied
from plugins.remote.pairing import PairingBook
from plugins.remote.permissions import Permission
from plugins.remote.ports import PORT_AGENT, PORT_AUDIT, PORT_STORE
from plugins.remote.router import router
from plugins.remote.service import service
from plugins.remote.states import SessionState

PLUGIN = Path(__file__).resolve().parent.parent / "plugins" / "remote"
KEY = "cle-publique-de-test-0123456789"
DEVICE = "dev_bureau_0001"


def run(coro):
    return asyncio.run(coro)


class Clock:
    def __init__(self):
        self.now = 1000.0

    def __call__(self):
        return self.now


@pytest.fixture
def remote(monkeypatch):
    """Service neuf, agent simulé (appareil `DEVICE` en ligne), stockage et journal en mémoire."""
    gateway, store, audit = adapters.LoopbackAgentGateway(online={DEVICE}), adapters.MemoryDeviceStore(), adapters.MemoryAuditSink()
    monkeypatch.setattr(service, "sessions", {})
    monkeypatch.setattr(service, "pairing", PairingBook())
    security.reset_rate_limits()
    with ports.override(PORT_AGENT, gateway), ports.override(PORT_STORE, store), ports.override(PORT_AUDIT, audit):
        yield type("Remote", (), {"gateway": gateway, "store": store, "audit": audit, "service": service})


def paired(remote, device_id=DEVICE, name="Bureau"):
    _, code = remote.service.register_device(device_id, name, KEY)
    return remote.service.pair(code)


# -- machine d'état, permissions, appairage ---------------------------------------------------------------------------------
def test_state_machine_allows_only_the_documented_transitions():
    S = SessionState
    assert states.can_transition(S.CREATED, S.WAITING) and states.can_transition(S.CONNECTED, S.RECONNECTING) and states.can_transition(S.RECONNECTING, S.CONNECTED)
    assert not states.can_transition(S.CREATED, S.CONNECTED) and not states.can_transition(S.DISCONNECTED, S.CONNECTED)
    assert all(states.can_transition(s, S.DISCONNECTED) for s in S if s is not S.DISCONNECTED)
    with pytest.raises(InvalidState):
        states.ensure_transition(S.WAITING, S.CONNECTED)


def test_permissions_are_separate_and_only_v1_can_be_granted():
    assert permissions.parse(["view_screen", "control_mouse"]) == {Permission.VIEW_SCREEN, Permission.CONTROL_MOUSE}
    with pytest.raises(PermissionDenied, match="pas encore disponible"):
        permissions.parse(["terminal"])
    with pytest.raises(Exception, match="inconnue"):
        permissions.parse(["root"])
    with pytest.raises(PermissionDenied, match="control_keyboard"):
        permissions.ensure_granted(frozenset({Permission.CONTROL_KEYBOARD}), frozenset({Permission.VIEW_SCREEN}))


def test_pairing_codes_expire_work_once_and_lock_after_repeated_failures():
    clock = Clock()
    book = PairingBook(clock)
    code = book.issue("dev_a")
    assert len(code) == 6 and code.isdigit()
    assert book.redeem(code) == "dev_a"
    with pytest.raises(PairingFailed):
        book.redeem(code)                                  # usage unique
    old = book.issue("dev_b")
    clock.now += 301
    with pytest.raises(PairingFailed):
        book.redeem(old)                                   # expiré
    fresh = book.issue("dev_c")
    assert book.issue("dev_c") != fresh and "dev_c" in {v[0] for v in book._codes.values()} and len(book._codes) == 1   # un nouveau code remplace l'ancien
    assert book.redeem(book.issue("dev_c")) == "dev_c"      # une réussite remet le compteur d'échecs à zéro
    for _ in range(4):
        with pytest.raises(PairingFailed):
            book.redeem("000000")
    with pytest.raises(PairingFailed):
        book.redeem("000000")                              # 5e échec : verrouillage
    with pytest.raises(PairingLocked):
        book.redeem(book.issue("dev_c"))                   # même le bon code est refusé pendant le verrouillage
    clock.now += 61
    assert book.redeem(book.issue("dev_c")) == "dev_c"


# -- service : appairage et sessions -----------------------------------------------------------------------------------------
def test_pairing_registers_the_device_and_a_paired_device_cannot_be_taken_over(remote):
    device = paired(remote)
    assert device.paired and device.granted == {Permission.VIEW_SCREEN} and device.fingerprint()
    assert remote.audit.names() == ["PAIRING_REQUESTED", "PAIRING_ACCEPTED"]
    with pytest.raises(InvalidState, match="autre identité"):
        remote.service.register_device(DEVICE, "Pirate", "une-autre-cle-publique-9876543210")
    assert remote.audit.names()[-1] == "AUTHENTICATION_FAILED"
    with pytest.raises(PairingFailed):
        remote.service.pair("123456")
    assert "key" not in json.dumps(run(remote.service.list_devices())) and run(remote.service.list_devices())[0]["online"] is True


def test_full_session_flow_with_audit(remote):
    paired(remote)
    remote.store.get(DEVICE).granted = frozenset({Permission.VIEW_SCREEN, Permission.CONTROL_MOUSE})
    session = run(remote.service.create_session(DEVICE, ["view_screen", "control_mouse"]))
    assert session.state is SessionState.CONNECTING and remote.gateway.opened == [session.session_id]
    with pytest.raises(InvalidState, match="déjà ouverte"):
        run(remote.service.create_session(DEVICE))
    with pytest.raises(InvalidState, match="Pas d'offre"):
        remote.service.submit_answer(session.session_id, "v=0 answer")
    remote.service.submit_offer(session.session_id, "v=0 offer")
    remote.service.add_ice(session.session_id, "candidate:1 1 udp ...")
    assert remote.service.submit_answer(session.session_id, "v=0 answer").state is SessionState.CONNECTED
    run(remote.service.close_session(session.session_id))
    assert remote.service.session(session.session_id).state is SessionState.DISCONNECTED and remote.gateway.closed == [session.session_id]
    assert remote.audit.names() == ["PAIRING_REQUESTED", "PAIRING_ACCEPTED", "SESSION_CREATED", "AUTHENTICATION_SUCCESS", "REMOTE_CONNECTED",
                                    "MOUSE_CONTROL_ENABLED", "REMOTE_DISCONNECTED"]
    run(remote.service.close_session(session.session_id))                       # idempotent
    assert run(remote.service.create_session(DEVICE)).state is SessionState.CONNECTING   # une nouvelle session est possible après la fin


def test_a_session_never_exceeds_the_granted_permissions_and_needs_a_paired_online_device(remote):
    remote.service.register_device(DEVICE, "Bureau", KEY)
    with pytest.raises(PermissionDenied, match="pas appairé"):
        run(remote.service.create_session(DEVICE))
    remote.service.pair(next(iter(remote.service.pairing._codes)))
    with pytest.raises(PermissionDenied, match="control_keyboard"):
        run(remote.service.create_session(DEVICE, ["control_keyboard"]))
    with pytest.raises(NotFound):
        run(remote.service.create_session("dev_inconnu_01"))
    remote.gateway.online.clear()
    with pytest.raises(AgentUnavailable, match="hors ligne"):
        run(remote.service.create_session(DEVICE))
    assert not remote.service.sessions and "SESSION_REFUSED" in remote.audit.names()


def test_an_agent_that_refuses_ends_the_session_and_is_audited(remote):
    paired(remote)
    remote.gateway.refuse.add(DEVICE)
    with pytest.raises(PermissionDenied, match="refusé"):
        run(remote.service.create_session(DEVICE))
    assert next(iter(remote.service.sessions.values())).state is SessionState.DISCONNECTED and "PERMISSION_DENIED" in remote.audit.names()


def test_without_an_installed_agent_no_session_can_open(monkeypatch):
    store, audit = adapters.MemoryDeviceStore(), adapters.MemoryAuditSink()
    monkeypatch.setattr(service, "sessions", {})
    monkeypatch.setattr(service, "pairing", PairingBook())
    with ports.override(PORT_STORE, store), ports.override(PORT_AUDIT, audit):
        _, code = service.register_device(DEVICE, "Bureau", KEY)
        service.pair(code)
        assert not service.agent_installed()
        with pytest.raises(AgentUnavailable, match="pas installé|hors ligne"):
            run(service.create_session(DEVICE))


def test_signaling_only_relays_bounded_text_in_the_right_state(remote):
    paired(remote)
    session = run(remote.service.create_session(DEVICE))
    with pytest.raises(InvalidState):
        remote.service.submit_offer(session.session_id, "   ")
    with pytest.raises(InvalidState):
        remote.service.submit_offer(session.session_id, "x" * 70_000)
    with pytest.raises(InvalidState):
        remote.service.add_ice(session.session_id, "c" * 3000)
    run(remote.service.close_session(session.session_id))
    with pytest.raises(InvalidState, match="signaling impossible"):
        remote.service.submit_offer(session.session_id, "v=0")


def test_json_device_store_persists_public_data_only(tmp_path):
    path = tmp_path / "devices.json"
    store = adapters.JsonDeviceStore(path)
    from plugins.remote.models import Device
    store.save(Device(DEVICE, "Bureau", KEY, paired=True, granted=frozenset({Permission.VIEW_SCREEN})))
    again = adapters.JsonDeviceStore(path).get(DEVICE)
    assert again and again.paired and again.granted == {Permission.VIEW_SCREEN} and again.public_key == KEY
    path.write_text("pas du json", encoding="utf-8")
    assert adapters.JsonDeviceStore(path).list() == []        # fichier illisible : on repart de zéro sans planter


# -- API ---------------------------------------------------------------------------------------------------------------------
@pytest.fixture
def client(remote, monkeypatch):
    monkeypatch.setattr(security.settings, "API_AUTH_TOKEN", "secret")
    monkeypatch.setattr(security.settings, "REMOTE_PAIR_RATE_LIMIT", 5, raising=False)
    app = FastAPI()
    app.include_router(router, prefix="/api/remote")
    http = TestClient(app, headers={"x-api-key": "secret"})
    http.app_instance = app
    return http


def test_every_remote_route_requires_the_api_key(client):
    assert unprotected_routes(client.app_instance) == []
    anonymous = TestClient(client.app_instance)
    assert anonymous.get("/api/remote/devices").status_code == 401 and anonymous.post("/api/remote/pair", json={"code": "123456"}).status_code == 401


def test_http_flow_from_registration_to_disconnection(client):
    body = {"device_id": DEVICE, "name": "Bureau", "public_key": KEY}
    registered = client.post("/api/remote/devices/register", json=body).json()
    code = registered["pairing_code"]
    assert registered["expires_in"] == 300 and "public_key" not in registered["device"]
    assert client.post("/api/remote/pair", json={"code": code}).json()["device"]["paired"] is True
    assert client.get("/api/remote/devices").json()["devices"][0]["online"] is True
    assert client.get(f"/api/remote/devices/{DEVICE}").json()["name"] == "Bureau"
    session = client.post("/api/remote/sessions", json={"device_id": DEVICE, "permissions": ["view_screen"]}).json()
    sid = session["session_id"]
    assert session["state"] == "CONNECTING" and client.get("/api/remote/status").json()["agent_installed"] is True
    assert client.post("/api/remote/signaling/offer", json={"session_id": sid, "sdp": "v=0 offer"}).status_code == 200
    assert client.post("/api/remote/signaling/ice", json={"session_id": sid, "candidate": "candidate:1"}).json()["ice_candidates"] == 1
    assert client.post("/api/remote/signaling/answer", json={"session_id": sid, "sdp": "v=0 answer"}).json()["state"] == "CONNECTED"
    assert client.delete(f"/api/remote/sessions/{sid}").json()["state"] == "DISCONNECTED"
    assert client.get("/api/remote/sessions/sess_inconnue").status_code == 404


def test_http_input_is_validated_and_pairing_attempts_are_limited(client):
    assert client.post("/api/remote/devices/register", json={"device_id": "x", "name": "n", "public_key": "court"}).status_code == 422
    assert client.post("/api/remote/pair", json={"code": "12"}).status_code == 422
    assert client.post("/api/remote/sessions", json={"device_id": DEVICE, "permissions": ["terminal"]}).status_code in (403, 404)
    codes = [client.post("/api/remote/pair", json={"code": "000000"}).status_code for _ in range(5)]
    assert codes[:3] == [400, 400, 400] and codes[3:] == [429, 429]       # 2 requêtes invalides + 3 essais = 5 par minute ; au-delà : refus


# -- chat --------------------------------------------------------------------------------------------------------------------
def test_chat_phrases_come_from_intents_json_and_examples_are_recognised():
    for name in chat_handler.INTENTS.names():
        spec = chat_handler.INTENTS.spec(name)
        for example in spec["examples"]:
            assert chat_handler.INTENTS.is_a(example, name), example
        for negative in spec["negatives"]:
            assert not chat_handler.matches(negative), negative


def test_chat_goes_through_the_same_service_rules(remote):
    ask = lambda text: run(chat_handler.handle(text, {}))["response"]      # noqa: E731
    assert "Aucun appareil appairé" in ask("quels pc distants")
    assert "Je ne connais pas d'appareil nommé « bureau »" in ask("connecte-moi au PC du bureau")
    paired(remote)
    assert "Bureau (en ligne)" in ask("statut remote")
    assert "Connexion à Bureau en cours" in ask("connecte-moi au PC du bureau")
    assert "déjà ouverte" in ask("prends le contrôle du PC bureau")
    assert ask("déconnecte la session remote") == "Session coupée." and "Aucune session" in ask("coupe le contrôle à distance")
    remote.gateway.online.clear()
    assert "hors ligne" in ask("connecte-moi au PC du bureau")


def test_the_manifest_is_valid_and_the_plugin_starts_disabled():
    manifest = json.loads((PLUGIN / "manifest.json").read_text(encoding="utf-8"))
    assert validate(manifest, PLUGIN).ok and manifest["enabled_by_default"] is False and manifest["permission"] == "EXTERNAL"
    assert validate(manifest, PLUGIN).advice == []
