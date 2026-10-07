#!/usr/bin/env bash
# Lance l'agent Remote sur le PC À CONTRÔLER. Au premier lancement l'agent demande l'adresse de PC Assistant (le PC qui contrôle) et sa clé
# d'API, puis les retient. Si backend/.env existe (PC de PC Assistant), la clé en est lue.
# Usage : ./start-agent.sh [--server URL] [--api-key CLE] [--allow mouse,keyboard] [--name NOM] [--auto-accept]
# (hors Windows, l'écran est une image de test et la saisie est seulement journalisée : voir README.md)
set -euo pipefail
cd "$(dirname "$0")"

args=("$@")
if [[ " ${args[*]:-} " != *" --api-key "* && -z "${REMOTE_API_KEY:-}" && -f ../backend/.env ]]; then
  key="$(sed -n 's/^[[:space:]]*API_AUTH_TOKEN[[:space:]]*=[[:space:]]*//p' ../backend/.env | head -n1 | tr -d "\"'\r")"
  if [[ -n "$key" ]]; then args+=(--api-key "$key"); echo "[agent] clé d'API lue dans backend/.env"; fi
fi

if [[ -x ./remote-agent ]]; then exec ./remote-agent "${args[@]}"; fi
if ! command -v cargo >/dev/null 2>&1; then
  echo "Ni ./remote-agent ni Rust (cargo) sur ce poste : copie l'exécutable compilé ici, ou installe Rust (https://rustup.rs)." >&2
  exit 1
fi
exec cargo run --release -- "${args[@]}"
