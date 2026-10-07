#!/usr/bin/env python3
"""Vérifie un serveur de rendez-vous déployé (Render, VPS…) : HTTPS, clés, canal WebSocket des agents, serveurs STUN/TURN.

    python check_server.py https://pc-assistant-rendezvous.onrender.com
    (les clés sont lues dans RENDEZVOUS_AGENT_KEY et RENDEZVOUS_CONTROLLER_KEY, ou demandées ; elles ne sont jamais affichées)

Ne modifie rien sur le serveur : il ne fait que lire (et tenter, sans succès attendu, des accès qui doivent être REFUSÉS). Aucune dépendance : bibliothèque
standard seulement. Code de sortie 0 si tout est conforme, 1 sinon.
"""
from __future__ import annotations

import base64
import getpass
import json
import os
import socket
import ssl
import sys
import time
import urllib.error
import urllib.request
from urllib.parse import urlparse

WAKE_TIMEOUT = 120      # une offre gratuite qui dormait peut mettre une à deux minutes à se réveiller
results: list[tuple[bool, str]] = []


def check(ok: bool, text: str, detail: str = "") -> bool:
    results.append((ok, text))
    print(f"  {'OK ' if ok else 'ÉCHEC'}  {text}" + (f"  ({detail})" if detail else ""))
    return ok


def http(base: str, path: str, key: str | None = None, timeout: float = 30) -> tuple[int, str]:
    request = urllib.request.Request(base + path, headers={"x-api-key": key} if key else {})
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            return response.status, response.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as error:
        return error.code, error.read().decode("utf-8", "replace")


def websocket_status(base: str, key: str) -> int:
    """Poignée de main WebSocket de l'agent : 101 = canal accepté, 401/403 = clé refusée. Ne va pas plus loin (l'appareil de test n'existe pas)."""
    url = urlparse(base)
    secure = url.scheme == "https"
    host, port = url.hostname, url.port or (443 if secure else 80)
    raw = socket.create_connection((host, port), timeout=30)
    connection = ssl.create_default_context().wrap_socket(raw, server_hostname=host) if secure else raw
    try:
        nonce = base64.b64encode(os.urandom(16)).decode()
        connection.sendall((f"GET /api/remote/agent/ws?device_id=dev_verification0001 HTTP/1.1\r\nHost: {url.netloc}\r\nUpgrade: websocket\r\n"
                            f"Connection: Upgrade\r\nSec-WebSocket-Key: {nonce}\r\nSec-WebSocket-Version: 13\r\nx-api-key: {key}\r\n\r\n").encode())
        first_line = connection.recv(256).split(b"\r\n", 1)[0].decode("latin-1")
        return int(first_line.split()[1])
    finally:
        connection.close()


def main() -> int:
    if len(sys.argv) != 2 or not sys.argv[1].startswith(("http://", "https://")):
        print(__doc__)
        return 2
    base = sys.argv[1].rstrip("/")
    agent_key = os.environ.get("RENDEZVOUS_AGENT_KEY") or getpass.getpass("Clé des agents (RENDEZVOUS_AGENT_KEY) : ")
    controller_key = os.environ.get("RENDEZVOUS_CONTROLLER_KEY") or getpass.getpass("Clé du contrôleur (RENDEZVOUS_CONTROLLER_KEY) : ")
    print(f"\nServeur de rendez-vous : {base}\n")

    print("Disponibilité")
    started, health = time.monotonic(), None
    while time.monotonic() - started < WAKE_TIMEOUT:
        try:
            health = http(base, "/health", timeout=20)
            if health[0] == 200:
                break
        except (urllib.error.URLError, OSError, ssl.SSLError) as error:
            health = (0, str(error))
        time.sleep(5)
    ok = health is not None and health[0] == 200
    check(ok, "le serveur répond sur /health", f"{time.monotonic() - started:.0f} s" if ok else str(health))
    if not ok:
        print("\nLe serveur ne répond pas : s'il vient de se réveiller (offre gratuite), réessaie dans une minute ; sinon regarde ses journaux.")
        return 1
    if urlparse(base).hostname in ("localhost", "127.0.0.1", "::1"):
        print("  info  adresse locale : HTTPS non vérifié")
    else:
        check(base.startswith("https://"), "connexion chiffrée (HTTPS)", "" if base.startswith("https://") else "http : à ne pas utiliser par Internet")
    check(http(base, "/docs")[0] == 404 and http(base, "/openapi.json")[0] == 404, "aucune documentation publique exposée")

    print("\nClés et rôles")
    check(http(base, "/api/remote/devices")[0] == 401, "sans clé : refusé (401)")
    check(http(base, "/api/remote/devices", "x" * 40)[0] == 403, "mauvaise clé : refusée (403)")
    check(http(base, "/api/remote/devices", agent_key)[0] == 403, "la clé des agents ne peut pas lister les appareils")
    status, body = http(base, "/api/remote/devices", controller_key)
    check(status == 200, "la clé du contrôleur peut lister les appareils", f"{len(json.loads(body)['devices'])} appareil(s)" if status == 200 else f"code {status}")

    print("\nCanal des agents (WebSocket)")
    try:
        check(websocket_status(base, agent_key) == 101, "la clé des agents ouvre le canal WebSocket (101)")
        refused = websocket_status(base, controller_key)
        check(refused in (401, 403), "la clé du contrôleur est refusée sur ce canal", f"code {refused}")
    except (OSError, ssl.SSLError, ValueError, IndexError) as error:
        check(False, "le canal WebSocket est joignable", str(error))

    print("\nConnexion directe et relais (STUN / TURN)")
    status, body = http(base, "/api/remote/ice-servers", controller_key)
    servers = json.loads(body).get("ice_servers", []) if status == 200 else []
    check(status == 200, "le serveur fournit les serveurs ICE", f"code {status}" if status != 200 else "")
    stun = any(any(u.startswith("stun:") for u in s["urls"]) for s in servers)
    turn = any("username" in s for s in servers)
    check(stun, "STUN configuré (traversée de la plupart des box domestiques)")
    print(f"  {'OK ' if turn else 'info'}  TURN {'configuré' if turn else 'non configuré : certains réseaux stricts ne pourront pas se connecter (normal sur Render)'}")

    failures = [text for ok, text in results if not ok]
    print(f"\n{'Tout est conforme.' if not failures else str(len(failures)) + ' point(s) à corriger.'}")
    return 0 if not failures else 1


if __name__ == "__main__":
    sys.exit(main())
