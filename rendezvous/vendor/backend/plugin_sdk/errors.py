"""Erreurs communes à tous les plugins.

`PluginError` EST une `HTTPException` : un service de plugin lève `NotFound("Appareil inconnu.")` et FastAPI répond 404 avec ce
message, sans table de correspondance ni `try/except` à réécrire dans chaque routeur. Le message est affichable tel quel."""
from __future__ import annotations

from fastapi import HTTPException


class PluginError(HTTPException):
    status_code = 400

    def __init__(self, message: str, *, headers: dict[str, str] | None = None) -> None:
        super().__init__(status_code=type(self).status_code, detail=message, headers=headers)
        self.message = message

    def __str__(self) -> str:
        return self.message


class BadRequest(PluginError):
    status_code = 400


class Forbidden(PluginError):
    status_code = 403


class NotFound(PluginError):
    status_code = 404


class Conflict(PluginError):
    status_code = 409


class TooManyRequests(PluginError):
    status_code = 429


class Unavailable(PluginError):
    """Un service dont dépend le plugin est absent ou injoignable (agent, appareil, fournisseur externe)."""
    status_code = 503
