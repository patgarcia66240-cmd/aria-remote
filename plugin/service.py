"""Logique du plugin Remote : appairage, permissions, sessions et signaling. Indépendante de FastAPI et du chat ; ne parle à l'agent, au stockage et
au journal que par les ports (ports.py), remplaçables via `plugin_sdk.ports`. Le service ne transporte JAMAIS la vidéo : il relaie seulement SDP et ICE."""
from __future__ import annotations

from collections.abc import Iterable

from plugin_sdk.ports import ports

from . import adapters, permissions, states
from .errors import AgentUnavailable, InvalidState, NotFound, PermissionDenied
from .models import MAX_ICE, MAX_ICE_SIZE, MAX_SDP, Device, Session
from .pairing import PairingBook
from .permissions import Permission
from .ports import PORT_AGENT, PORT_AUDIT, PORT_STORE, AgentGateway, AuditSink, DeviceStore
from .states import SessionState

_default_gateway = adapters.NoAgentGateway()
_default_audit = adapters.LogAuditSink()
_default_store: DeviceStore | None = None


class RemoteService:
    def __init__(self, pairing: PairingBook | None = None) -> None:
        self.pairing = pairing or PairingBook()
        self.sessions: dict[str, Session] = {}

    # -- ports (résolus à chaque appel : un fournisseur peut être branché ou remplacé en cours de route) ---------------------
    @property
    def gateway(self) -> AgentGateway:
        return ports.get(PORT_AGENT, _default_gateway)

    @property
    def store(self) -> DeviceStore:
        global _default_store
        found = ports.get(PORT_STORE)
        if found is not None:
            return found
        if _default_store is None:
            _default_store = adapters.JsonDeviceStore()
        return _default_store

    @property
    def audit(self) -> AuditSink:
        return ports.get(PORT_AUDIT, _default_audit)

    def agent_installed(self) -> bool:
        return ports.get(PORT_AGENT) is not None

    # -- appareils et appairage ----------------------------------------------------------------------------------------------
    def register_device(self, device_id: str, name: str, public_key: str, platform: str = "windows") -> tuple[Device, str]:
        """L'agent annonce son identité ; renvoie l'appareil et le code d'appairage à saisir côté contrôleur. Un appareil déjà appairé ne peut pas
        être « repris » par une autre clé publique (sinon un tiers pourrait usurper son identité)."""
        known = self.store.get(device_id)
        if known and known.paired and known.public_key != public_key:
            self.audit.record("AUTHENTICATION_FAILED", device_id=device_id, result="refused", reason="public_key_mismatch")
            raise InvalidState("Cet appareil est déjà appairé avec une autre identité.")
        device = known or Device(device_id, name, public_key, platform)
        device.name = name
        self.store.save(device)
        self.audit.record("PAIRING_REQUESTED", device_id=device_id)
        return device, self.pairing.issue(device_id)

    def pair(self, code: str) -> Device:
        try:
            device_id = self.pairing.redeem(code)
        except Exception:
            self.audit.record("AUTHENTICATION_FAILED", result="refused", reason="pairing_code")
            raise
        device = self.store.get(device_id)
        if device is None:
            raise NotFound("Appareil inconnu.")
        device.paired = True
        self.store.save(device)
        self.audit.record("PAIRING_ACCEPTED", device_id=device_id)
        return device

    async def list_devices(self) -> list[dict]:
        return [device.to_dict(online=await self.gateway.is_online(device.device_id)) for device in self.store.list()]

    def device(self, device_id: str) -> Device:
        found = self.store.get(device_id)
        if found is None:
            raise NotFound("Appareil inconnu.")
        return found

    def find_device(self, name: str) -> Device | None:
        wanted = name.strip().lower()
        return next((d for d in self.store.list() if d.name.lower() == wanted), None)

    # -- sessions ------------------------------------------------------------------------------------------------------------
    async def create_session(self, device_id: str, requested: Iterable[str] | None = None) -> Session:
        device = self.device(device_id)
        if not device.paired:
            raise PermissionDenied("Cet appareil n'est pas appairé.")
        wanted = permissions.parse(requested) if requested else frozenset(device.granted)
        permissions.ensure_granted(wanted, device.granted)
        if not wanted:
            raise PermissionDenied("Aucune permission demandée.")
        if any(s.device_id == device_id and s.state in states.ACTIVE for s in self.sessions.values()):
            raise InvalidState("Une session est déjà ouverte avec cet appareil.")
        if not await self.gateway.is_online(device_id):
            self.audit.record("SESSION_REFUSED", device_id=device_id, result="offline")
            raise AgentUnavailable("L'appareil est hors ligne ou aucun agent Remote n'est installé.")
        session = Session(device_id, wanted)
        self.sessions[session.session_id] = session
        self.audit.record("SESSION_CREATED", device_id=device_id, session_id=session.session_id)
        self._move(session, SessionState.WAITING)
        try:
            await self.gateway.open_session(session)
        except Exception as error:
            self._move(session, SessionState.DISCONNECTED)
            self.audit.record("PERMISSION_DENIED" if isinstance(error, PermissionDenied) else "SESSION_REFUSED", device_id=device_id,
                              session_id=session.session_id, result="refused")
            raise
        self._move(session, SessionState.AUTHENTICATING)
        self.audit.record("AUTHENTICATION_SUCCESS", device_id=device_id, session_id=session.session_id)
        self._move(session, SessionState.CONNECTING)
        return session

    def session(self, session_id: str) -> Session:
        found = self.sessions.get(session_id)
        if found is None:
            raise NotFound("Session inconnue.")
        return found

    def active_session(self) -> Session | None:
        return next((s for s in reversed(list(self.sessions.values())) if s.state in states.ACTIVE), None)

    async def close_session(self, session_id: str) -> Session:
        session = self.session(session_id)
        if session.state is SessionState.DISCONNECTED:
            return session
        self._move(session, SessionState.DISCONNECTED)
        try:
            await self.gateway.close_session(session_id)
        except Exception:        # noqa: BLE001 — la session est coupée côté serveur quoi qu'il arrive ; l'agent s'arrêtera à l'expiration du lien
            self.audit.record("REMOTE_DISCONNECTED", device_id=session.device_id, session_id=session_id, result="agent_unreachable")
        else:
            self.audit.record("REMOTE_DISCONNECTED", device_id=session.device_id, session_id=session_id)
        return session

    def _move(self, session: Session, target: SessionState) -> None:
        session.state = states.ensure_transition(session.state, target)

    # -- signaling : on relaie des textes, jamais de flux --------------------------------------------------------------------
    def submit_offer(self, session_id: str, sdp: str) -> Session:
        session = self._signaling_session(session_id, {SessionState.CONNECTING})
        session.offer = self._sdp(sdp)
        return session

    def submit_answer(self, session_id: str, sdp: str) -> Session:
        session = self._signaling_session(session_id, {SessionState.CONNECTING})
        if not session.offer:
            raise InvalidState("Pas d'offre à laquelle répondre.")
        session.answer = self._sdp(sdp)
        self._move(session, SessionState.CONNECTED)
        self.audit.record("REMOTE_CONNECTED", device_id=session.device_id, session_id=session_id)
        if Permission.CONTROL_MOUSE in session.permissions:
            self.audit.record("MOUSE_CONTROL_ENABLED", device_id=session.device_id, session_id=session_id)
        if Permission.CONTROL_KEYBOARD in session.permissions:
            self.audit.record("KEYBOARD_CONTROL_ENABLED", device_id=session.device_id, session_id=session_id)
        return session

    def add_ice(self, session_id: str, candidate: str) -> Session:
        session = self._signaling_session(session_id, {SessionState.CONNECTING, SessionState.CONNECTED, SessionState.RECONNECTING})
        if len(candidate) > MAX_ICE_SIZE or len(session.ice) >= MAX_ICE:
            raise InvalidState("Candidat ICE refusé (trop gros ou trop nombreux).")
        session.ice.append(candidate)
        return session

    def _signaling_session(self, session_id: str, allowed: set[SessionState]) -> Session:
        session = self.session(session_id)
        if session.state not in allowed:
            raise InvalidState(f"Session en état {session.state.value} : signaling impossible.")
        return session

    @staticmethod
    def _sdp(sdp: str) -> str:
        if not sdp.strip() or len(sdp) > MAX_SDP:
            raise InvalidState("SDP vide ou trop grand.")
        return sdp


service = RemoteService()
