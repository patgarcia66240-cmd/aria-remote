"""Routeur de plugin SÉCURISÉ PAR DÉFAUT : la clé d'API est exigée sur chaque route sans que l'auteur du plugin ait à y penser (l'audit du
30/09/2026 avait trouvé des routes oubliées). Ouvrir une route à tous est un choix explicite : `plugin_router(public=True)`."""
from __future__ import annotations

from typing import Any

from fastapi import APIRouter, Depends
from fastapi.routing import APIRoute

from security import rate_limit, require_api_key


def plugin_router(*, public: bool = False, rate_limit_setting: str | None = None, **kwargs: Any) -> APIRouter:
    """APIRouter avec la clé d'API exigée (sauf `public=True`) et, si `rate_limit_setting` nomme un réglage de config
    (ex. "REMOTE_PAIR_RATE_LIMIT"), une limite de requêtes par minute et par client."""
    dependencies = list(kwargs.pop("dependencies", []))
    if not public:
        dependencies.append(Depends(require_api_key))
    if rate_limit_setting:
        dependencies.append(Depends(rate_limit(rate_limit_setting.lower(), rate_limit_setting)))
    return APIRouter(dependencies=dependencies, **kwargs)


def _needs_key(dependant: Any) -> bool:
    return any(d.call is require_api_key or _needs_key(d) for d in dependant.dependencies)


def _walk(routes: Any, prefix: str = "", protected: bool = False):
    """Routes de l'application à plat avec leur chemin complet ; suit les routeurs inclus (FastAPI récent ne les met plus à plat)."""
    for route in routes:
        if isinstance(route, APIRoute):
            yield prefix + route.path, route, protected
            continue
        inner = getattr(route, "original_router", None)
        if inner is not None:
            context = getattr(route, "include_context", None)
            deps = getattr(context, "dependencies", None) or []
            yield from _walk(inner.routes, prefix + (getattr(context, "prefix", "") or ""),
                             protected or any(getattr(d, "dependency", None) is require_api_key for d in deps))


def unprotected_routes(app_or_router: Any) -> list[str]:
    """« MÉTHODE chemin » des routes qui n'exigent pas la clé d'API : outil d'audit (tests, diagnostic), jamais appelé en production."""
    return sorted(
        f"{','.join(sorted(route.methods - {'HEAD', 'OPTIONS'}))} {path}"
        for path, route, protected in _walk(app_or_router.routes)
        if not protected and not _needs_key(route.dependant)
    )
