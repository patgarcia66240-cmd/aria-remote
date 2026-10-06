"""Logique du plugin Remote : appairage, permissions, sessions et signaling. Indépendante de FastAPI et du chat ; ne parle à l'agent, au stockage et
au journal que par les ports (ports.py), remplaçables via `plugin_sdk.ports`. Le service ne transporte JAMAIS la vidéo : il relaie seulement SDP et ICE."""
from __future__ import annotations

import time
from collections.abc import Callable, Iterable

from plugin_sdk.ports import ports

from . import adapters, identity, permissions, states
from .errors import AgentUnavailable, InvalidState, NotFound, PermissionDenied
from .gateway import hub
from .models import MAX_ICE, MAX_ICE_SIZE, MAX_SDP, Device, Session
from .pairing import PairingBook
from .permissions import DEFAULT_GRANT, Permission
from .ports import PORT_AGENT, PORT_AUDIT, PORT_STORE, AgentGateway, AuditSink, DeviceStore
from .states import SessionState

RECONNECT_GRACE = 60.0       # une session qui n'a pas retrouvé son lien au bout d'une minute est terminée

_default_gateway = hub       # agents connectés en WebSocket ; un autre fournisseur peut le remplacer via ports.provide(PORT_AGENT, …)
_default_audit = adapters.LogAuditSink()
_default_store: DeviceStore | None = None


class RemoteService:
    def __init__(self, pairing: PairingBook | None = None, clock: Callable[[], float] = time.monotonic) -> None:
        self.pairing = pairing or PairingBook()
        self.sessions: dict[str, Session] = {}
        self.replay = identity.ReplayGuard()
        self.clock = clock
        self.reconnect_grace = RECONNECT_GRACE

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
        """Un agent a-t-il déjà été installé ? Vrai dès qu'un fournisseur d'agent est branché ou qu'un appareil s'est enregistré."""
        return ports.get(PORT_AGENT) is not None or bool(self.store.list())

    # -- appareils et appairage ----------------------------------------------------------------------------------------------
    def register_device(self, device_id: str, name: str, public_key: str, platform: str, timestamp: int, nonce: str,
                        signature: str) -> tuple[Device, str]:
        """L'agent annonce son identité en SIGNANT sa demande avec sa clé privée : seul celui qui la détient peut enregistrer cette clé publique
        (preuve de possession) et la demande n'est pas rejouable (horodatage + nonce). Renvoie l'appareil et le code d'appairage à saisir côté
        contrôleur. Un appareil déjà appairé ne peut pas être « repris » par une autre clé publique."""
        identity.parse_public_key(public_key)
        try:
            identity.verify(public_key, identity.register_message(device_id, name, timestamp, nonce), signature)
            self.replay.check(timestamp, nonce)
        except PermissionDenied:
            self.audit.record("AUTHENTICATION_FAILED", device_id=device_id, result="refused", reason="register_signature")
            raise
        known = self.store.get(device_id)
        if known and known.paired and known.public_key != public_key:
            self.audit.record("AUTHENTICATION_FAILED", device_id=device_id, result="refused", reason="public_key_mismatch")
            raise InvalidState("Cet appareil est déjà appairé avec une autre identité.")
        device = known or Device(device_id, name, public_key, platform)
        device.name = name
        device.public_key = public_key       # appareil pas encore appairé : il peut changer de clé, pas un appareil appairé (refusé plus haut)
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

    def set_granted(self, device_id: str, values: Iterable[str]) -> Device:
        """Ce que l'appareil accepte de laisser faire. Décidé PAR l'appareil (son agent, authentifié par sa clé), jamais par le contrôleur : c'est le
        plafond de toute session. Seules les permissions de la V1 sont acceptées (permissions.parse)."""
        device = self.device(device_id)
        device.granted = permissions.parse(values) or DEFAULT_GRANT
        self.store.save(device)
        self.audit.record("PERMISSION_GRANTED", device_id=device_id, permissions=",".join(sorted(p.value for p in device.granted)))
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
        await self.sweep()
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
        session = Session(device_id, wanted, state_changed_at=self.clock())
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
        self._expire(found)
        return found

    def active_session(self) -> Session | None:
        for candidate in self.sessions.values():
            self._expire(candidate)
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
        session.state_changed_at = self.clock()

    # -- reconnexion : le lien WebRTC est tombé, la session survit un temps limité -------------------------------------------
    def _expire(self, session: Session) -> bool:
        """Termine une session restée trop longtemps en RECONNECTING (état seulement ; `sweep` prévient aussi l'agent)."""
        if session.state is SessionState.RECONNECTING and self.clock() - session.state_changed_at > self.reconnect_grace:
            self._move(session, SessionState.DISCONNECTED)
            self.audit.record("REMOTE_DISCONNECTED", device_id=session.device_id, session_id=session.session_id, result="reconnect_timeout")
            return True
        return False

    async def sweep(self) -> None:
        for session in list(self.sessions.values()):
            if self._expire(session):
                try:
                    await self.gateway.close_session(session.session_id)
                except Exception:        # noqa: BLE001 — l'agent est injoignable : il n'y a rien d'autre à faire
                    pass

    def _start_reconnecting(self, session: Session) -> None:
        self._move(session, SessionState.RECONNECTING)
        session.offer = session.answer = ""
        session.ice.clear()
        session.agent_ice.clear()
        self.audit.record("REMOTE_RECONNECTING", device_id=session.device_id, session_id=session.session_id)

    async def reconnect(self, session_id: str) -> Session:
        """Le contrôleur signale que le lien est tombé : nouvelle négociation (offre, réponse signée) dans le délai de grâce."""
        session = self.session(session_id)
        if session.state is SessionState.RECONNECTING:
            return session
        if session.state is not SessionState.CONNECTED:
            raise InvalidState(f"Session en état {session.state.value} : reconnexion impossible.")
        self._start_reconnecting(session)
        try:
            await self.gateway.send_signal(session, "reconnect", "")
        except Exception:        # noqa: BLE001 — agent injoignable : la session expirera seule au bout du délai de grâce
            pass
        return session

    def agent_lost(self, device_id: str) -> None:
        """La connexion de l'agent est tombée : une session établie passe en reconnexion, une session en cours de négociation est abandonnée."""
        for session in self.sessions.values():
            if session.device_id != device_id or session.state not in states.ACTIVE:
                continue
            if session.state is SessionState.CONNECTED:
                self._start_reconnecting(session)
            elif session.state is not SessionState.RECONNECTING:
                self._move(session, SessionState.DISCONNECTED)
                self.audit.record("REMOTE_DISCONNECTED", device_id=device_id, session_id=session.session_id, result="agent_lost")

    def agent_ended(self, session_id: str, device_id: str) -> None:
        """L'utilisateur de l'appareil a coupé la session (droit prévu par la section 17 du doc) : on la termine, sans rappeler l'agent."""
        session = self.session(session_id)
        if session.device_id != device_id:
            raise PermissionDenied("Cette session n'appartient pas à cet appareil.")
        if session.state is not SessionState.DISCONNECTED:
            self._move(session, SessionState.DISCONNECTED)
            self.audit.record("REMOTE_DISCONNECTED", device_id=device_id, session_id=session_id, result="ended_by_device")
        hub.forget_session(session_id)

    # -- signaling : on relaie des textes, jamais de flux --------------------------------------------------------------------
    async def submit_offer(self, session_id: str, sdp: str) -> Session:
        session = self._signaling_session(session_id, {SessionState.CONNECTING, SessionState.RECONNECTING})
        offer = self._sdp(sdp)
        await self.gateway.send_signal(session, "offer", offer)
        session.offer, session.answer = offer, ""
        session.agent_ice.clear()
        return session

    def submit_answer(self, session_id: str, sdp: str, signature: str, device_id: str | None = None) -> Session:
        """Réponse de l'agent. Elle doit être SIGNÉE par la clé de l'appareil, sur cette offre précise : sans la clé privée, impossible de se faire
        passer pour l'agent, même avec la clé d'API (le SDP contient l'empreinte DTLS : la signer verrouille l'extrémité média)."""
        session = self._signaling_session(session_id, {SessionState.CONNECTING, SessionState.RECONNECTING})
        if device_id is not None and session.device_id != device_id:
            raise PermissionDenied("Cette session n'appartient pas à cet appareil.")
        if not session.offer:
            raise InvalidState("Pas d'offre à laquelle répondre.")
        answer = self._sdp(sdp)
        try:
            identity.verify(self.device(session.device_id).public_key, identity.answer_message(session.session_id, session.offer, answer), signature)
        except PermissionDenied:
            self.audit.record("AUTHENTICATION_FAILED", device_id=session.device_id, session_id=session_id, result="refused", reason="answer_signature")
            raise
        reconnected = session.state is SessionState.RECONNECTING
        session.answer = answer
        self._move(session, SessionState.CONNECTED)
        if reconnected:
            self.audit.record("REMOTE_RECONNECTED", device_id=session.device_id, session_id=session_id)
            return session
        self.audit.record("REMOTE_CONNECTED", device_id=session.device_id, session_id=session_id)
        if Permission.CONTROL_MOUSE in session.permissions:
            self.audit.record("MOUSE_CONTROL_ENABLED", device_id=session.device_id, session_id=session_id)
        if Permission.CONTROL_KEYBOARD in session.permissions:
            self.audit.record("KEYBOARD_CONTROL_ENABLED", device_id=session.device_id, session_id=session_id)
        return session

    async def add_ice(self, session_id: str, candidate: str) -> Session:
        """Candidat ICE du contrôleur, relayé à l'agent."""
        session = self._ice_session(session_id, candidate)
        await self.gateway.send_signal(session, "ice", candidate)
        session.ice.append(candidate)
        return session

    def add_agent_ice(self, session_id: str, candidate: str, device_id: str) -> Session:
        """Candidat ICE de l'agent, que le contrôleur vient lire."""
        session = self._ice_session(session_id, candidate)
        if session.device_id != device_id:
            raise PermissionDenied("Cette session n'appartient pas à cet appareil.")
        session.agent_ice.append(candidate)
        return session

    def signaling_view(self, session_id: str, after: int = 0) -> dict:
        """Ce que le contrôleur vient chercher : la réponse de l'agent et ses candidats ICE à partir du n-ième."""
        session = self.session(session_id)
        return {"state": session.state.value, "answer": session.answer, "ice": session.agent_ice[after:], "next": len(session.agent_ice)}

    def _ice_session(self, session_id: str, candidate: str) -> Session:
        session = self._signaling_session(session_id, {SessionState.CONNECTING, SessionState.CONNECTED, SessionState.RECONNECTING})
        if len(candidate) > MAX_ICE_SIZE or len(session.ice) >= MAX_ICE or len(session.agent_ice) >= MAX_ICE:
            raise InvalidState("Candidat ICE refusé (trop gros ou trop nombreux).")
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
