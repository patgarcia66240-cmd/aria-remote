#!/usr/bin/env bash
# Installe le serveur de rendez-vous sur un VPS Linux (Hostinger ou autre) : Docker, secrets, pare-feu, HTTPS (Caddy) et relais (coturn).
#
# À lancer SUR LE VPS, depuis une copie du dépôt, en root (ou avec sudo) :
#   git clone -b main https://github.com/patgarcia66240-cmd/aria-remote.git
#   cd aria-remote/rendezvous
#   sudo ./install-vps.sh --domain rendezvous.mondomaine.fr
#   sudo ./install-vps.sh --sslip            # sans nom de domaine : utilise <ip>.sslip.io (HTTPS valide quand même)
#
# Options : --domain NOM | --sslip | --dry-run (n'exécute rien, montre ce qui serait fait) | --no-docker-install | --no-firewall | --no-traefik
# Si un Traefik tourne déjà sur ce serveur (modèle « Ubuntu avec n8n » de Hostinger), le script s'y branche au lieu d'installer son propre HTTPS.
# Relancer le script est sans danger : le fichier .env existant (donc les clés) est conservé.
set -euo pipefail
cd "$(dirname "$0")"

DOMAIN=""; SSLIP=0; DRY=0; INSTALL_DOCKER=1; FIREWALL=1; USE_TRAEFIK=auto
while [[ $# -gt 0 ]]; do
  case "$1" in
    --domain) DOMAIN="${2:-}"; shift 2 ;;
    --sslip) SSLIP=1; shift ;;
    --dry-run) DRY=1; shift ;;
    --no-docker-install) INSTALL_DOCKER=0; shift ;;
    --no-firewall) FIREWALL=0; shift ;;
    --no-traefik) USE_TRAEFIK=0; shift ;;
    -h|--help) sed -n '2,15p' "$0"; exit 0 ;;
    *) echo "Option inconnue : $1 (voir --help)" >&2; exit 2 ;;
  esac
done

say() { printf '\n== %s\n' "$*"; }
run() { if [[ $DRY -eq 1 ]]; then echo "   [simulation] $*"; else "$@"; fi; }
secret() { openssl rand -hex 32; }

[[ $DRY -eq 1 || $EUID -eq 0 ]] || { echo "Lance ce script en root : sudo ./install-vps.sh ..." >&2; exit 1; }

# -- 1. Nom de domaine ------------------------------------------------------------------------------------------------------------
public_ip() { curl -4 -fsS --max-time 8 https://api.ipify.org 2>/dev/null || true; }
if [[ -f .env ]] && grep -q '^DOMAIN=.\+' .env; then
  DOMAIN="$(grep '^DOMAIN=' .env | head -n1 | cut -d= -f2-)"
  say "Configuration existante conservée (.env) : domaine $DOMAIN"
else
  if [[ $SSLIP -eq 1 ]]; then
    ip="$(public_ip)"; [[ -n "$ip" ]] || { echo "Adresse IP publique introuvable : utilise --domain." >&2; exit 1; }
    DOMAIN="${ip//./-}.sslip.io"
  fi
  [[ -n "$DOMAIN" ]] || { echo "Il faut un nom de domaine : --domain rendezvous.mondomaine.fr (ou --sslip)." >&2; exit 2; }
  say "Domaine : $DOMAIN"
fi

# Le DNS du domaine doit pointer vers ce serveur, sinon Let's Encrypt ne peut pas délivrer le certificat.
ip="$(public_ip)"; resolved="$(getent hosts "$DOMAIN" 2>/dev/null | awk '{print $1; exit}' || true)"
if [[ -n "$ip" && -n "$resolved" && "$ip" != "$resolved" ]]; then
  echo "ATTENTION : $DOMAIN pointe vers $resolved, mais ce serveur est $ip. Corrige l'enregistrement DNS (type A) avant de continuer." >&2
  [[ $DRY -eq 1 ]] || exit 1
elif [[ -z "$resolved" ]]; then
  echo "ATTENTION : $DOMAIN ne se résout pas encore. Crée un enregistrement DNS de type A vers ${ip:-adresse IP du serveur} et patiente quelques minutes." >&2
  [[ $DRY -eq 1 ]] || exit 1
fi

# -- 2. Traefik déjà présent ? Ports 80 / 443 libres sinon ----------------------------------------------------------------------
COMPOSE_FILE=docker-compose.yml
ports_busy() { command -v ss >/dev/null 2>&1 && ss -ltn "sport = :$1" 2>/dev/null | grep -q LISTEN; }
our_caddy_running() { docker compose -f docker-compose.yml ps --services --status running 2>/dev/null | grep -qx caddy; }
traefik_container() { command -v docker >/dev/null 2>&1 && docker ps --format '{{.Names}}|{{.Image}}' 2>/dev/null | awk -F'|' 'tolower($2) ~ /traefik/ {print $1; exit}'; }

TRAEFIK_NAME=""
if [[ "$USE_TRAEFIK" != "0" ]]; then TRAEFIK_NAME="$(traefik_container || true)"; fi
if [[ -n "$TRAEFIK_NAME" ]]; then
  say "Traefik détecté ($TRAEFIK_NAME) : le serveur s'y branche, sans toucher à ce qui existe (n8n, etc.)"
  COMPOSE_FILE=docker-compose.traefik.yml
else
  for port in 80 443; do
    if ports_busy "$port" && ! our_caddy_running; then
      echo "Le port $port est déjà utilisé par un autre service de ce serveur (Nginx, Apache…). Libère-le, ou installe derrière ce service." >&2
      [[ $DRY -eq 1 ]] || exit 1
    fi
  done
fi

# -- 3. Docker --------------------------------------------------------------------------------------------------------------------
if ! command -v docker >/dev/null 2>&1; then
  if [[ $INSTALL_DOCKER -eq 1 ]]; then
    say "Installation de Docker (script officiel get.docker.com)"
    if [[ $DRY -eq 1 ]]; then echo "   [simulation] curl -fsSL https://get.docker.com | sh"; else curl -fsSL https://get.docker.com | sh; fi
  else
    echo "Docker est absent (--no-docker-install) : installe-le puis relance." >&2; exit 1
  fi
fi
[[ $DRY -eq 1 ]] || docker compose version >/dev/null 2>&1 || { echo "« docker compose » indisponible : installe le plugin Docker Compose v2." >&2; exit 1; }

# -- 4. Secrets (.env) ------------------------------------------------------------------------------------------------------------
if [[ ! -f .env ]]; then
  say "Création de .env avec des secrets aléatoires"
  if [[ $DRY -eq 1 ]]; then
    echo "   [simulation] .env : DOMAIN=$DOMAIN + 3 secrets de 64 caractères hexadécimaux (droits 600)"
  else
    umask 077
    cat > .env <<EOF
DOMAIN=$DOMAIN
RENDEZVOUS_AGENT_KEY=$(secret)
RENDEZVOUS_CONTROLLER_KEY=$(secret)
TURN_SECRET=$(secret)
PAIRING_CODE_TTL=1800
PAIR_RATE_LIMIT=10
EOF
    chmod 600 .env
  fi
fi

# Réglages de Traefik, lus sur le conteneur en marche (réseau, résolveur de certificats, point d'entrée HTTPS) : rien à deviner.
if [[ -n "$TRAEFIK_NAME" && $DRY -eq 0 ]] && ! grep -q '^TRAEFIK_NETWORK=' .env; then
  network="$(docker inspect -f '{{range $name, $_ := .NetworkSettings.Networks}}{{$name}} {{end}}' "$TRAEFIK_NAME" | awk '{print $1}')"
  args="$(docker inspect -f '{{join .Args " "}} {{join .Config.Cmd " "}}' "$TRAEFIK_NAME")"
  resolver="$(grep -oE 'certificatesresolvers\.[A-Za-z0-9_-]+' <<<"$args" | head -n1 | cut -d. -f2 || true)"
  entry="$(grep -oE 'entrypoints\.[A-Za-z0-9_-]+\.address=:443' <<<"$args" | head -n1 | cut -d. -f2 || true)"
  [[ -n "$resolver" && -n "$entry" ]] || echo "ATTENTION : réglages Traefik non lus dans la ligne de commande (configuration par fichier ?) : valeurs par défaut (websecure, mytlschallenge). Corrige .env si besoin." >&2
  {
    echo "TRAEFIK_NETWORK=${network:-n8n_default}"
    echo "TRAEFIK_ENTRYPOINT=${entry:-websecure}"
    echo "TRAEFIK_CERT_RESOLVER=${resolver:-mytlschallenge}"
  } >> .env
  say "Traefik : réseau ${network:-n8n_default}, point d'entrée ${entry:-websecure}, résolveur ${resolver:-mytlschallenge}"
fi

# -- 5. Pare-feu (seulement si UFW est déjà actif : on n'en active jamais un, pour ne pas te couper l'accès SSH) ----------------------
if [[ $FIREWALL -eq 1 ]] && command -v ufw >/dev/null 2>&1 && ufw status 2>/dev/null | grep -q "Status: active"; then
  say "Pare-feu UFW : ouverture des ports nécessaires"
  run ufw allow 22/tcp
  run ufw allow 80/tcp
  run ufw allow 443/tcp
  run ufw allow 3478/tcp
  run ufw allow 3478/udp
  run ufw allow 49152:65535/udp
else
  say "Pare-feu : rien modifié ici. Si un pare-feu est actif (UFW, ou celui du panneau Hostinger), ouvre : 80/tcp, 443/tcp, 3478/tcp+udp, 49152-65535/udp."
fi

# -- 6. Démarrage -----------------------------------------------------------------------------------------------------------------
say "Construction et démarrage (la première fois, quelques minutes)"
run docker compose -f "$COMPOSE_FILE" up -d --build

if [[ $DRY -eq 0 ]]; then
  say "Attente du serveur (le certificat HTTPS peut prendre une minute)"
  for _ in $(seq 1 40); do
    if curl -fsS --max-time 6 "https://$DOMAIN/health" >/dev/null 2>&1; then ok=1; break; fi
    sleep 5
  done
  [[ "${ok:-0}" -eq 1 ]] || { echo "Le serveur ne répond pas encore sur https://$DOMAIN/health. Regarde : docker compose -f $COMPOSE_FILE logs --tail 50" >&2; exit 1; }
fi

# -- 7. Récapitulatif -------------------------------------------------------------------------------------------------------------
AGENT_KEY="<RENDEZVOUS_AGENT_KEY>"; CONTROLLER_KEY="<RENDEZVOUS_CONTROLLER_KEY>"
if [[ $DRY -eq 0 ]]; then
  AGENT_KEY="$(grep '^RENDEZVOUS_AGENT_KEY=' .env | cut -d= -f2-)"; CONTROLLER_KEY="$(grep '^RENDEZVOUS_CONTROLLER_KEY=' .env | cut -d= -f2-)"
fi
cat <<EOF

================================================================================
Serveur de rendez-vous prêt : https://$DOMAIN
================================================================================
1) Sur le PC de CONTRÔLE (PC Assistant), dans backend/.env, puis redémarre le backend :
     REMOTE_RENDEZVOUS_URL=https://$DOMAIN
     REMOTE_RENDEZVOUS_KEY=$CONTROLLER_KEY

2) Sur chaque PC À CONTRÔLER, dans la fenêtre de l'agent :
     Adresse : https://$DOMAIN
     Clé     : $AGENT_KEY

Ces clés sont dans $(pwd)/.env (lisible par root seulement). Ne les partage pas.
Mise à jour : git pull && docker compose -f $COMPOSE_FILE up -d --build      Journaux : docker compose -f $COMPOSE_FILE logs -f
EOF
