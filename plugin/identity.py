"""Identité cryptographique des appareils (docs/REMOTE_ARCHITECTURE.md, sections 6 et 21.8) : clé Ed25519 générée sur l'appareil, dont seule la
partie PUBLIQUE arrive ici. L'appareil prouve qu'il détient la clé privée en signant des messages ; le code d'appairage, lui, ne prouve rien.

Les messages signés sont des champs joints par « | », préfixés par une version et un type (séparation des usages : une signature d'enregistrement
ne vaut jamais réponse de signaling). L'agent Rust (agent/src/identity.rs) construit exactement les mêmes octets."""
from __future__ import annotations

import base64
import binascii
import hashlib
import re
import secrets
import time
from collections.abc import Callable

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

from .errors import PermissionDenied, RemoteError

PREFIX = "pc-assistant-remote-v1"
NONCE_PATTERN = r"^[A-Za-z0-9_-]{16,64}$"
NONCE_RE = re.compile(NONCE_PATTERN)
CLOCK_SKEW = 120.0            # un enregistrement signé n'est valable que quelques minutes autour de l'heure du serveur


def sha256_hex(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def new_nonce() -> str:
    return secrets.token_urlsafe(24)


def register_message(device_id: str, name: str, timestamp: int, nonce: str) -> bytes:
    return "|".join((PREFIX, "register", device_id, name, str(timestamp), nonce)).encode("utf-8")


def answer_message(session_id: str, offer: str, sdp: str) -> bytes:
    """Réponse de signaling : liée à CETTE offre (pas rejouable sur une autre) et à son contenu, empreinte DTLS comprise — un intermédiaire ne peut
    donc pas substituer sa propre extrémité média."""
    return "|".join((PREFIX, "answer", session_id, sha256_hex(offer), sha256_hex(sdp))).encode("utf-8")


def agent_auth_message(device_id: str, nonce: str) -> bytes:
    return "|".join((PREFIX, "agent", device_id, nonce)).encode("utf-8")


def parse_public_key(public_key: str) -> Ed25519PublicKey:
    """Clé publique Ed25519 : 32 octets bruts en base64 standard."""
    try:
        raw = base64.b64decode(public_key, validate=True)
        if len(raw) != 32:
            raise ValueError("longueur")
        return Ed25519PublicKey.from_public_bytes(raw)
    except (binascii.Error, ValueError) as error:
        raise RemoteError("Clé publique invalide : Ed25519, 32 octets en base64 attendus.") from error


def verify(public_key: str, message: bytes, signature: str) -> None:
    """Lève PermissionDenied si la signature (base64) n'a pas été produite avec la clé privée correspondante."""
    key = parse_public_key(public_key)
    try:
        key.verify(base64.b64decode(signature, validate=True), message)
    except (InvalidSignature, binascii.Error, ValueError) as error:
        raise PermissionDenied("Signature de l'appareil invalide.") from error


class ReplayGuard:
    """Retient les nonces vus pendant la fenêtre de validité : un enregistrement signé capturé ne peut pas être rejoué."""

    def __init__(self, clock: Callable[[], float] = time.time, window: float = CLOCK_SKEW) -> None:
        self._clock, self.window = clock, window
        self._seen: dict[str, float] = {}

    def check(self, timestamp: int, nonce: str) -> None:
        now = self._clock()
        if abs(now - timestamp) > self.window:
            raise PermissionDenied("Horodatage hors fenêtre : vérifie l'heure de l'appareil.")
        self._seen = {n: exp for n, exp in self._seen.items() if exp > now}
        if nonce in self._seen:
            raise PermissionDenied("Requête déjà utilisée.")
        self._seen[nonce] = now + 2 * self.window
