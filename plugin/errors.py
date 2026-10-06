"""Erreurs du plugin Remote : celles du kit commun (plugin_sdk.errors, ce sont des HTTPException) plus deux cas propres à l'appairage."""
from plugin_sdk.errors import BadRequest as PairingFailed
from plugin_sdk.errors import Conflict as InvalidState
from plugin_sdk.errors import Forbidden as PermissionDenied
from plugin_sdk.errors import NotFound, PluginError as RemoteError, TooManyRequests as PairingLocked, Unavailable as AgentUnavailable

__all__ = ["AgentUnavailable", "InvalidState", "NotFound", "PairingFailed", "PairingLocked", "PermissionDenied", "RemoteError"]
