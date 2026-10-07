"""Générateur de plugin : `python -m plugin_sdk.scaffold <id> "<Nom affiché>" [--permission READ|WRITE|EXTERNAL|DESTRUCTIVE] [--no-chat]`

Crée backend/plugins/<id>/ (manifeste, routeur sécurisé, service, gestionnaire de chat + intents.json) et backend/tests/test_<id>_plugin.py.
Le résultat est immédiatement chargeable et ses tests passent : manifeste valide, toutes les routes exigent la clé d'API, phrases du chat reconnues.
Un plugin EXTERNAL ou DESTRUCTIVE démarre désactivé (« enabled_by_default »: false) : on l'active après l'avoir relu."""
from __future__ import annotations

import argparse
import json
from pathlib import Path

from .manifest import PERMISSIONS, validate

BACKEND = Path(__file__).resolve().parent.parent

_INIT = '"""Plugin __NAME__."""\n'

_ROUTER = '''"""Routes du plugin __NAME__ : la clé d'API est exigée sur chacune par `plugin_router` (plugin_router(public=True) pour ouvrir)."""
from plugin_sdk.router import plugin_router

from .service import service

router = plugin_router()


@router.get("/status")
async def status() -> dict:
    return service.status()
'''

_SERVICE = '''"""Logique du plugin __NAME__, indépendante de FastAPI et du chat : testable seule.

Erreurs : lever `plugin_sdk.errors.NotFound("…")`, `Forbidden`, `Conflict`, `Unavailable`… (ce sont des HTTPException : la réponse est
correcte sans code supplémentaire). Dépendance externe : la demander à `plugin_sdk.ports.ports` plutôt que de l'importer en dur."""
from __future__ import annotations


class Service:
    def status(self) -> dict:
        return {"plugin": "__ID__", "ready": True}


service = Service()
'''

_CHAT = '''"""Gestionnaire de chat du plugin __NAME__ : AUCUNE phrase à reconnaître ici, elles vivent dans intents.json."""
from plugin_sdk.chat import load_intents

from .service import service

INTENTS = load_intents(__file__, "__ID__")


def matches(message: str) -> bool:
    return INTENTS.match(message) is not None


async def handle(message: str, context: dict) -> dict:
    found = INTENTS.match(message)
    if found and found.intent == "status":
        return {"response": f"Plugin __NAME__ : {'prêt' if service.status()['ready'] else 'indisponible'}.", "source": "local", "source_type": "__ID__"}
    return {"response": "Je n'ai pas compris cette demande.", "source": "local", "source_type": "__ID__"}
'''

_TEST = '''"""Tests de base du plugin __NAME__ : manifeste valide, routes protégées__CHAT_TITLE__."""
import json
from pathlib import Path

from fastapi import FastAPI

from plugin_sdk.manifest import validate
from plugin_sdk.router import unprotected_routes
from plugins.__ID__.router import router
__CHAT_IMPORT__
PLUGIN = Path(__file__).resolve().parent.parent / "plugins" / "__ID__"


def test_manifest_is_valid():
    report = validate(json.loads((PLUGIN / "manifest.json").read_text(encoding="utf-8")), PLUGIN)
    assert report.ok, report.errors


def test_every_route_requires_the_api_key():
    app = FastAPI()
    app.include_router(router, prefix="/api/__PREFIX__")
    assert unprotected_routes(app) == []
__CHAT_TEST__'''

_CHAT_TEST = '''

def test_chat_examples_are_recognised_and_negatives_are_not():
    for name in chat_handler.INTENTS.names():
        spec = chat_handler.INTENTS.spec(name)
        for example in spec["examples"]:
            assert chat_handler.INTENTS.is_a(example, name), example
        for negative in spec["negatives"]:
            assert not chat_handler.matches(negative), negative
'''


def _intents(words: str) -> str:
    return json.dumps({
        "lexicon": {"ETAT": ["etat", "statut", "status"], "DET": ["du", "de", "de la", "le", "la", "l'"]},
        "intents": {"status": {"label": f"État du plugin {words}", "phrases": ["{ETAT} {?DET} " + words],
                               "examples": [f"statut du {words}", f"état {words}"], "negatives": ["bonjour"]}},
    }, ensure_ascii=False, indent=2) + "\n"


def create_plugin(plugin_id: str, name: str, permission: str = "READ", *, chat: bool = True, backend_dir: Path | None = None) -> list[Path]:
    """Écrit le plugin et son test ; renvoie les fichiers créés. Refuse d'écraser un plugin existant."""
    backend = backend_dir or BACKEND
    if permission not in PERMISSIONS:
        raise ValueError(f"permission : {' | '.join(PERMISSIONS)}")
    folder = backend / "plugins" / plugin_id
    if folder.exists():
        raise ValueError(f"Le plugin « {plugin_id} » existe déjà ({folder}).")
    manifest = {"id": plugin_id, "name": name, "version": "0.1.0", "description": f"{name} (généré par plugin_sdk.scaffold : à décrire).",
                "router_prefix": "/api/" + plugin_id.replace("_", "-"), "permission": permission, "enabled_by_default": permission == "READ"}
    if chat:
        manifest |= {"chat_handler": True, "chat_handler_order": 100, "chat_error_message": f"{name} indisponible"}
    report = validate(manifest)
    if not report.ok:
        raise ValueError(" ; ".join(report.errors))

    def fill(template: str) -> str:
        return (template.replace("__ID__", plugin_id).replace("__NAME__", name).replace("__PREFIX__", plugin_id.replace("_", "-"))
                .replace("__CHAT_TITLE__", ", phrases du chat reconnues" if chat else "")
                .replace("__CHAT_IMPORT__", "from plugins.%s import chat_handler\n" % plugin_id if chat else "")
                .replace("__CHAT_TEST__", _CHAT_TEST if chat else ""))

    files = {folder / "manifest.json": json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", folder / "__init__.py": fill(_INIT),
             folder / "router.py": fill(_ROUTER), folder / "service.py": fill(_SERVICE),
             backend / "tests" / f"test_{plugin_id}_plugin.py": fill(_TEST)}
    if chat:
        files |= {folder / "chat_handler.py": fill(_CHAT), folder / "intents.json": _intents(plugin_id.replace("_", " "))}
    created = []
    for path, content in files.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
        created.append(path)
    return created


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Crée le squelette d'un plugin ARIA.")
    parser.add_argument("id", help="identifiant : minuscules, chiffres, _ (ex. remote)")
    parser.add_argument("name", help="nom affiché (ex. « Contrôle à distance »)")
    parser.add_argument("--permission", default="READ", choices=PERMISSIONS)
    parser.add_argument("--no-chat", action="store_true", help="sans gestionnaire de chat")
    args = parser.parse_args(argv)
    try:
        created = create_plugin(args.id, args.name, args.permission, chat=not args.no_chat)
    except ValueError as error:
        print(f"Erreur : {error}")
        return 1
    print("Créé :\n" + "\n".join(f"  {path.relative_to(BACKEND)}" for path in created))
    risky = args.permission in ("EXTERNAL", "DESTRUCTIVE")
    print(f"\nSuite : pytest tests/test_{args.id}_plugin.py" + ("   (plugin désactivé par défaut : l'activer dans les réglages après relecture)" if risky else ""))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
