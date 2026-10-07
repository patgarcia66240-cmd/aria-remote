"""Mode « serveur de rendez-vous » du plugin Remote (accès par Internet, docs/REMOTE_ARCHITECTURE.md §24).

Avec REMOTE_RENDEZVOUS_URL, ce backend ne garde plus lui-même les appareils, les codes et les sessions : il relaie les appels de l'interface vers le
serveur de rendez-vous public (rendezvous/), avec la clé du CONTRÔLEUR. L'interface ne change pas (mêmes routes /api/remote/*) et la clé du
serveur ne quitte jamais ce backend : le navigateur ne la voit pas. Les agents, eux, se connectent DIRECTEMENT au serveur de rendez-vous
(ils n'ont pas besoin d'atteindre ce PC)."""
from __future__ import annotations

import httpx
from fastapi import APIRouter, Request, Response

from config import settings
from plugin_sdk.router import plugin_router

# Les confirmations de l'appareil contrôlé (jusqu'à 60 s) se font pendant la création de session : le délai doit les couvrir.
TIMEOUT = httpx.Timeout(75.0, connect=10.0)
# Routes d'agent : elles n'existent que sur le serveur de rendez-vous, jamais relayées (le contrôleur n'a pas à agir en tant qu'agent).
AGENT_PREFIXES = ("agent/", "devices/register")

_client: httpx.AsyncClient | None = None


def configured() -> bool:
    return bool(settings.REMOTE_RENDEZVOUS_URL.strip())


def base_url() -> str:
    return settings.REMOTE_RENDEZVOUS_URL.strip().rstrip("/")


def get_client() -> httpx.AsyncClient:
    global _client
    if _client is None:
        _client = httpx.AsyncClient(timeout=TIMEOUT)
    return _client


def set_client(client: httpx.AsyncClient | None) -> None:
    """Remplace le client HTTP (tests : transport ASGI vers une application de rendez-vous en mémoire)."""
    global _client
    _client = client


def unreachable(error: Exception) -> Response:
    return Response(
        content=f'{{"detail": "Serveur de rendez-vous injoignable ({base_url()}) : {type(error).__name__}"}}',
        status_code=503, media_type="application/json")


async def call(method: str, path: str, *, params: dict | None = None, content: bytes | None = None) -> httpx.Response:
    """Appel au serveur de rendez-vous avec la clé du contrôleur. Lève httpx.HTTPError s'il est injoignable."""
    headers = {"x-api-key": settings.REMOTE_RENDEZVOUS_KEY}
    if content:
        headers["content-type"] = "application/json"
    return await get_client().request(method, f"{base_url()}/api/remote/{path.lstrip('/')}", params=params, content=content, headers=headers)


def build_router() -> APIRouter:
    router = plugin_router()     # la clé d'API LOCALE reste exigée : seul ton interface peut utiliser ce relais

    @router.api_route("/{path:path}", methods=["GET", "POST", "DELETE"])
    async def forward(path: str, request: Request) -> Response:
        if path.startswith(AGENT_PREFIXES):
            return Response(content='{"detail": "Route réservée aux agents."}', status_code=404, media_type="application/json")
        try:
            upstream = await call(request.method, path, params=dict(request.query_params), content=await request.body() or None)
        except httpx.HTTPError as error:
            return unreachable(error)
        headers = {name: value for name, value in upstream.headers.items() if name.lower() == "retry-after"}
        content = upstream.content
        if path == "status" and upstream.status_code == 200:
            try:
                data = upstream.json()
                data["mode"] = "rendezvous"
                data["server"] = base_url()
                content = httpx.Response(200, json=data).content
            except ValueError:
                pass
        return Response(content=content, status_code=upstream.status_code, headers=headers, media_type="application/json")

    return router
