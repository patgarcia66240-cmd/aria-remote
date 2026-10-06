"""Implémentations par défaut des ports. Aucune ne contrôle un vrai PC : l'agent Windows viendra avec sa propre implémentation de
AgentGateway, fournie par `ports.provide(PORT_AGENT, …)`."""
from __future__ import annotations

import json
import os
import tempfile
from pathlib import Path

from services.audit_log import log_action

from .errors import AgentUnavailable, PermissionDenied
from .models import Device, Session
from .permissions import Permission

DEVICES_PATH = Path(__file__).resolve().parent.parent.parent / "data" / "remote_devices.json"


class NoAgentGateway:
    """Par défaut : aucun agent n'est installé. Dit clairement pourquoi aucune session ne peut s'ouvrir, au lieu de simuler une réussite."""

    async def is_online(self, device_id: str) -> bool:
        return False

    async def open_session(self, session: Session) -> None:
        raise AgentUnavailable("Aucun agent Remote n'est installé : le contrôle à distance n'est pas encore opérationnel.")

    async def close_session(self, session_id: str) -> None:
        return None

    async def send_signal(self, session: Session, kind: str, value: str) -> None:
        raise AgentUnavailable("Aucun agent Remote n'est installé.")


class LoopbackAgentGateway:
    """Agent simulé en mémoire (tests, développement de l'interface) : `online` = appareils joignables, `refuse` = appareils qui refusent."""

    def __init__(self, online: set[str] | None = None, refuse: set[str] | None = None) -> None:
        self.online = set(online or ())
        self.refuse = set(refuse or ())
        self.opened: list[str] = []
        self.closed: list[str] = []
        self.signals: list[tuple[str, str, str]] = []

    async def is_online(self, device_id: str) -> bool:
        return device_id in self.online

    async def open_session(self, session: Session) -> None:
        if session.device_id not in self.online:
            raise AgentUnavailable("L'appareil est hors ligne.")
        if session.device_id in self.refuse:
            raise PermissionDenied("L'utilisateur de l'appareil a refusé la connexion.")
        self.opened.append(session.session_id)

    async def close_session(self, session_id: str) -> None:
        self.closed.append(session_id)

    async def send_signal(self, session: Session, kind: str, value: str) -> None:
        self.signals.append((session.session_id, kind, value))


class MemoryDeviceStore:
    def __init__(self) -> None:
        self._devices: dict[str, Device] = {}

    def get(self, device_id: str) -> Device | None:
        return self._devices.get(device_id)

    def list(self) -> list[Device]:
        return sorted(self._devices.values(), key=lambda d: d.name.lower())

    def save(self, device: Device) -> None:
        self._devices[device.device_id] = device


class JsonDeviceStore(MemoryDeviceStore):
    """Appareils appairés dans data/remote_devices.json (clés publiques seulement : rien de secret). Écriture atomique."""

    def __init__(self, path: Path = DEVICES_PATH) -> None:
        super().__init__()
        self.path = path
        try:
            for raw in json.loads(path.read_text(encoding="utf-8")):
                self._devices[raw["device_id"]] = Device(raw["device_id"], raw["name"], raw["public_key"], raw.get("platform", "windows"), bool(raw.get("paired")),
                                                         frozenset(Permission(p) for p in raw.get("granted", [])), float(raw.get("registered_at", 0)))
        except (OSError, ValueError, KeyError):
            self._devices.clear()       # fichier absent ou illisible : on repart de zéro, le prochain appairage le recréera

    def save(self, device: Device) -> None:
        super().save(device)
        rows = [{"device_id": d.device_id, "name": d.name, "public_key": d.public_key, "platform": d.platform, "paired": d.paired,
                 "granted": sorted(p.value for p in d.granted), "registered_at": d.registered_at} for d in self._devices.values()]
        self.path.parent.mkdir(parents=True, exist_ok=True)
        handle, temp = tempfile.mkstemp(dir=self.path.parent, suffix=".tmp")
        with os.fdopen(handle, "w", encoding="utf-8") as out:
            json.dump(rows, out, ensure_ascii=False, indent=2)
        os.replace(temp, self.path)


class LogAuditSink:
    """Écrit dans le journal d'audit commun (services/audit_log.py, voir GET /api/audit/recent)."""

    def record(self, event: str, **fields: str) -> None:
        log_action(tool="remote", action=event, target=fields.get("device_id") or fields.get("session_id") or "-",
                   result=fields.get("result", "ok"), detail=", ".join(f"{k}={v}" for k, v in fields.items() if k not in ("device_id", "result")) or None)


class MemoryAuditSink:
    def __init__(self) -> None:
        self.events: list[tuple[str, dict[str, str]]] = []

    def record(self, event: str, **fields: str) -> None:
        self.events.append((event, fields))

    def names(self) -> list[str]:
        return [name for name, _ in self.events]
