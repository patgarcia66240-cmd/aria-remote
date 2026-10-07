# ARIA Remote

Contrôle d'un PC à distance, sécurisé : un **agent Windows** (fenêtre locale, code d'appairage à 6 chiffres), un **serveur de rendez-vous** (WebRTC, STUN/TURN), un **plugin** et une **interface** pour ARIA (PC Assistant), et la **page de téléchargement** de l'agent.

| Dossier | Contenu |
|---|---|
| `agent/` | l'agent Rust (`remote-agent.exe`) : capture, souris, clavier, fenêtre locale |
| `rendezvous/` | le serveur public (FastAPI + coturn), déploiement VPS/Docker |
| `download/` | la page de téléchargement (nginx), construite depuis les releases de ce dépôt |
| `plugin/` | le plugin Remote d'ARIA (appairage, sessions, signaling) |
| `ui/` | l'onglet « Maintenance à distance » d'ARIA (React) |
| `docs/` | architecture et sécurité (`REMOTE_ARCHITECTURE.md`) |

## Versions
L'agent suit son propre numéro (`agent/Cargo.toml`). Une release se publie avec une étiquette identique : `git tag agent-v1.5.0 && git push origin agent-v1.5.0`.
La compilation refuse une étiquette qui ne correspond pas au `Cargo.toml`. La page de téléchargement affiche la dernière version et garde les anciennes.

Les versions 0.1.0 à 0.1.4 ont été publiées dans le dépôt `pc-assistant`, dont ce projet est issu.
