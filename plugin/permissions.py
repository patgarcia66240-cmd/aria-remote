"""Permissions séparées (docs/REMOTE_ARCHITECTURE.md, section 7). Le catalogue est complet dès maintenant pour que les futures
fonctions s'ajoutent sans toucher au modèle ; seules celles de la V1 peuvent être accordées aujourd'hui."""
from __future__ import annotations

from collections.abc import Iterable
from enum import Enum

from .errors import PermissionDenied, RemoteError


class Permission(str, Enum):
    VIEW_SCREEN = "view_screen"
    CONTROL_MOUSE = "control_mouse"
    CONTROL_KEYBOARD = "control_keyboard"
    CLIPBOARD = "clipboard"
    FILE_READ = "file_read"
    FILE_WRITE = "file_write"
    REBOOT = "reboot"
    SHUTDOWN = "shutdown"
    TERMINAL = "terminal"


V1: frozenset[Permission] = frozenset({Permission.VIEW_SCREEN, Permission.CONTROL_MOUSE, Permission.CONTROL_KEYBOARD})
SUPPORTED: frozenset[Permission] = V1     # à élargir version après version, jamais avant que l'agent sache les refuser
DEFAULT_GRANT: frozenset[Permission] = frozenset({Permission.VIEW_SCREEN})   # par défaut : regarder seulement, contrôler se demande


def parse(values: Iterable[str]) -> frozenset[Permission]:
    """Texte -> permissions ; une permission inconnue ou pas encore prise en charge est refusée (jamais ignorée en silence)."""
    found: set[Permission] = set()
    for value in values:
        try:
            permission = Permission(str(value))
        except ValueError as error:
            raise RemoteError(f"Permission inconnue : {value}.") from error
        if permission not in SUPPORTED:
            raise PermissionDenied(f"Permission pas encore disponible : {permission.value}.")
        found.add(permission)
    return frozenset(found)


def ensure_granted(requested: frozenset[Permission], granted: frozenset[Permission]) -> None:
    """Une session ne reçoit jamais plus que ce que l'appareil a accordé."""
    missing = sorted(p.value for p in requested - granted)
    if missing:
        raise PermissionDenied("Permission non accordée par l'appareil : " + ", ".join(missing) + ".")
