"""Serveur de rendez-vous de PC Assistant Remote : le point de rencontre PUBLIC qui permet d'atteindre un PC par Internet avec son ID et son code.

Comme chez TeamViewer, chaque PC contrôlé garde une connexion SORTANTE vers ce serveur (aucun port à ouvrir chez lui) ; le contrôleur le retrouve
par son identifiant ; le serveur relaie seulement la négociation (SDP, ICE). L'écran et les commandes passent ensuite en direct (WebRTC, chiffré),
ou par le relais coturn quand le routeur bloque la connexion directe.

Ce service est volontairement MINIMAL et isolé : il réutilise tel quel le plugin « remote » de PC Assistant (même protocole, mêmes tests) mais
n'embarque aucun autre plugin — ni fichiers, ni système, ni agenda. Il est fait pour être exposé sur Internet, derrière HTTPS (voir Caddyfile).

Deux clés distinctes, parce qu'un PC contrôlé peut être compromis :
- RENDEZVOUS_AGENT_KEY      : agents seulement (s'enregistrer, ouvrir le canal WebSocket) ;
- RENDEZVOUS_CONTROLLER_KEY : contrôleur seulement (lister, appairer, ouvrir des sessions, signaling).
Une clé d'agent qui fuit ne permet donc ni de lister les appareils ni d'ouvrir une session. Chaque session exige de toute façon l'accord de la personne
devant l'appareil contrôlé, et chaque message d'un appareil est signé avec sa clé privée.
"""
from __future__ import annotations

import os
import secrets
import sys
from pathlib import Path

# Le plugin et ses modules communs vivent dans backend/ (voir Dockerfile : seuls les fichiers nécessaires y sont copiés).
_HERE = Path(__file__).resolve().parent
# Dans l'image : /app/app.py et /app/backend ; dans le dépôt : remote-rendezvous/app.py et backend/.
BACKEND = Path(os.environ.get("RENDEZVOUS_BACKEND_DIR") or next((p for p in (_HERE / "backend", _HERE.parent / "backend") if p.exists()), _HERE.parent / "backend"))
sys.path.insert(0, str(BACKEND))
# config.py génère et écrit des clés pour d'autres plugins s'il n'en trouve pas : inutile ici, on évite d'écrire un .env.
os.environ.setdefault("IP_CAMERAS_ENCRYPTION_KEY", "inutilise-sur-le-serveur-de-rendez-vous")
os.environ.setdefault("MQTT_ENCRYPTION_KEY", "inutilise-sur-le-serveur-de-rendez-vous")

from fastapi import FastAPI, HTTPException  # noqa: E402
from starlette.requests import HTTPConnection  # noqa: E402

import security  # noqa: E402
from plugins.remote.router import router  # noqa: E402

MIN_KEY_LENGTH = 24
# Routes réservées aux agents ; tout le reste est réservé au contrôleur.
AGENT_PATHS = frozenset({"/api/remote/devices/register", "/api/remote/agent/ws"})


class ConfigurationError(RuntimeError):
    pass


def check_keys(agent_key: str, controller_key: str) -> None:
    """Refuse de démarrer avec des clés absentes, courtes ou identiques : un serveur public sans clé solide serait une porte ouverte."""
    for name, key in (("RENDEZVOUS_AGENT_KEY", agent_key), ("RENDEZVOUS_CONTROLLER_KEY", controller_key)):
        if len(key) < MIN_KEY_LENGTH:
            raise ConfigurationError(f"{name} manquante ou trop courte (au moins {MIN_KEY_LENGTH} caractères) : génère-en une avec « openssl rand -hex 32 ».")
    if secrets.compare_digest(agent_key.encode(), controller_key.encode()):
        raise ConfigurationError("RENDEZVOUS_AGENT_KEY et RENDEZVOUS_CONTROLLER_KEY doivent être différentes.")


def make_guard(agent_key: str, controller_key: str):
    async def guard(connection: HTTPConnection) -> None:
        expected = agent_key if connection.url.path in AGENT_PATHS else controller_key
        given = connection.headers.get("x-api-key")
        if not given:
            raise HTTPException(status_code=401, detail="API key required")
        if not secrets.compare_digest(given.encode("utf-8", "replace"), expected.encode()):
            raise HTTPException(status_code=403, detail="Invalid API key")

    return guard


def create_app(agent_key: str, controller_key: str) -> FastAPI:
    check_keys(agent_key, controller_key)
    # Pas de documentation interactive ni de schéma OpenAPI sur un serveur public : rien à montrer aux curieux.
    app = FastAPI(title="PC Assistant Remote — rendez-vous", docs_url=None, redoc_url=None, openapi_url=None)
    # Le plugin protège chaque route par `security.require_api_key` (clé unique du backend) : ici on la remplace par la garde à deux rôles.
    app.dependency_overrides[security.require_api_key] = make_guard(agent_key, controller_key)
    app.include_router(router, prefix="/api/remote")

    @app.get("/health")
    async def health() -> dict:
        return {"status": "ok"}

    return app


def app_from_environment() -> FastAPI:
    return create_app(os.environ.get("RENDEZVOUS_AGENT_KEY", ""), os.environ.get("RENDEZVOUS_CONTROLLER_KEY", ""))


# `uvicorn app:app` : l'application n'est construite que si les variables sont là (sinon import sans effet, utile aux tests).
app = app_from_environment() if os.environ.get("RENDEZVOUS_AGENT_KEY") or os.environ.get("RENDEZVOUS_CONTROLLER_KEY") else None
