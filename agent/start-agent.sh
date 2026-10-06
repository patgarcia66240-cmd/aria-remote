#!/usr/bin/env bash
# Lance l'agent Remote sans rien retaper : se place dans remote-agent, lit API_AUTH_TOKEN dans backend/.env et démarre cargo.
# Usage : ./remote-agent/start-agent.sh [--server URL] [--allow mouse,keyboard] [--name NOM] [--api-key CLE] [--auto-accept]
# (hors Windows, l'écran est une image de test et la saisie est seulement journalisée : voir README.md)
set -euo pipefail
cd "$(dirname "$0")"

args=("$@")
if [[ " ${args[*]:-} " != *" --api-key "* && -z "${REMOTE_API_KEY:-}" && -f ../backend/.env ]]; then
  key="$(sed -n 's/^[[:space:]]*API_AUTH_TOKEN[[:space:]]*=[[:space:]]*//p' ../backend/.env | head -n1 | tr -d "\"'\r")"
  if [[ -n "$key" ]]; then args+=(--api-key "$key"); echo "[agent] clé d'API lue dans backend/.env"; fi
fi
if [[ " ${args[*]:-} " != *" --allow "* ]]; then args+=(--allow mouse,keyboard); fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "Rust (cargo) n'est pas installé : https://rustup.rs" >&2
  exit 1
fi
exec cargo run --release -- "${args[@]}"
