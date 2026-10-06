"""Serveurs STUN/TURN donnés aux deux extrémités (docs/REMOTE_ARCHITECTURE.md, section 9). Le TURN relaie le trafic chiffré quand la connexion
directe est impossible ; il ne remplace pas le signaling. Avec REMOTE_TURN_SECRET (coturn `use-auth-secret`), chaque session reçoit des
identifiants éphémères : un identifiant qui fuit expire seul, et aucun mot de passe durable ne circule."""
from __future__ import annotations

import base64
import hashlib
import hmac
import time

from config import settings


def _split(value: str) -> list[str]:
    return [item.strip() for item in value.split(",") if item.strip()]


def ice_servers(session_id: str = "remote", now: float | None = None) -> list[dict]:
    servers: list[dict] = []
    stun = _split(settings.REMOTE_STUN_URLS)
    if stun:
        servers.append({"urls": stun})
    turn = _split(settings.REMOTE_TURN_URLS)
    if turn:
        if settings.REMOTE_TURN_SECRET:
            expires = int((time.time() if now is None else now)) + max(60, settings.REMOTE_TURN_TTL)
            username = f"{expires}:{session_id}"
            digest = hmac.new(settings.REMOTE_TURN_SECRET.encode("utf-8"), username.encode("utf-8"), hashlib.sha1).digest()
            servers.append({"urls": turn, "username": username, "credential": base64.b64encode(digest).decode("ascii")})
        elif settings.REMOTE_TURN_USERNAME and settings.REMOTE_TURN_CREDENTIAL:
            servers.append({"urls": turn, "username": settings.REMOTE_TURN_USERNAME, "credential": settings.REMOTE_TURN_CREDENTIAL})
    return servers
