"""Aide pour les gestionnaires de chat : les phrases à reconnaître vivent dans `intents.json` (à côté du plugin), jamais dans le code."""
from __future__ import annotations

from pathlib import Path

from services.intents import Intents

USER_INTENTS_DIR = Path(__file__).resolve().parent.parent / "data" / "intents"


def load_intents(plugin_file: str, user_name: str | None = None) -> Intents:
    """Intentions du plugin dont le fichier Python est `plugin_file` (passer `__file__`) : `intents.json` voisin, plus le fichier
    facultatif de l'utilisatrice `data/intents/<user_name>.json` qui AJOUTE des formulations sans rien retirer."""
    base = Path(plugin_file).with_name("intents.json")
    return Intents(base, USER_INTENTS_DIR / f"{user_name}.json" if user_name else None)
