"""Journal d'audit des actions sensibles (WRITE/DESTRUCTIVE/EXTERNAL), voir audit.md section 24.

Format JSONL (une action par ligne, backend/data/audit_log.jsonl) — même convention que les
autres états persistés en JSON (plugins_state.json, ip_cameras.json...), pas de nouvelle table
SQLite pour rester simple et lisible directement (ex. `Get-Content audit_log.jsonl -Tail 20`).

Best-effort : une erreur d'écriture du journal ne doit JAMAIS faire échouer l'action réelle
qu'elle décrit (mieux vaut une suppression de fichier réussie sans log qu'un journal qui bloque
l'application) — voir log_action() ci-dessous.

Portée volontairement limitée pour l'instant à ce qui est réellement câblé : la suppression de
fichiers (le seul exemple concret de l'audit, section 24). Étendre à d'autres outils
WRITE/EXTERNAL (messagerie, agenda...) est un chantier séparé, pas fait ici pour rester
progressif (voir audit.md, section 14 : "ne pas tout casser d'un coup").
"""
import json
import logging
from datetime import datetime, timezone
from pathlib import Path
from threading import Lock

logger = logging.getLogger(__name__)

LOG_PATH = Path(__file__).resolve().parent.parent / "data" / "audit_log.jsonl"
_write_lock = Lock()

# Volume raisonnable pour un usage perso : au-delà, on tronque le fichier en gardant les entrées
# les plus récentes plutôt que de le laisser grossir indéfiniment (voir _rotate_if_needed).
MAX_ENTRIES = 5000
_KEEP_ON_ROTATE = 2000


def _rotate_if_needed() -> None:
    if not LOG_PATH.exists():
        return
    try:
        lines = LOG_PATH.read_text(encoding="utf-8").splitlines()
    except OSError:
        return
    if len(lines) <= MAX_ENTRIES:
        return
    LOG_PATH.write_text("\n".join(lines[-_KEEP_ON_ROTATE:]) + "\n", encoding="utf-8")


def log_action(
    *,
    tool: str,
    action: str,
    target: str,
    result: str,
    confirmation: bool | None = None,
    conversation_id: str | None = None,
    detail: str | None = None,
) -> None:
    """Ajoute une entrée au journal. Ne lève jamais d'exception : un échec d'écriture est
    seulement loggé (logger.warning), pour ne jamais interrompre l'action qu'on essaie de tracer.

    Exemple (voir audit.md, section 24) :
        log_action(tool="files.delete", action="DELETE", target="C:/.../test.txt",
                   result="success", confirmation=True)
    """
    entry = {
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "tool": tool,
        "action": action,
        "target": target,
        "result": result,
        "confirmation": confirmation,
        "conversation_id": conversation_id,
        "detail": detail,
    }
    try:
        with _write_lock:
            LOG_PATH.parent.mkdir(parents=True, exist_ok=True)
            with open(LOG_PATH, "a", encoding="utf-8") as handle:
                handle.write(json.dumps(entry, ensure_ascii=False) + "\n")
            _rotate_if_needed()
    except OSError as error:
        logger.warning("Écriture du journal d'audit échouée (%s) : %s", tool, error)


def read_recent(limit: int = 100) -> list[dict]:
    """Renvoie les `limit` entrées les plus récentes (les plus récentes en premier)."""
    if not LOG_PATH.exists():
        return []
    try:
        lines = LOG_PATH.read_text(encoding="utf-8").splitlines()
    except OSError:
        return []
    entries = []
    for line in reversed(lines[-limit:]):
        try:
            entries.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    return entries
