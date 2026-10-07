#!/bin/sh
# Sert la page de téléchargement ET la tient à jour : toutes les REFRESH_SECONDS (10 min par défaut), il regarde les Releases GitHub et, s'il y a du
# nouveau, reconstruit la page dans un dossier à part puis l'échange d'un coup (aucune coupure). En cas d'échec (GitHub injoignable...), la page
# précédente reste en ligne.
set -u
SRV="${SRV:-/srv}"
export CACHE="${SRV}/cache"
INTERVAL="${REFRESH_SECONDS:-600}"
mkdir -p "${CACHE}"

refresh() {
  new="$(mktemp -d "${SRV}/build-XXXXXX")" || return 1
  if ( cd "${new}" && sh /app/fetch.sh && TEMPLATE=/app/index.html sh /app/assemble.sh ); then
    chmod -R a+rX "${new}"
    # Échange atomique du lien ; repli (sans -T, selon le mv disponible) : remplacement direct, quelques microsecondes d'écart.
    { ln -sfn "${new}" "${SRV}/next" && mv -T "${SRV}/next" "${SRV}/current" 2>/dev/null; } || ln -sfn "${new}" "${SRV}/current"
    for old in "${SRV}"/build-*; do [ "${old}" = "${new}" ] || rm -rf "${old}"; done
    echo "[refresh] page à jour : agent $(cat "${new}/version.txt")"
  else
    rm -rf "${new}"
    echo "[refresh] échec : la page précédente reste en ligne"
    return 1
  fi
}

placeholder() {
  d="${SRV}/build-placeholder"; mkdir -p "${d}"
  cat > "${d}/index.html" <<'H'
<!doctype html><html lang="fr"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>ARIA Remote</title>
<style>body{margin:0;min-height:100vh;display:grid;place-items:center;background:#0a0f18;color:#e8ecf4;font:18px/1.5 "Segoe UI",system-ui,sans-serif;text-align:center;padding:24px}p{color:#96a1b6}</style></head>
<body><div><h1>ARIA Remote</h1><p>Le téléchargement sera disponible dans un instant.</p></div></body></html>
H
  chmod -R a+rX "${d}"; ln -sfn "${d}" "${SRV}/current"
}

# Page de secours seulement s'il n'y a rien du tout à servir (premier démarrage sans GitHub) : jamais par-dessus une page valide.
if ! refresh && [ ! -f "${SRV}/current/index.html" ]; then placeholder; fi
(
  while true; do
    if [ -e "${SRV}/current" ] && [ "$(readlink "${SRV}/current")" != "${SRV}/build-placeholder" ]; then sleep "${INTERVAL}"; else sleep 60; fi
    refresh
  done
) &
exec nginx -g 'daemon off;'
