"""Optional protection for sensitive local API routes."""
import secrets

from fastapi import Header, HTTPException

from config import settings


async def require_api_key(x_api_key: str | None = Header(default=None)) -> None:
    """Require X-API-Key when API_AUTH_TOKEN is configured.

    The default remains compatible with the local development setup. Setting
    API_AUTH_TOKEN enables protection for configuration and calendar mutations.
    """
    if settings.API_AUTH_TOKEN and not x_api_key:
        raise HTTPException(status_code=401, detail="API key required")
    if settings.API_AUTH_TOKEN and not secrets.compare_digest(x_api_key, settings.API_AUTH_TOKEN):
        raise HTTPException(status_code=403, detail="Invalid API key")


# --- Limitation de débit (audit du 30/09/2026, point prioritaire) -----------------------------
# Fenêtre glissante en mémoire, par (nom de limite, adresse du client). Pas de dépendance externe :
# ARIA est un process unique et local, un compteur en mémoire suffit. Les compteurs sont perdus au
# redémarrage, ce qui est acceptable ici. Une limite à 0 (ou négative) désactive le contrôle.
import time
from collections import defaultdict, deque

from fastapi import Request

_rate_hits: dict[tuple[str, str], deque] = defaultdict(deque)


def rate_limit(name: str, limit_attr: str, window_seconds: float = 60.0):
    """Dépendance FastAPI : refuse (429 + Retry-After) au-delà de `settings.<limit_attr>` requêtes
    par fenêtre et par client. La limite est relue à chaque requête pour rester modifiable
    (et testable) sans redémarrage."""

    async def dependency(request: Request) -> None:
        limit = int(getattr(settings, limit_attr, 0))
        if limit <= 0:
            return
        client = request.client.host if request.client else "unknown"
        now = time.monotonic()
        bucket = _rate_hits[(name, client)]
        while bucket and now - bucket[0] > window_seconds:
            bucket.popleft()
        if len(bucket) >= limit:
            retry_after = max(1, int(window_seconds - (now - bucket[0])) + 1)
            raise HTTPException(
                status_code=429,
                detail="Trop de requêtes, réessaie dans quelques secondes",
                headers={"Retry-After": str(retry_after)},
            )
        bucket.append(now)

    return dependency


def reset_rate_limits() -> None:
    """Vide tous les compteurs (utilisé par les tests pour les isoler les uns des autres)."""
    _rate_hits.clear()
