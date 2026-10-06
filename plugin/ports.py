"""Les trois interfaces dont dépend le plugin Remote (voir plugin_sdk/ports.py). Le service ne connaît que ces contrats :
l'agent Windows (Rust), le stockage et le journal sont remplaçables sans toucher à la logique des sessions."""
from __future__ import annotations

from typing import Protocol

from .models import Device, Session

PORT_AGENT = "remote.agent_gateway"
PORT_STORE = "remote.device_store"
PORT_AUDIT = "remote.audit_sink"


class AgentGateway(Protocol):
    """Lien avec le Remote Agent installé sur l'appareil contrôlé."""

    async def is_online(self, device_id: str) -> bool: ...

    async def open_session(self, session: Session) -> None:
        """L'agent accepte la session (lève AgentUnavailable s'il est injoignable, PermissionDenied s'il la refuse)."""

    async def close_session(self, session_id: str) -> None: ...


class DeviceStore(Protocol):
    def get(self, device_id: str) -> Device | None: ...

    def list(self) -> list[Device]: ...

    def save(self, device: Device) -> None: ...


class AuditSink(Protocol):
    """Journal des événements importants (docs/REMOTE_ARCHITECTURE.md, section 16). Ne doit jamais lever d'exception."""

    def record(self, event: str, **fields: str) -> None: ...
