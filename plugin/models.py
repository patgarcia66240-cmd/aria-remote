"""Données du plugin Remote (docs/REMOTE_ARCHITECTURE.md, sections 6, 7 et 10) : appareil appairé, session, canal de signaling."""
from __future__ import annotations

import hashlib
import re
import secrets
import time
from dataclasses import dataclass, field
from typing import Any

from .permissions import DEFAULT_GRANT, Permission
from .states import SessionState

ID_PATTERN = r"^[A-Za-z0-9_-]{8,64}$"
ID_RE = re.compile(ID_PATTERN)
MAX_SDP = 65_536          # un SDP fait quelques ko : au-delà, ce n'est pas du signaling
MAX_ICE = 200             # candidats ICE conservés par session
MAX_ICE_SIZE = 2_048


@dataclass
class Device:
    device_id: str
    name: str
    public_key: str                    # clé PUBLIQUE seulement : la clé privée ne quitte jamais l'appareil
    platform: str = "windows"
    paired: bool = False
    granted: frozenset[Permission] = DEFAULT_GRANT
    registered_at: float = field(default_factory=time.time)

    def fingerprint(self) -> str:
        return hashlib.sha256(self.public_key.encode("utf-8")).hexdigest()[:16]

    def to_dict(self, *, online: bool = False) -> dict[str, Any]:
        return {"device_id": self.device_id, "name": self.name, "platform": self.platform, "paired": self.paired, "online": online,
                "fingerprint": self.fingerprint(), "granted": sorted(p.value for p in self.granted)}


@dataclass
class Session:
    device_id: str
    permissions: frozenset[Permission]
    session_id: str = field(default_factory=lambda: "sess_" + secrets.token_urlsafe(12))
    state: SessionState = SessionState.CREATED
    created_at: float = field(default_factory=time.time)
    state_changed_at: float = field(default_factory=time.monotonic)
    offer: str = ""
    answer: str = ""
    ice: list[str] = field(default_factory=list)             # candidats ICE du contrôleur
    agent_ice: list[str] = field(default_factory=list)       # candidats ICE de l'agent

    def to_dict(self) -> dict[str, Any]:
        return {"session_id": self.session_id, "device_id": self.device_id, "state": self.state.value, "created_at": self.created_at,
                "permissions": sorted(p.value for p in self.permissions), "has_offer": bool(self.offer), "has_answer": bool(self.answer),
                "ice_candidates": len(self.ice), "agent_ice_candidates": len(self.agent_ice)}
