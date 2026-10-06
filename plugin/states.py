"""Machine d'état d'une session (docs/REMOTE_ARCHITECTURE.md, section 10) : une seule table décide quelles transitions existent,
plutôt que des états implicites dispersés dans le code."""
from __future__ import annotations

from enum import Enum

from .errors import InvalidState


class SessionState(str, Enum):
    CREATED = "CREATED"
    WAITING = "WAITING"
    AUTHENTICATING = "AUTHENTICATING"
    CONNECTING = "CONNECTING"
    CONNECTED = "CONNECTED"
    RECONNECTING = "RECONNECTING"
    DISCONNECTED = "DISCONNECTED"


S = SessionState
TRANSITIONS: dict[SessionState, frozenset[SessionState]] = {
    S.CREATED: frozenset({S.WAITING, S.DISCONNECTED}),
    S.WAITING: frozenset({S.AUTHENTICATING, S.DISCONNECTED}),
    S.AUTHENTICATING: frozenset({S.CONNECTING, S.DISCONNECTED}),
    S.CONNECTING: frozenset({S.CONNECTED, S.DISCONNECTED}),
    S.CONNECTED: frozenset({S.RECONNECTING, S.DISCONNECTED}),
    S.RECONNECTING: frozenset({S.CONNECTED, S.DISCONNECTED}),
    S.DISCONNECTED: frozenset(),
}
ACTIVE = frozenset(state for state in S if state is not S.DISCONNECTED)


def can_transition(current: SessionState, target: SessionState) -> bool:
    return target in TRANSITIONS[current]


def ensure_transition(current: SessionState, target: SessionState) -> SessionState:
    if not can_transition(current, target):
        raise InvalidState(f"Transition impossible : {current.value} → {target.value}.")
    return target
