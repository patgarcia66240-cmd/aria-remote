"""Plugin Remote (squelette) : machine d'état, permissions, appairage, sessions via les ports, API protégée et chat. Aucun vrai agent : LoopbackAgentGateway."""
import asyncio
import base64
import hashlib
import hmac
import json
import threading
import time
from pathlib import Path

import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from fastapi import FastAPI
from fastapi.testclient import TestClient

import security
from plugin_sdk.manifest import validate
from plugin_sdk.ports import ports
from plugin_sdk.router import unprotected_routes
from plugins.remote import adapters, chat_handler, ice, identity, permissions, states
from plugins.remote.gateway import AgentConnection, AgentHub, hub
from plugins.remote.errors import AgentUnavailable, InvalidState, NotFound, PairingFailed, PairingLocked, PermissionDenied
from plugins.remote.pairing import PairingBook
from plugins.remote.permissions import Permission
from plugins.remote.ports import PORT_AGENT, PORT_AUDIT, PORT_STORE
from plugins.remote.router import router
from plugins.remote.service import service
from plugins.remote.states import SessionState

PLUGIN = Path(__file__).resolve().parent.parent / "plugins" / "remote"
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
    monkeypatch.setattr(service, "replay", identity.ReplayGuard())
    security.reset_rate_limits()
    with ports.override(PORT_AGENT, gateway), ports.override(PORT_STORE, store), ports.override(PORT_AUDIT, audit):
        yield type("Remote", (), {"gateway": gateway, "store": store, "audit": audit, "service": service})


class Agent:
    """Appareil de test : sa clé privée ne sort jamais d'ici, comme sur le vrai agent."""

    def __init__(self, device_id=DEVICE, name="Bureau"):
        self.key = Ed25519PrivateKey.generate()
        self.device_id, self.name = device_id, name
        self.public_key = base64.b64encode(self.key.public_key().public_bytes_raw()).decode()

    def sign(self, message: bytes) -> str:
        return base64.b64encode(self.key.sign(message)).decode()

    def register_args(self, *, timestamp=None, nonce=None, name=None) -> dict:
        timestamp = int(time.time()) if timestamp is None else timestamp
        nonce = nonce or identity.new_nonce()
        name = name or self.name
        return {"device_id": self.device_id, "name": name, "public_key": self.public_key, "platform": "windows", "timestamp": timestamp, "nonce": nonce,
                "signature": self.sign(identity.register_message(self.device_id, name, timestamp, nonce))}

    def answer(self, session, sdp="v=0 answer") -> str:
        return self.sign(identity.answer_message(session.session_id, session.offer, sdp))


def paired(remote, agent=None):
    agent = agent or Agent()
    _, code = remote.service.register_device(**agent.register_args())
    remote.service.pair(code)
    return agent


def connect(remote, agent, permissions_=None):
    """Session ouverte jusqu'à CONNECTED, avec une réponse correctement signée."""
    session = run(remote.service.create_session(agent.device_id, permissions_))
    run(remote.service.submit_offer(session.session_id, "v=0 offer"))
    remote.service.submit_answer(session.session_id, "v=0 answer", agent.answer(session))
    return session


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


def test_the_pairing_code_lifetime_is_configurable_within_sane_bounds(monkeypatch):
    for configured, expected in ((600, 600), (5, 60), (10 ** 9, 86400), (1800, 1800)):
        monkeypatch.setattr(security.settings, "REMOTE_PAIRING_CODE_TTL", configured, raising=False)
        assert PairingBook().ttl == expected
    assert PairingBook(ttl=120).ttl == 120                 # un test ou un appelant peut aussi fixer la durée directement


def test_pairing_codes_expire_work_once_and_lock_after_repeated_failures():
    clock = Clock()
    book = PairingBook(clock)
    assert book.ttl == 1800                                # 30 minutes par défaut
    code = book.issue("dev_a")
    assert len(code) == 6 and code.isdigit()
    assert book.redeem(code) == "dev_a"
    with pytest.raises(PairingFailed):
        book.redeem(code)                                  # usage unique
    old = book.issue("dev_b")
    clock.now += 1801
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


# -- identité cryptographique ------------------------------------------------------------------------------------------------
def test_public_key_must_be_a_real_ed25519_key_and_signatures_must_match():
    agent = Agent()
    for bad in ("cle-publique-de-test-0123456789", base64.b64encode(b"court").decode(), ""):
        with pytest.raises(Exception, match="Clé publique invalide"):
            identity.parse_public_key(bad)
    signature = agent.sign(b"message")
    identity.verify(agent.public_key, b"message", signature)
    with pytest.raises(PermissionDenied):
        identity.verify(agent.public_key, b"autre message", signature)
    with pytest.raises(PermissionDenied):
        identity.verify(Agent().public_key, b"message", signature)
    with pytest.raises(PermissionDenied):
        identity.verify(agent.public_key, b"message", "pas-du-base64!")


def test_signed_messages_have_separate_purposes():
    agent = Agent()
    forged = agent.sign(identity.agent_auth_message(DEVICE, "n" * 20))
    with pytest.raises(PermissionDenied):
        identity.verify(agent.public_key, identity.answer_message("sess_x", "offre", "reponse"), forged)


def test_replay_guard_refuses_stale_future_and_reused_requests():
    clock = Clock()
    guard = identity.ReplayGuard(clock, window=120)
    guard.check(1000, "a" * 20)
    with pytest.raises(PermissionDenied, match="déjà utilisée"):
        guard.check(1000, "a" * 20)
    with pytest.raises(PermissionDenied, match="Horodatage"):
        guard.check(800, "b" * 20)
    with pytest.raises(PermissionDenied, match="Horodatage"):
        guard.check(1200, "c" * 20)
    clock.now += 300
    guard.check(1300, "a" * 20)          # la fenêtre est passée : le nonce a été oublié, mais l'horodatage protège encore


# -- service : appairage et sessions -----------------------------------------------------------------------------------------
def test_registration_needs_the_private_key_and_cannot_be_replayed(remote):
    agent = Agent()
    args = agent.register_args()
    remote.service.register_device(**args)
    with pytest.raises(PermissionDenied, match="déjà utilisée"):
        remote.service.register_device(**args)                                    # même requête rejouée
    forged = {**agent.register_args(), "signature": Agent().sign(b"x")}
    with pytest.raises(PermissionDenied, match="Signature"):
        remote.service.register_device(**forged)                                  # signée par quelqu'un d'autre
    stale = agent.register_args(timestamp=int(time.time()) - 3600)
    with pytest.raises(PermissionDenied, match="Horodatage"):
        remote.service.register_device(**stale)
    renamed = {**agent.register_args(), "name": "Pirate"}
    with pytest.raises(PermissionDenied, match="Signature"):
        remote.service.register_device(**renamed)                                 # le nom est signé lui aussi
    assert remote.audit.names().count("AUTHENTICATION_FAILED") == 4


def test_pairing_registers_the_device_and_a_paired_device_cannot_be_taken_over(remote):
    agent = paired(remote)
    device = remote.store.get(DEVICE)
    assert device.paired and device.granted == {Permission.VIEW_SCREEN} and device.fingerprint()
    assert remote.audit.names() == ["PAIRING_REQUESTED", "PAIRING_ACCEPTED"]
    pirate = Agent(name="Pirate")                  # même identifiant, VRAIE signature, mais une autre clé
    with pytest.raises(InvalidState, match="autre identité"):
        remote.service.register_device(**pirate.register_args())
    assert remote.audit.names()[-1] == "AUTHENTICATION_FAILED" and remote.store.get(DEVICE).public_key == agent.public_key
    with pytest.raises(PairingFailed):
        remote.service.pair("123456")
    assert "key" not in json.dumps(run(remote.service.list_devices())) and run(remote.service.list_devices())[0]["online"] is True


def test_an_unpaired_device_may_change_its_key_but_only_by_proving_the_new_one(remote):
    first, second = Agent(), Agent()
    remote.service.register_device(**first.register_args())
    remote.service.register_device(**second.register_args())
    assert remote.store.get(DEVICE).public_key == second.public_key


def test_full_session_flow_with_signed_answer_and_audit(remote):
    agent = paired(remote)
    remote.store.get(DEVICE).granted = frozenset({Permission.VIEW_SCREEN, Permission.CONTROL_MOUSE})
    session = run(remote.service.create_session(DEVICE, ["view_screen", "control_mouse"]))
    assert session.state is SessionState.CONNECTING and remote.gateway.opened == [session.session_id]
    with pytest.raises(InvalidState, match="déjà ouverte"):
        run(remote.service.create_session(DEVICE))
    with pytest.raises(InvalidState, match="Pas d'offre"):
        remote.service.submit_answer(session.session_id, "v=0 answer", agent.sign(b"x"))
    run(remote.service.submit_offer(session.session_id, "v=0 offer"))
    run(remote.service.add_ice(session.session_id, "candidate:1 1 udp ..."))
    assert remote.gateway.signals == [(session.session_id, "offer", "v=0 offer"), (session.session_id, "ice", "candidate:1 1 udp ...")]
    assert remote.service.submit_answer(session.session_id, "v=0 answer", agent.answer(session)).state is SessionState.CONNECTED
    run(remote.service.close_session(session.session_id))
    assert remote.service.session(session.session_id).state is SessionState.DISCONNECTED and remote.gateway.closed == [session.session_id]
    assert remote.audit.names() == ["PAIRING_REQUESTED", "PAIRING_ACCEPTED", "SESSION_CREATED", "AUTHENTICATION_SUCCESS", "REMOTE_CONNECTED",
                                    "MOUSE_CONTROL_ENABLED", "REMOTE_DISCONNECTED"]
    run(remote.service.close_session(session.session_id))                       # idempotent
    assert run(remote.service.create_session(DEVICE)).state is SessionState.CONNECTING   # une nouvelle session est possible après la fin


def test_an_answer_not_signed_by_the_device_is_refused_even_with_the_api_key(remote):
    agent = paired(remote)
    session = run(remote.service.create_session(DEVICE))
    run(remote.service.submit_offer(session.session_id, "v=0 offer"))
    intruder = Agent()
    for bad in (intruder.answer(session), agent.sign(b"autre chose"), agent.answer(session, sdp="v=0 autre reponse")):
        with pytest.raises(PermissionDenied, match="Signature"):
            remote.service.submit_answer(session.session_id, "v=0 answer", bad)
    assert session.state is SessionState.CONNECTING and not session.answer and remote.audit.names().count("AUTHENTICATION_FAILED") == 3
    with pytest.raises(PermissionDenied, match="n'appartient pas"):
        remote.service.submit_answer(session.session_id, "v=0 answer", agent.answer(session), device_id="dev_autre_0001")
    assert remote.service.submit_answer(session.session_id, "v=0 answer", agent.answer(session)).state is SessionState.CONNECTED


def test_a_session_never_exceeds_the_granted_permissions_and_needs_a_paired_online_device(remote):
    remote.service.register_device(**Agent().register_args())
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


def test_without_a_connected_agent_no_session_can_open(monkeypatch):
    store, audit = adapters.MemoryDeviceStore(), adapters.MemoryAuditSink()
    monkeypatch.setattr(service, "sessions", {})
    monkeypatch.setattr(service, "pairing", PairingBook())
    monkeypatch.setattr(service, "replay", identity.ReplayGuard())
    hub.connections.clear()
    with ports.override(PORT_STORE, store), ports.override(PORT_AUDIT, audit):
        assert not service.agent_installed()
        _, code = service.register_device(**Agent().register_args())
        service.pair(code)
        assert service.agent_installed()
        with pytest.raises(AgentUnavailable, match="hors ligne"):
            run(service.create_session(DEVICE))


def test_signaling_only_relays_bounded_text_in_the_right_state(remote):
    paired(remote)
    session = run(remote.service.create_session(DEVICE))
    with pytest.raises(InvalidState):
        run(remote.service.submit_offer(session.session_id, "   "))
    with pytest.raises(InvalidState):
        run(remote.service.submit_offer(session.session_id, "x" * 70_000))
    with pytest.raises(InvalidState):
        run(remote.service.add_ice(session.session_id, "c" * 3000))
    run(remote.service.close_session(session.session_id))
    with pytest.raises(InvalidState, match="signaling impossible"):
        run(remote.service.submit_offer(session.session_id, "v=0"))


# -- reconnexion (état RECONNECTING) -----------------------------------------------------------------------------------------
def test_a_dropped_link_reconnects_with_a_fresh_signed_negotiation(remote):
    agent = paired(remote)
    session = connect(remote, agent)
    old_answer = agent.answer(session)
    run(remote.service.reconnect(session.session_id))
    assert session.state is SessionState.RECONNECTING and not session.offer and not session.answer and not session.ice
    assert remote.gateway.signals[-1] == (session.session_id, "reconnect", "")
    run(remote.service.submit_offer(session.session_id, "v=0 offer 2"))
    with pytest.raises(PermissionDenied):
        remote.service.submit_answer(session.session_id, "v=0 answer", old_answer)         # l'ancienne réponse ne vaut pas pour la nouvelle offre
    assert remote.service.submit_answer(session.session_id, "v=0 answer", agent.answer(session)).state is SessionState.CONNECTED
    names = [n for n in remote.audit.names() if n.startswith("REMOTE_")]
    assert names == ["REMOTE_CONNECTED", "REMOTE_RECONNECTING", "REMOTE_RECONNECTED"] and remote.audit.names().count("AUTHENTICATION_FAILED") == 1


def test_reconnection_is_only_possible_from_connected_and_expires_after_the_grace_period(remote, monkeypatch):
    clock = Clock()
    monkeypatch.setattr(service, "clock", clock)
    agent = paired(remote)
    session = run(remote.service.create_session(DEVICE))
    with pytest.raises(InvalidState, match="reconnexion impossible"):
        run(remote.service.reconnect(session.session_id))
    connect_session = connect_after(remote, agent, session)
    run(remote.service.reconnect(connect_session.session_id))
    clock.now += service.reconnect_grace - 1
    assert remote.service.session(session.session_id).state is SessionState.RECONNECTING
    clock.now += 2
    run(remote.service.sweep())
    assert session.state is SessionState.DISCONNECTED and remote.gateway.closed == [session.session_id]
    assert remote.audit.names()[-1] == "REMOTE_DISCONNECTED" and run(remote.service.create_session(DEVICE)).state is SessionState.CONNECTING


def connect_after(remote, agent, session):
    run(remote.service.submit_offer(session.session_id, "v=0 offer"))
    remote.service.submit_answer(session.session_id, "v=0 answer", agent.answer(session))
    return session


def test_losing_the_agent_reconnects_an_established_session_and_drops_a_pending_one(remote):
    agent = paired(remote)
    pending = run(remote.service.create_session(DEVICE))
    remote.service.agent_lost(DEVICE)
    assert pending.state is SessionState.DISCONNECTED
    established = connect(remote, agent)
    remote.service.agent_lost(DEVICE)
    assert established.state is SessionState.RECONNECTING
    remote.service.agent_lost(DEVICE)                                           # idempotent
    assert established.state is SessionState.RECONNECTING


def test_a_session_survives_a_websocket_drop_when_the_agent_resumes_it(remote):
    agent = paired(remote)
    session = connect(remote, agent)
    remote.service.agent_lost(DEVICE)
    assert session.state is SessionState.RECONNECTING and session.answer          # la négociation est conservée : le lien peut être intact
    with pytest.raises(PermissionDenied):
        remote.service.agent_resumed(session.session_id, "dev_autre_0001")
    assert remote.service.agent_resumed(session.session_id, DEVICE).state is SessionState.CONNECTED
    with pytest.raises(InvalidState, match="Rien à reprendre"):
        remote.service.agent_resumed(session.session_id, DEVICE)                  # déjà reprise
    run(remote.service.reconnect(session.session_id))                              # le contrôleur renégocie : la négociation est effacée
    with pytest.raises(InvalidState, match="renégociée"):
        remote.service.agent_resumed(session.session_id, DEVICE)


def test_messages_match_the_agent_wire_format():
    """Mêmes octets que remote-agent/src/identity.rs (test messages_have_the_exact_wire_format_of_the_backend)."""
    assert identity.register_message("dev_bureau_0001", "Bureau", 1700000000, "abcdefghijklmnop") == \
        b"pc-assistant-remote-v1|register|dev_bureau_0001|Bureau|1700000000|abcdefghijklmnop"
    assert identity.agent_auth_message("dev_bureau_0001", "n0nce") == b"pc-assistant-remote-v1|agent|dev_bureau_0001|n0nce"
    assert identity.sha256_hex("abc") == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    assert identity.answer_message("sess_x", "offre", "reponse").decode() == \
        f"pc-assistant-remote-v1|answer|sess_x|{identity.sha256_hex('offre')}|{identity.sha256_hex('reponse')}"


def test_the_device_user_can_end_a_session_but_only_their_own(remote):
    agent = paired(remote)
    session = connect(remote, agent)
    with pytest.raises(PermissionDenied):
        remote.service.agent_ended(session.session_id, "dev_autre_0001")
    remote.service.agent_ended(session.session_id, DEVICE)
    assert session.state is SessionState.DISCONNECTED and remote.audit.events[-1][1]["result"] == "ended_by_device"


def test_only_the_device_decides_what_it_allows_and_only_v1_permissions(remote):
    paired(remote)
    assert remote.service.set_granted(DEVICE, ["view_screen", "control_mouse"]).granted == {Permission.VIEW_SCREEN, Permission.CONTROL_MOUSE}
    assert remote.audit.events[-1] == ("PERMISSION_GRANTED", {"device_id": DEVICE, "permissions": "control_mouse,view_screen"})
    with pytest.raises(PermissionDenied, match="pas encore disponible"):
        remote.service.set_granted(DEVICE, ["terminal"])
    assert remote.service.set_granted(DEVICE, []).granted == {Permission.VIEW_SCREEN}       # rien de demandé : on revient au minimum


def test_agent_ice_candidates_are_readable_by_the_controller_from_an_offset(remote):
    agent = paired(remote)
    session = run(remote.service.create_session(DEVICE))
    remote.service.add_agent_ice(session.session_id, "candidate:a", DEVICE)
    remote.service.add_agent_ice(session.session_id, "candidate:b", DEVICE)
    with pytest.raises(PermissionDenied):
        remote.service.add_agent_ice(session.session_id, "candidate:c", "dev_autre_0001")
    view = remote.service.signaling_view(session.session_id)
    assert view["ice"] == ["candidate:a", "candidate:b"] and view["next"] == 2 and view["answer"] == ""
    assert remote.service.signaling_view(session.session_id, 2)["ice"] == []
    run(remote.service.submit_offer(session.session_id, "v=0 offer"))
    remote.service.submit_answer(session.session_id, "v=0 answer", agent.answer(session))
    assert remote.service.signaling_view(session.session_id)["answer"] == "v=0 answer"


# -- STUN / TURN -------------------------------------------------------------------------------------------------------------
def test_no_third_party_server_is_contacted_unless_configured(monkeypatch):
    for name in ("REMOTE_STUN_URLS", "REMOTE_TURN_URLS", "REMOTE_TURN_SECRET", "REMOTE_TURN_USERNAME", "REMOTE_TURN_CREDENTIAL"):
        monkeypatch.setattr(ice.settings, name, "")
    assert ice.ice_servers() == []


def test_turn_credentials_are_ephemeral_per_session_when_a_secret_is_set(monkeypatch):
    monkeypatch.setattr(ice.settings, "REMOTE_STUN_URLS", "stun:a.example:3478, stun:b.example:3478")
    monkeypatch.setattr(ice.settings, "REMOTE_TURN_URLS", "turn:t.example:3478?transport=udp")
    monkeypatch.setattr(ice.settings, "REMOTE_TURN_SECRET", "secret-coturn")
    monkeypatch.setattr(ice.settings, "REMOTE_TURN_TTL", 600)
    stun, turn = ice.ice_servers("sess_abc", now=1000)
    assert stun == {"urls": ["stun:a.example:3478", "stun:b.example:3478"]}
    assert turn["username"] == "1600:sess_abc"
    expected = base64.b64encode(hmac.new(b"secret-coturn", b"1600:sess_abc", hashlib.sha1).digest()).decode()
    assert turn["credential"] == expected and "secret-coturn" not in json.dumps(turn)


def test_static_turn_credentials_are_used_without_a_secret(monkeypatch):
    monkeypatch.setattr(ice.settings, "REMOTE_STUN_URLS", "")
    monkeypatch.setattr(ice.settings, "REMOTE_TURN_URLS", "turn:t.example:3478")
    monkeypatch.setattr(ice.settings, "REMOTE_TURN_SECRET", "")
    monkeypatch.setattr(ice.settings, "REMOTE_TURN_USERNAME", "u")
    monkeypatch.setattr(ice.settings, "REMOTE_TURN_CREDENTIAL", "p")
    assert ice.ice_servers() == [{"urls": ["turn:t.example:3478"], "username": "u", "credential": "p"}]


# -- passerelle (hub) --------------------------------------------------------------------------------------------------------
class FakeLink:
    def __init__(self, device_id=DEVICE):
        self.sent, self.closed = [], False
        async def send(message):
            self.sent.append(message)
        async def close():
            self.closed = True
        self.connection = AgentConnection(device_id, send, close)


def test_hub_waits_for_the_person_at_the_device_to_accept_or_refuse():
    async def scenario():
        local = AgentHub(reply_timeout=0.2)
        link = FakeLink()
        await local.attach(link.connection)
        assert await local.is_online(DEVICE)
        from plugins.remote.models import Session
        session = Session(DEVICE, frozenset({Permission.VIEW_SCREEN}))
        task = asyncio.create_task(local.open_session(session))
        await asyncio.sleep(0.02)
        assert link.sent[0]["type"] == "open_session" and link.sent[0]["permissions"] == ["view_screen"]
        local.resolve_reply(session.session_id, True)
        await task
        await local.close_session(session.session_id)
        assert link.sent[-1]["type"] == "close_session"
        refused = Session(DEVICE, frozenset({Permission.VIEW_SCREEN}))
        task = asyncio.create_task(local.open_session(refused))
        await asyncio.sleep(0.02)
        local.resolve_reply(refused.session_id, False, "Pas maintenant")
        with pytest.raises(PermissionDenied, match="Pas maintenant"):
            await task
        silent = Session(DEVICE, frozenset({Permission.VIEW_SCREEN}))
        with pytest.raises(AgentUnavailable, match="Personne"):
            await local.open_session(silent)
        assert not local._pending
    run(scenario())


def test_hub_keeps_one_agent_per_device_and_ignores_stale_disconnections():
    async def scenario():
        local = AgentHub()
        old, new = FakeLink(), FakeLink()
        await local.attach(old.connection)
        await local.attach(new.connection)
        assert old.closed and local.connections[DEVICE] is new.connection
        assert local.detach(old.connection) is False and await local.is_online(DEVICE)
        assert local.detach(new.connection) is True and not await local.is_online(DEVICE)
        from plugins.remote.models import Session
        with pytest.raises(AgentUnavailable, match="hors ligne"):
            await local.open_session(Session(DEVICE, frozenset({Permission.VIEW_SCREEN})))
    run(scenario())


def test_json_device_store_persists_public_data_only(tmp_path):
    path = tmp_path / "devices.json"
    store = adapters.JsonDeviceStore(path)
    from plugins.remote.models import Device
    store.save(Device(DEVICE, "Bureau", "cle", paired=True, granted=frozenset({Permission.VIEW_SCREEN})))
    again = adapters.JsonDeviceStore(path).get(DEVICE)
    assert again and again.paired and again.granted == {Permission.VIEW_SCREEN} and again.public_key == "cle"
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
    with pytest.raises(Exception):
        with anonymous.websocket_connect(f"/api/remote/agent/ws?device_id={DEVICE}"):
            pass


def test_http_flow_from_registration_to_disconnection(client):
    agent = Agent()
    registered = client.post("/api/remote/devices/register", json=agent.register_args()).json()
    code = registered["pairing_code"]
    assert registered["expires_in"] == 1800 and "public_key" not in registered["device"]
    assert client.post("/api/remote/pair", json={"code": code}).json()["device"]["paired"] is True
    assert client.get("/api/remote/devices").json()["devices"][0]["online"] is True
    assert client.get(f"/api/remote/devices/{DEVICE}").json()["name"] == "Bureau"
    session = client.post("/api/remote/sessions", json={"device_id": DEVICE, "permissions": ["view_screen"]}).json()
    sid = session["session_id"]
    assert session["state"] == "CONNECTING" and client.get("/api/remote/status").json()["agent_installed"] is True
    assert client.post("/api/remote/signaling/offer", json={"session_id": sid, "sdp": "v=0 offer"}).status_code == 200
    assert client.post("/api/remote/signaling/ice", json={"session_id": sid, "candidate": "candidate:1"}).json()["ice_candidates"] == 1
    forged = {"session_id": sid, "sdp": "v=0 answer", "signature": Agent().sign(b"x")}
    assert client.post("/api/remote/signaling/answer", json=forged).status_code == 403
    real = service.session(sid)
    signed = {"session_id": sid, "sdp": "v=0 answer", "signature": agent.answer(real)}
    assert client.post("/api/remote/signaling/answer", json=signed).json()["state"] == "CONNECTED"
    assert client.get(f"/api/remote/sessions/{sid}/signaling").json()["answer"] == "v=0 answer"
    assert client.post(f"/api/remote/sessions/{sid}/reconnect").json()["state"] == "RECONNECTING"
    assert client.delete(f"/api/remote/sessions/{sid}").json()["state"] == "DISCONNECTED"
    assert client.get("/api/remote/sessions/sess_inconnue").status_code == 404
    assert client.get("/api/remote/ice-servers").json() == {"ice_servers": ice.ice_servers()}


def test_http_input_is_validated_and_pairing_attempts_are_limited(client):
    assert client.post("/api/remote/devices/register", json={"device_id": "x", "name": "n", "public_key": "court"}).status_code == 422
    assert client.post("/api/remote/devices/register", json={**Agent().register_args(), "name": "a|b"}).status_code == 422
    assert client.post("/api/remote/pair", json={"code": "12"}).status_code == 422
    assert client.post("/api/remote/sessions", json={"device_id": DEVICE, "permissions": ["terminal"]}).status_code in (403, 404)
    codes = [client.post("/api/remote/pair", json={"code": "000000"}).status_code for _ in range(5)]
    assert codes[:2] == [400, 400] and codes[2:] == [429, 429, 429]       # 3 requêtes invalides + 2 essais = 5 par minute ; au-delà : refus


# -- WebSocket de l'agent ----------------------------------------------------------------------------------------------------
@pytest.fixture
def live(remote, monkeypatch):
    """Vrai hub, sans agent simulé : un agent de test se connecte en WebSocket comme le ferait remote-agent."""
    monkeypatch.setattr(security.settings, "API_AUTH_TOKEN", "secret")
    hub.connections.clear()
    app = FastAPI()
    app.include_router(router, prefix="/api/remote")
    with ports.override(PORT_AGENT, hub), TestClient(app, headers={"x-api-key": "secret"}) as http:
        yield http
    hub.connections.clear()


def agent_login(live, agent):
    socket = live.websocket_connect(f"/api/remote/agent/ws?device_id={agent.device_id}")
    ws = socket.__enter__()
    challenge = ws.receive_json()
    assert challenge["type"] == "challenge"
    ws.send_json({"type": "auth", "signature": agent.sign(identity.agent_auth_message(agent.device_id, challenge["nonce"]))})
    assert ws.receive_json()["type"] == "ready"
    return socket, ws


def test_agent_websocket_rejects_unknown_unpaired_and_unproven_devices(live, remote):
    agent = Agent()
    for expected in (4403,):                                   # inconnu
        with live.websocket_connect(f"/api/remote/agent/ws?device_id={agent.device_id}") as ws:
            with pytest.raises(Exception) as caught:
                ws.receive_json()
        assert str(expected) in repr(caught.value) or caught.value.__class__.__name__ == "WebSocketDisconnect"
    remote.service.register_device(**agent.register_args())     # enregistré mais pas appairé
    with live.websocket_connect(f"/api/remote/agent/ws?device_id={agent.device_id}") as ws:
        with pytest.raises(Exception):
            ws.receive_json()
    remote.service.pair(next(iter(remote.service.pairing._codes)))
    with live.websocket_connect(f"/api/remote/agent/ws?device_id={agent.device_id}") as ws:
        assert ws.receive_json()["type"] == "challenge"
        ws.send_json({"type": "auth", "signature": Agent().sign(b"x")})        # signé par une autre clé
        with pytest.raises(Exception):
            ws.receive_json()
    assert not hub.connections and "AUTHENTICATION_FAILED" in remote.audit.names()


def test_agent_websocket_end_to_end_session_with_consent_signed_answer_and_reconnection(live, remote):
    agent = paired(remote)
    socket, ws = agent_login(live, agent)
    try:
        assert live.get("/api/remote/devices").json()["devices"][0]["online"] is True and live.get("/api/remote/status").json()["agents_online"] == 1
        result: dict = {}
        opener = threading.Thread(target=lambda: result.update(live.post("/api/remote/sessions", json={"device_id": DEVICE}).json()))
        opener.start()
        request = ws.receive_json()
        assert request["type"] == "open_session" and request["permissions"] == ["view_screen"]
        sid = request["session_id"]
        ws.send_json({"type": "session_reply", "session_id": sid, "accepted": True})
        opener.join(5)
        assert result["state"] == "CONNECTING" and result["session_id"] == sid
        live.post("/api/remote/signaling/offer", json={"session_id": sid, "sdp": "v=0 offer"})
        assert ws.receive_json() == {"type": "offer", "session_id": sid, "sdp": "v=0 offer"}
        session = service.session(sid)
        ws.send_json({"type": "grant", "permissions": ["view_screen", "control_mouse"]})
        ws.send_json({"type": "ice", "session_id": sid, "candidate": "candidate:agent"})
        ws.send_json({"type": "answer", "session_id": sid, "sdp": "v=0 answer", "signature": Agent().sign(b"intrus")})
        assert ws.receive_json()["type"] == "error"                                  # réponse mal signée : refusée
        ws.send_json({"type": "answer", "session_id": sid, "sdp": "v=0 answer", "signature": agent.answer(session)})
        for _ in range(50):
            view = live.get(f"/api/remote/sessions/{sid}/signaling").json()
            if view["state"] == "CONNECTED":
                break
            time.sleep(0.05)
        assert view["state"] == "CONNECTED" and view["answer"] == "v=0 answer" and view["ice"] == ["candidate:agent"]
        assert remote.store.get(DEVICE).granted == {Permission.VIEW_SCREEN, Permission.CONTROL_MOUSE}
        ws.send_json({"type": "bye", "session_id": sid})
        for _ in range(50):
            if service.session(sid).state is SessionState.DISCONNECTED:
                break
            time.sleep(0.05)
        assert service.session(sid).state is SessionState.DISCONNECTED
    finally:
        socket.__exit__(None, None, None)
    for _ in range(50):
        if not hub.connections:
            break
        time.sleep(0.05)
    assert not hub.connections


def test_dropping_the_agent_connection_puts_an_established_session_in_reconnecting(live, remote):
    agent = paired(remote)
    socket, ws = agent_login(live, agent)
    result: dict = {}
    opener = threading.Thread(target=lambda: result.update(live.post("/api/remote/sessions", json={"device_id": DEVICE}).json()))
    opener.start()
    sid = ws.receive_json()["session_id"]
    ws.send_json({"type": "session_reply", "session_id": sid, "accepted": True})
    opener.join(5)
    live.post("/api/remote/signaling/offer", json={"session_id": sid, "sdp": "v=0 offer"})
    ws.receive_json()
    ws.send_json({"type": "answer", "session_id": sid, "sdp": "v=0 answer", "signature": agent.answer(service.session(sid))})
    for _ in range(50):
        if service.session(sid).state is SessionState.CONNECTED:
            break
        time.sleep(0.05)
    socket.__exit__(None, None, None)
    for _ in range(50):
        if service.session(sid).state is SessionState.RECONNECTING:
            break
        time.sleep(0.05)
    assert service.session(sid).state is SessionState.RECONNECTING


def test_a_refusal_on_the_device_ends_the_session(live, remote):
    agent = paired(remote)
    socket, ws = agent_login(live, agent)
    try:
        result: dict = {}
        opener = threading.Thread(target=lambda: result.update(status=live.post("/api/remote/sessions", json={"device_id": DEVICE}).status_code))
        opener.start()
        sid = ws.receive_json()["session_id"]
        ws.send_json({"type": "session_reply", "session_id": sid, "accepted": False, "reason": "Non merci"})
        opener.join(5)
        assert result["status"] == 403 and service.session(sid).state is SessionState.DISCONNECTED
    finally:
        socket.__exit__(None, None, None)


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
