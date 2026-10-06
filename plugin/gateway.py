"""Passerelle vers les agents Remote connectés en WebSocket (route /api/remote/agent/ws). Implémente le port `AgentGateway` : le service ne sait pas
qu'il parle à des WebSocket. Le hub ne garde que des connexions et des réponses en attente ; aucune décision de sécurité n'est prise ici (le
service décide, l'agent a le dernier mot : il peut refuser une session)."""
from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Any

from . import ice
from .errors import AgentUnavailable, PermissionDenied
from .models import Session

REPLY_TIMEOUT = 60.0      # temps laissé à la personne devant l'appareil pour accepter ou refuser


@dataclass
class AgentConnection:
    device_id: str
    send: Callable[[dict[str, Any]], Awaitable[None]]
    close: Callable[[], Awaitable[None]]


class AgentHub:
    def __init__(self, reply_timeout: float = REPLY_TIMEOUT) -> None:
        self.reply_timeout = reply_timeout
        self.connections: dict[str, AgentConnection] = {}
        self._pending: dict[str, asyncio.Future[tuple[bool, str]]] = {}
        self._session_device: dict[str, str] = {}

    # -- connexions ----------------------------------------------------------------------------------------------------------
    async def attach(self, connection: AgentConnection) -> None:
        """Un seul agent par appareil : une nouvelle connexion remplace (et ferme) l'ancienne."""
        previous = self.connections.get(connection.device_id)
        self.connections[connection.device_id] = connection
        if previous is not None:
            try:
                await previous.close()
            except Exception:       # noqa: BLE001 — l'ancienne connexion est peut-être déjà morte
                pass

    def detach(self, connection: AgentConnection) -> bool:
        """Retire la connexion si c'est encore la courante ; False si elle avait déjà été remplacée (rien à signaler alors)."""
        if self.connections.get(connection.device_id) is connection:
            del self.connections[connection.device_id]
            return True
        return False

    def _connection(self, device_id: str) -> AgentConnection:
        found = self.connections.get(device_id)
        if found is None:
            raise AgentUnavailable("L'appareil est hors ligne.")
        return found

    # -- port AgentGateway ---------------------------------------------------------------------------------------------------
    async def is_online(self, device_id: str) -> bool:
        return device_id in self.connections

    async def open_session(self, session: Session) -> None:
        connection = self._connection(session.device_id)
        reply: asyncio.Future[tuple[bool, str]] = asyncio.get_running_loop().create_future()
        self._pending[session.session_id] = reply
        self._session_device[session.session_id] = session.device_id
        try:
            await connection.send({"type": "open_session", "session_id": session.session_id, "permissions": sorted(p.value for p in session.permissions),
                                   "ice_servers": ice.ice_servers(session.session_id)})
            accepted, reason = await asyncio.wait_for(reply, self.reply_timeout)
        except asyncio.TimeoutError:
            self._session_device.pop(session.session_id, None)
            raise AgentUnavailable("Personne n'a répondu sur l'appareil : connexion abandonnée.") from None
        except Exception:
            self._session_device.pop(session.session_id, None)
            raise
        finally:
            self._pending.pop(session.session_id, None)
        if not accepted:
            self._session_device.pop(session.session_id, None)
            raise PermissionDenied(reason or "L'utilisateur de l'appareil a refusé la connexion.")

    def resolve_reply(self, session_id: str, accepted: bool, reason: str = "") -> None:
        reply = self._pending.get(session_id)
        if reply is not None and not reply.done():
            reply.set_result((accepted, reason))

    async def close_session(self, session_id: str) -> None:
        device_id = self._session_device.pop(session_id, None)
        connection = self.connections.get(device_id) if device_id else None
        if connection is not None:
            await connection.send({"type": "close_session", "session_id": session_id})

    async def send_signal(self, session: Session, kind: str, value: str) -> None:
        message: dict[str, Any] = {"type": kind, "session_id": session.session_id}
        if kind == "offer":
            message["sdp"] = value
        elif kind == "ice":
            message["candidate"] = value
        await self._connection(session.device_id).send(message)

    def forget_session(self, session_id: str) -> None:
        """La session s'est terminée par l'agent lui-même : plus rien à lui dire."""
        self._session_device.pop(session_id, None)


hub = AgentHub()
