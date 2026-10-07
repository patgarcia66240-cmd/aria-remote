"""Configuration management"""
import re
from pydantic_settings import BaseSettings, SettingsConfigDict
from pydantic import field_validator
from dotenv import load_dotenv
import os
from pathlib import Path

# Chemin exposé pour que d'autres modules (ex: plugins/config/router.py, plugins/messaging/
# router.py) puissent persister des valeurs dans le même fichier .env que celui chargé ici,
# sans le recalculer ailleurs.
ENV_PATH = Path(__file__).resolve().parent / ".env"
load_dotenv(ENV_PATH)


def write_env_value(key: str, value, env_path: Path = ENV_PATH) -> None:
    """Met à jour (ou ajoute) une ligne KEY=value dans un fichier .env, sans toucher au reste.

    Partagé par plugins/config (préférences ARIA) et plugins/messaging (secrets/listes
    blanches WhatsApp/Telegram) — auparavant dupliqué dans plugins/config/router.py. Un
    `env_path` différent du .env du backend est accepté pour pouvoir écrire aussi dans
    bridges/whatsapp-bridge/.env (process Node.js séparé, voir plugins/messaging/router.py), qui doit
    garder EXACTEMENT le même WHATSAPP_BRIDGE_SECRET que ce backend."""
    safe_value = str(value).replace("\r", " ").replace("\n", " ")
    lines = env_path.read_text(encoding="utf-8").splitlines() if env_path.exists() else []
    pattern = re.compile(rf"^{re.escape(key)}=")
    for index, line in enumerate(lines):
        if pattern.match(line):
            lines[index] = f"{key}={safe_value}"
            break
    else:
        lines.append(f"{key}={safe_value}")
    env_path.parent.mkdir(parents=True, exist_ok=True)
    env_path.write_text("\n".join(lines) + "\n", encoding="utf-8")

# Chemin absolu (même principe que KINGS_DB_PATH dans kings_service.py) : avec un chemin relatif
# "./pc_assistant.db", le fichier de DB réellement utilisé dépend du dossier depuis lequel le
# process est lancé (uvicorn direct, gunicorn, desktop/main.rs...) — constaté en pratique : deux
# pc_assistant.db différents et désynchronisés existaient (un à la racine du repo, un dans
# backend/). Seule la valeur PAR DÉFAUT est ainsi rendue absolue ; un DATABASE_URL fourni via .env
# (y compris pour une autre base que SQLite) n'est pas touché.
_DEFAULT_DB_PATH = (Path(__file__).resolve().parent / "pc_assistant.db").as_posix()
_DEFAULT_KOKORO_DIR = Path(__file__).resolve().parent / "data" / "tts"

class Settings(BaseSettings):
    model_config = SettingsConfigDict(env_file=ENV_PATH, extra="ignore")
    CLAUDE_API_KEY: str = os.getenv("CLAUDE_API_KEY", "")
    CLAUDE_BASE_URL: str = os.getenv("CLAUDE_BASE_URL", "https://api.anthropic.com")
    CLAUDE_MODEL: str = os.getenv("CLAUDE_MODEL", "claude-sonnet-4-5-20250929")
    AI_PROVIDER: str = os.getenv("AI_PROVIDER", "anthropic")
    OPENAI_API_KEY: str = os.getenv("OPENAI_API_KEY", "")
    OPENAI_BASE_URL: str = os.getenv("OPENAI_BASE_URL", "https://api.openai.com/v1")
    OPENAI_MODEL: str = os.getenv("OPENAI_MODEL", "gpt-4.1-mini")
    OPENAI_IMAGE_MODEL: str = os.getenv("OPENAI_IMAGE_MODEL", "gpt-image-2")
    GEMINI_API_KEY: str = os.getenv("GEMINI_API_KEY", "")
    GEMINI_BASE_URL: str = os.getenv(
        "GEMINI_BASE_URL", "https://generativelanguage.googleapis.com/v1beta"
    )
    GEMINI_MODEL: str = os.getenv("GEMINI_MODEL", "gemini-2.5-flash")
    QWEN_API_KEY: str = os.getenv("QWEN_API_KEY", "")
    QWEN_BASE_URL: str = os.getenv(
        "QWEN_BASE_URL", "https://dashscope-intl.aliyuncs.com/compatible-mode/v1"
    )
    QWEN_MODEL: str = os.getenv("QWEN_MODEL", "qwen-plus")
    TWELVE_DATA_API_KEY: str = os.getenv("TWELVE_DATA_API_KEY", "")
    EODHD_API_KEY: str = os.getenv("EODHD_API_KEY", "")
    METAL_SENTINEL_API_KEY: str = os.getenv("METAL_SENTINEL_API_KEY", "")
    VIRUSTOTAL_API_KEY: str = os.getenv("VIRUSTOTAL_API_KEY", "")
    DATABASE_URL: str = os.getenv("DATABASE_URL", f"sqlite+aiosqlite:///{_DEFAULT_DB_PATH}")
    FILES_ROOT: str = os.getenv("FILES_ROOT", os.path.expanduser("~"))
    # Limite de taille pour /api/files/upload (audit.md, section 11) : la lecture par blocs
    # existante évite déjà de charger tout le fichier en mémoire, mais rien ne bornait la taille
    # totale — un envoi pouvait remplir le disque. 100 Mo par défaut, redéfinissable via .env.
    MAX_UPLOAD_SIZE_MB: int = int(os.getenv("MAX_UPLOAD_SIZE_MB", "100"))
    # Limitation de débit par client et par minute (audit du 30/09/2026) : le chat consomme des
    # crédits IA payants, l'upload écrit sur le disque. Valeurs larges pour ne jamais gêner un
    # usage normal (y compris vocal) ; 0 = désactivé. Redéfinissables via .env.
    RATE_LIMIT_CHAT_PER_MIN: int = int(os.getenv("RATE_LIMIT_CHAT_PER_MIN", "30"))
    RATE_LIMIT_UPLOAD_PER_MIN: int = int(os.getenv("RATE_LIMIT_UPLOAD_PER_MIN", "30"))
    REMOTE_PAIR_RATE_LIMIT: int = int(os.getenv("REMOTE_PAIR_RATE_LIMIT", "10"))   # essais d appairage par minute et par client (plugin remote)
    # Plugin remote, NAT (docs/REMOTE_ARCHITECTURE.md, section 9). Vide par défaut : aucun serveur tiers n'est contacté sans que tu l'aies choisi.
    # Réseau local : rien à régler. Via Internet : un STUN (ex. stun:mon-serveur:3478) et, si la connexion directe échoue, un TURN (coturn).
    REMOTE_PAIRING_CODE_TTL: int = int(os.getenv("REMOTE_PAIRING_CODE_TTL", "1800"))   # durée de validité d'un code d'appairage, en secondes (30 min par défaut)
    # Accès par Internet : adresse du serveur de rendez-vous (remote-rendezvous/) et clé du CONTRÔLEUR (RENDEZVOUS_CONTROLLER_KEY). Quand l'adresse est
    # renseignée, le plugin Remote relaie vers ce serveur au lieu de gérer les appareils lui-même (redémarrer le backend après modification).
    REMOTE_RENDEZVOUS_URL: str = os.getenv("REMOTE_RENDEZVOUS_URL", "")
    REMOTE_RENDEZVOUS_KEY: str = os.getenv("REMOTE_RENDEZVOUS_KEY", "")
    REMOTE_STUN_URLS: str = os.getenv("REMOTE_STUN_URLS", "")                 # séparés par des virgules
    REMOTE_TURN_URLS: str = os.getenv("REMOTE_TURN_URLS", "")                 # ex. turn:mon-serveur:3478?transport=udp
    REMOTE_TURN_SECRET: str = os.getenv("REMOTE_TURN_SECRET", "")             # static-auth-secret de coturn : identifiants éphémères par session
    REMOTE_TURN_USERNAME: str = os.getenv("REMOTE_TURN_USERNAME", "")         # sinon : identifiants fixes
    REMOTE_TURN_CREDENTIAL: str = os.getenv("REMOTE_TURN_CREDENTIAL", "")
    REMOTE_TURN_TTL: int = int(os.getenv("REMOTE_TURN_TTL", "3600"))          # durée de vie des identifiants éphémères (secondes)
    # Mémoire court terme du chat (plan P3, sprint 1 — claude/plan-p3-aria-1.0-2026-09-18.md) :
    # nombre de derniers messages de la conversation en cours renvoyés au modèle à chaque tour,
    # en plus du message courant. 0 = comportement d'avant (aucun historique envoyé au modèle,
    # même si l'UI en affichait un). Redéfinissable via .env, pour pouvoir revenir en arrière
    # instantanément si le coût/latence supplémentaire pose problème.
    CHAT_HISTORY_WINDOW: int = int(os.getenv("CHAT_HISTORY_WINDOW", "12"))
    ARIA_NAME: str = os.getenv("ARIA_NAME", "ARIA")
    ARIA_AVATAR: str = os.getenv("ARIA_AVATAR", "🤖")
    ARIA_LANGUAGE: str = os.getenv("ARIA_LANGUAGE", "fr")
    CORS_ORIGINS: str = os.getenv("CORS_ORIGINS", "http://localhost:5173,http://tauri.localhost,https://tauri.localhost")
    API_HOST: str = os.getenv("API_HOST", "127.0.0.1")
    API_PORT: int = int(os.getenv("API_PORT", "8000"))
    API_AUTH_TOKEN: str = os.getenv("API_AUTH_TOKEN", "")
    KOKORO_MODEL_PATH: str = os.getenv(
        "KOKORO_MODEL_PATH", str(_DEFAULT_KOKORO_DIR / "kokoro-v1.0.onnx")
    )
    KOKORO_VOICES_PATH: str = os.getenv(
        "KOKORO_VOICES_PATH", str(_DEFAULT_KOKORO_DIR / "voices-v1.0.bin")
    )
    KOKORO_VOICE: str = os.getenv("KOKORO_VOICE", "ff_siwis")
    # Recherche web pour « vérifier » une affirmation du fact-check (plugin url_reader, verify.py). Facultative : sans clé, seule Wikipédia est consultée.
    # Clé gratuite sur https://api.search.brave.com (2 000 requêtes par mois) ; à mettre dans .env : BRAVE_SEARCH_API_KEY=...
    BRAVE_SEARCH_API_KEY: str = os.getenv("BRAVE_SEARCH_API_KEY", "")
    PIPER_MODEL_PATH: str = os.getenv(
        "PIPER_MODEL_PATH", str(_DEFAULT_KOKORO_DIR / "fr_FR-siwis-medium.onnx")
    )
    PIPER_CONFIG_PATH: str = os.getenv(
        "PIPER_CONFIG_PATH", str(_DEFAULT_KOKORO_DIR / "fr_FR-siwis-medium.onnx.json")
    )
    # Moteurs TTS actifs (liste séparée par des virgules) et moteurs chargés en mémoire au démarrage.
    # Par défaut aucun préchargement : le modèle ne se charge qu'à la première synthèse.
    TTS_ENABLED_ENGINES: str = os.getenv("TTS_ENABLED_ENGINES", "kokoro,piper")
    TTS_PRELOAD: str = os.getenv("TTS_PRELOAD", "")
    PIPER_USE_CUDA: bool = os.getenv("PIPER_USE_CUDA", "false").lower() in {"1", "true", "yes"}
    # Coordonnées par défaut : Saint-Martin-de-Crau (13310), utilisées pour la météo/lever-coucher
    # du soleil de la carte Saint du jour. Vérifiées le 10/09/2026 (coordonneesgps.net +
    # infobox Wikipedia, valeurs cohérentes à ~1km près). Redéfinissable via .env si besoin.
    USER_LATITUDE: float = float(os.getenv("USER_LATITUDE", "43.6408"))
    USER_LONGITUDE: float = float(os.getenv("USER_LONGITUDE", "4.8133"))
    USER_CITY_LABEL: str = os.getenv("USER_CITY_LABEL", "Saint-Martin-de-Crau")
    USER_COUNTRY: str = os.getenv("USER_COUNTRY", "France")
    # Identifiants OAuth Google Agenda (projet Google Cloud à créer par l'utilisatrice — voir
    # instructions de configuration fournies séparément). Vides par défaut : l'onglet Agenda le
    # détecte et affiche un état "non configuré" plutôt que d'échouer silencieusement.
    GOOGLE_CLIENT_ID: str = os.getenv("GOOGLE_CLIENT_ID", "")
    GOOGLE_CLIENT_SECRET: str = os.getenv("GOOGLE_CLIENT_SECRET", "")
    GOOGLE_CALENDAR_REDIRECT_URI: str = os.getenv(
        "GOOGLE_CALENDAR_REDIRECT_URI", "http://127.0.0.1:8000/api/calendar/oauth/callback"
    )
    # Pont WhatsApp (backend/plugins/whatsapp/, voir bridges/whatsapp-bridge/ à la racine du repo) : un
    # service Node.js séparé (Baileys, non officiel) qui parle à WhatsApp et relaie les messages
    # ici via POST /api/whatsapp/incoming. WHATSAPP_BRIDGE_SECRET protège cet échange (le pont et
    # ce backend doivent avoir exactement la même valeur, voir bridges/whatsapp-bridge/.env) — sans elle
    # le plugin refuse toute requête (503). WHATSAPP_ALLOWED_NUMBERS est vide par défaut : tant
    # qu'aucun numéro n'y est ajouté, ARIA ignore tous les messages WhatsApp entrants (pour ne
    # jamais répondre à un inconnu qui aurait trouvé le numéro dédié).
    WHATSAPP_BRIDGE_URL: str = os.getenv("WHATSAPP_BRIDGE_URL", "http://127.0.0.1:3001")
    WHATSAPP_BRIDGE_SECRET: str = os.getenv("WHATSAPP_BRIDGE_SECRET", "")
    WHATSAPP_ALLOWED_NUMBERS: str = os.getenv("WHATSAPP_ALLOWED_NUMBERS", "")
    # Bot Telegram (backend/telegram_bot.py, script séparé lancé à côté de main.py + voir
    # backend/plugins/telegram/) : API officielle Telegram, gratuite, contrairement à WhatsApp —
    # token créé une fois via @BotFather sur Telegram (aucune approbation, aucun coût). Beaucoup
    # plus simple que le pont WhatsApp : pas de service Node.js séparé à faire tourner en
    # permanence, telegram_bot.py interroge directement POST /api/telegram/incoming et attend la
    # réponse (pas de webhook retour). Mêmes principes de sécurité : TELEGRAM_BRIDGE_SECRET
    # (identique dans backend/.env, lu par les deux côtés puisque c'est le même process Python) et
    # TELEGRAM_ALLOWED_USER_IDS (identifiants Telegram numériques, vide par défaut = aucun message
    # traité).
    TELEGRAM_BOT_TOKEN: str = os.getenv("TELEGRAM_BOT_TOKEN", "")
    TELEGRAM_BRIDGE_SECRET: str = os.getenv("TELEGRAM_BRIDGE_SECRET", "")
    TELEGRAM_ALLOWED_USER_IDS: str = os.getenv("TELEGRAM_ALLOWED_USER_IDS", "")
    # Chiffrement au repos des mots de passe de caméras IP (plugins/ip_cameras/client.py,
    # backend/data/ip_cameras.json) : générée automatiquement si absente (voir plus bas), comme
    # API_AUTH_TOKEN ci-dessous. Clé Fernet (32 octets base64) partagée par toutes les caméras
    # enregistrées — plusieurs caméras, une seule clé de chiffrement.
    IP_CAMERAS_ENCRYPTION_KEY: str = os.getenv("IP_CAMERAS_ENCRYPTION_KEY", "")
    # Même principe pour le mot de passe du broker MQTT (plugins/mqtt/client.py,
    # backend/data/mqtt_config.json) — passerelle capteurs ajoutée le 19/09/2026.
    MQTT_ENCRYPTION_KEY: str = os.getenv("MQTT_ENCRYPTION_KEY", "")
    # Coffre de secrets Windows (plan P3, sprint 3 — claude/plan-p3-aria-1.0-2026-09-18.md,
    # services/secret_store.py). Off par défaut : tant que USE_SECRET_STORE n'est pas "true"
    # dans .env, ce flag ne change RIEN au comportement actuel (mêmes valeurs .env qu'avant).
    # Voir _apply_secret_store_overrides ci-dessous et scripts/migrate_secrets_to_store.py.
    USE_SECRET_STORE: bool = os.getenv("USE_SECRET_STORE", "false").lower() in {"1", "true", "yes"}
    DEBUG: bool = False

    @field_validator("DEBUG", mode="before")
    @classmethod
    def parse_debug(cls, value):
        if isinstance(value, str):
            normalized = value.strip().lower()
            if normalized in {"development", "dev", "debug"}:
                return True
            if normalized in {"release", "production", "prod"}:
                return False
        return value

# Clés migrées par scripts/migrate_secrets_to_store.py (même liste, importée par ce script pour
# ne jamais désynchroniser les deux) : les clés des fournisseurs IA + les 3 API de données
# financières signalées par l'audit du 10/09/2026 (rotation des clés) — voir aussi le docstring
# de services/secret_store.py. Ni les secrets de pont WhatsApp/Telegram ni le client_secret
# Google : hors périmètre de ce sprint (plan-p3-aria-1.0-2026-09-18.md, sprint 3).
SECRET_STORE_KEYS = (
    "CLAUDE_API_KEY",
    "OPENAI_API_KEY",
    "GEMINI_API_KEY",
    "QWEN_API_KEY",
    "METAL_SENTINEL_API_KEY",
    "TWELVE_DATA_API_KEY",
    "EODHD_API_KEY",
)


def _apply_secret_store_overrides(settings_obj: "Settings") -> None:
    """Si USE_SECRET_STORE est actif, remplace chaque clé de SECRET_STORE_KEYS par la valeur
    déjà présente dans le coffre Windows — sinon garde la valeur .env déjà chargée (get_secret()
    ne casse jamais rien, voir services/secret_store.py : coffre absent/indisponible/clé pas
    encore migrée retombent tous sur env_fallback). Extrait en fonction à part (plutôt qu'inline
    ci-dessous) pour rester testable sans réimporter tout le module — voir
    tests/test_secret_store_flag.py."""
    if not settings_obj.USE_SECRET_STORE:
        return
    from services.secret_store import get_secret

    for key in SECRET_STORE_KEYS:
        current = getattr(settings_obj, key)
        setattr(settings_obj, key, get_secret(key, env_fallback=current) or current)


settings = Settings()
_apply_secret_store_overrides(settings)

# Génère et persiste automatiquement une clé API dès que le backend écoute au-delà de
# 127.0.0.1 (typiquement API_HOST=0.0.0.0 pour que l'appli Android/ARIA le joigne en Wi-Fi) et
# qu'aucune clé n'a déjà été choisie à la main dans .env. Sans ça, ouvrir l'écoute réseau
# exposerait /api/files et /api/system (voir plugins/files/router.py, plugins/system/router.py)
# sans aucune protection à quiconque est sur le même réseau. Affichée une seule fois au
# démarrage : à reporter dans les réglages réseau de l'appli Android.
if settings.API_HOST != "127.0.0.1" and not settings.API_AUTH_TOKEN:
    import secrets as _secrets

    _generated_token = _secrets.token_hex(24)
    settings.API_AUTH_TOKEN = _generated_token
    write_env_value("API_AUTH_TOKEN", _generated_token)
    print(
        f"[ARIA] Accès réseau local activé (API_HOST={settings.API_HOST}) : "
        f"clé API générée automatiquement -> {_generated_token}"
    )
    print(
        "[ARIA] Renseigne cette clé dans l'appli Android (Réglages > Connexion PC). "
        "Elle est aussi sauvegardée dans backend/.env (API_AUTH_TOKEN)."
    )

# Même principe que le bloc API_AUTH_TOKEN ci-dessus, mais inconditionnel (pas besoin d'ouvrir
# le réseau pour que les mots de passe de caméras IP méritent d'être chiffrés au repos — voir
# plugins/ip_cameras/client.py). Générée une seule fois puis réutilisée à chaque démarrage.
if not settings.IP_CAMERAS_ENCRYPTION_KEY:
    from cryptography.fernet import Fernet as _Fernet

    _generated_camera_key = _Fernet.generate_key().decode("ascii")
    settings.IP_CAMERAS_ENCRYPTION_KEY = _generated_camera_key
    write_env_value("IP_CAMERAS_ENCRYPTION_KEY", _generated_camera_key)

# Même principe, pour le mot de passe du broker MQTT (plugins/mqtt/client.py).
if not settings.MQTT_ENCRYPTION_KEY:
    from cryptography.fernet import Fernet as _FernetMqtt

    _generated_mqtt_key = _FernetMqtt.generate_key().decode("ascii")
    settings.MQTT_ENCRYPTION_KEY = _generated_mqtt_key
    write_env_value("MQTT_ENCRYPTION_KEY", _generated_mqtt_key)
