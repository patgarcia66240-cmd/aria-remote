"""Contrat d'un `manifest.json` de plugin, vérifié au chargement (plugin_loader) et dans les tests.

Deux niveaux de retour : `errors` (le manifeste est inutilisable ou incohérent avec les fichiers du plugin) et `advice` (recommandations
de sécurité ou de style : jamais bloquantes, pour ne pas casser les plugins existants)."""
from __future__ import annotations

import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

PERMISSIONS = ("READ", "WRITE", "EXTERNAL", "DESTRUCTIVE")
RISKY = ("EXTERNAL", "DESTRUCTIVE")           # agissent hors de l'application ou détruisent : démarrent désactivés sauf raison contraire
KNOWN_KEYS = {"id", "name", "version", "description", "router_prefix", "permission", "enabled_by_default", "chat_handler", "chat_handler_order",
              "chat_error_message", "frontend", "lifecycle", "_dir"}
_ID = re.compile(r"^[a-z][a-z0-9_]{1,40}$")
_VERSION = re.compile(r"^\d+\.\d+\.\d+$")
_PREFIX = re.compile(r"^/api/[a-z0-9][a-z0-9/_-]*$")


@dataclass
class ManifestReport:
    errors: list[str] = field(default_factory=list)
    advice: list[str] = field(default_factory=list)

    @property
    def ok(self) -> bool:
        return not self.errors


def validate(manifest: dict[str, Any], directory: Path | None = None) -> ManifestReport:
    """Vérifie un manifeste ; avec `directory` (dossier du plugin) vérifie aussi la cohérence avec ses fichiers."""
    report = ManifestReport()
    err, tip = report.errors.append, report.advice.append
    plugin_id = manifest.get("id")
    if not isinstance(plugin_id, str) or not _ID.match(plugin_id):
        err("« id » obligatoire : minuscules, chiffres et _ (ex. « remote »).")
    elif directory and plugin_id != directory.name:
        err(f"« id » ({plugin_id}) doit être le nom du dossier ({directory.name}).")
    if not isinstance(manifest.get("name"), str) or not manifest.get("name", "").strip():
        err("« name » obligatoire (nom affiché).")
    if "version" in manifest and not _VERSION.match(str(manifest["version"])):
        err("« version » au format 1.2.3.")
    permission = manifest.get("permission", "READ")
    if permission not in PERMISSIONS:
        err(f"« permission » doit valoir {' | '.join(PERMISSIONS)}.")
    prefix = manifest.get("router_prefix")
    if prefix is not None and not _PREFIX.match(str(prefix)):
        err("« router_prefix » doit commencer par /api/ (ex. « /api/remote »).")
    order = manifest.get("chat_handler_order", 100)
    if not isinstance(order, int) or isinstance(order, bool) or not 0 <= order <= 1000:
        err("« chat_handler_order » : entier de 0 à 1000 (le plus petit passe en premier).")
    for flag in ("enabled_by_default", "chat_handler", "lifecycle"):
        if flag in manifest and not isinstance(manifest[flag], bool):
            err(f"« {flag} » doit être true ou false.")

    if directory:
        if prefix and not (directory / "router.py").exists():
            err("« router_prefix » déclaré mais router.py est absent.")
        if not prefix and (directory / "router.py").exists():
            tip("router.py existe mais « router_prefix » est absent : le routeur ne sera pas monté.")
        if manifest.get("chat_handler") and not (directory / "chat_handler.py").exists():
            err("« chat_handler » est true mais chat_handler.py est absent.")
        if manifest.get("lifecycle") and not (directory / "lifecycle.py").exists():
            err("« lifecycle » est true mais lifecycle.py est absent.")

    if permission in RISKY and manifest.get("enabled_by_default", True) is not False:
        tip(f"Plugin {permission} activé par défaut : prévoir « enabled_by_default »: false ou une confirmation avant toute action.")
    if "enabled_by_default" not in manifest:
        tip("« enabled_by_default » absent : le plugin sera activé par défaut.")
    unknown = sorted(set(manifest) - KNOWN_KEYS)
    if unknown:
        tip("Clés inconnues (ignorées) : " + ", ".join(unknown) + ".")
    return report
