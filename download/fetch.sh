#!/bin/sh
# Récupère les versions publiées (Releases GitHub) et prépare, dans le dossier courant, tout ce que la page de téléchargement affiche :
#   versions/<x.y.z>/remote-agent.exe(.sha256)  : chaque version de l'agent (étiquettes agent-v*), téléchargeable ;
#   remote-agent.exe, remote-agent.exe.sha256   : la plus récente (le bouton principal) ;
#   version.txt, sha256.txt, size.txt           : ses valeurs, pour remplir la page ;
#   rows.html                                   : la liste de toutes les versions ;
#   desktop/aria-remote-desktop.exe(.sha256), desktop.html : la dernière version de l'application de bureau (étiquettes desktop-v*), si elle existe.
#
# Les fichiers déjà téléchargés sont gardés dans $CACHE (une version publiée ne change jamais) : une mise à jour ne télécharge que les NOUVELLES versions
# et ne fait qu'un seul appel à l'API GitHub (limite anonyme : 60 par heure). Chaque exe est vérifié par son empreinte SHA-256.
# Variables : REPO (propriétaire/dépôt), CACHE (dossier de cache), API_BASE (tests) ; GIT_TOKEN facultatif (dépôt privé).
set -eu
REPO="${REPO:-patgarcia66240-cmd/aria-remote}"
API="${API_BASE:-https://api.github.com}/repos/${REPO}"
CACHE="${CACHE:-$PWD/.cache}"
mkdir -p "${CACHE}"

api_get() { if [ -n "${GIT_TOKEN:-}" ]; then curl -fsSL --retry 3 -H "Authorization: Bearer ${GIT_TOKEN}" "$@"; else curl -fsSL --retry 3 "$@"; fi; }
# Les fichiers des releases d'un dépôt public se téléchargent sans compte et sans quota d'API ; aucun jeton n'est envoyé aux redirections.
download() { curl -fsSL --retry 3 -o "$2" "$1"; }

# asset <release-json> <nom> -> adresse de téléchargement
asset_url() { echo "$1" | jq -r --arg n "$2" '[.assets[] | select(.name == $n)][0].browser_download_url // empty'; }

# ensure <genre> <version> <release-json> <fichier.exe> : met l'exe vérifié dans le cache ; affiche son chemin
ensure() {
  kind="$1"; ver="$2"; rel="$3"; exe="$4"
  dir="${CACHE}/${kind}-${ver}"; mkdir -p "${dir}"
  url_exe="$(asset_url "${rel}" "${exe}")"; url_sum="$(asset_url "${rel}" "${exe}.sha256")"
  [ -n "${url_exe}" ] && [ -n "${url_sum}" ] || { echo "Fichier ${exe} ou son empreinte absent de la version ${ver}" >&2; return 1; }
  download "${url_sum}" "${dir}/${exe}.sha256.new"
  sum="$(cut -d' ' -f1 "${dir}/${exe}.sha256.new" | tr 'A-F' 'a-f')"
  echo "${sum}" | grep -Eq '^[0-9a-f]{64}$' || { echo "Empreinte illisible pour ${ver}" >&2; return 1; }
  if ! { [ -f "${dir}/${exe}" ] && [ "$(sha256sum "${dir}/${exe}" | cut -d' ' -f1)" = "${sum}" ]; }; then
    echo "Téléchargement de ${exe} ${ver}..." >&2
    download "${url_exe}" "${dir}/${exe}.part"
    [ "$(sha256sum "${dir}/${exe}.part" | cut -d' ' -f1)" = "${sum}" ] || { rm -f "${dir}/${exe}.part"; echo "Empreinte SHA-256 différente pour ${exe} ${ver} : fichier corrompu" >&2; return 1; }
    mv "${dir}/${exe}.part" "${dir}/${exe}"
  fi
  echo "${sum}  ${exe}" > "${dir}/${exe}.sha256"; rm -f "${dir}/${exe}.sha256.new"
  echo "${dir}/${exe}"
}

# place <fichier du cache> <destination> : lien dur (pas de copie) si possible
place() { mkdir -p "$(dirname "$2")"; ln -f "$1" "$2" 2>/dev/null || cp -f "$1" "$2"; ln -f "$1.sha256" "$2.sha256" 2>/dev/null || cp -f "$1.sha256" "$2.sha256"; }

raw="$(api_get -H "Accept: application/vnd.github+json" "${API}/releases?per_page=50")" || { echo "GitHub injoignable (${API})." >&2; exit 1; }
releases="$(echo "${raw}" | jq -c '[.[] | select(.draft | not)]')" || { echo "Réponse de GitHub illisible." >&2; exit 1; }
agents="$(echo "${releases}" | jq -c '[.[] | select(.tag_name | startswith("agent-v"))]')"
count="$(echo "${agents}" | jq 'length')"
[ "${count}" -gt 0 ] || { echo "Aucune version agent-v* publiée dans ${REPO}." >&2; exit 1; }

mkdir -p versions
: > rows.html
i=0
while [ "${i}" -lt "${count}" ]; do
  rel="$(echo "${agents}" | jq -c ".[${i}]")"
  ver="$(echo "${rel}" | jq -r '.tag_name | ltrimstr("agent-v")')"
  day="$(echo "${rel}" | jq -r '.published_at[0:10]')"
  cached="$(ensure agent "${ver}" "${rel}" remote-agent.exe)"
  place "${cached}" "versions/${ver}/remote-agent.exe"
  sum="$(cut -d' ' -f1 "${cached}.sha256")"
  size="$(du -h "${cached}" | cut -f1)"
  badge=""
  if [ "${i}" -eq 0 ]; then
    badge='<span class="tag">Dernière</span>'
    place "${cached}" remote-agent.exe
    echo "${ver}" > version.txt; echo "${sum}" > sha256.txt; echo "${size}" > size.txt
  fi
  printf '<li class="ver"><span class="vn"><b>%s</b>%s</span><span class="vd">%s</span><span class="vs">%s</span><code class="vh" title="Empreinte SHA-256">%s</code><a class="vl" href="/versions/%s/remote-agent.exe" download>Télécharger</a></li>\n' \
    "${ver}" "${badge}" "${day}" "${size}" "${sum}" "${ver}" >> rows.html
  i=$((i + 1))
done

# -- Application de bureau (étiquettes desktop-v*) : facultative, seule la dernière version est proposée.
mkdir -p desktop
latest="$(echo "${releases}" | jq -c '[.[] | select(.tag_name | startswith("desktop-v"))][0] // empty')"
if [ -n "${latest}" ]; then
  dver="$(echo "${latest}" | jq -r '.tag_name | ltrimstr("desktop-v")')"
  dcached="$(ensure desktop "${dver}" "${latest}" aria-remote-desktop.exe)"
  place "${dcached}" desktop/aria-remote-desktop.exe
  dsum="$(cut -d' ' -f1 "${dcached}.sha256")"; dsize="$(du -h "${dcached}" | cut -f1)"
  printf '<a class="cta small" href="/desktop/aria-remote-desktop.exe" download><svg class="i"><use href="#i-download"/></svg>Télécharger pour Windows</a>\n<p class="meta">Version %s · %s · aucune installation</p>\n<code class="hash" title="Empreinte SHA-256">%s</code>\n' "${dver}" "${dsize}" "${dsum}" > desktop.html
  echo "Application de bureau ${dver}."
else
  printf '<span class="soon"><svg class="i"><use href="#i-play"/></svg>Bientôt disponible</span>\n<p class="meta">Elle est en cours de développement.</p>\n' > desktop.html
  echo "Pas de version desktop publiée : « Bientôt disponible »."
fi

# -- Nettoyage du cache : on ne garde que les versions encore publiées.
for d in "${CACHE}"/agent-* "${CACHE}"/desktop-*; do
  [ -d "${d}" ] || continue
  keep=no
  case "${d##*/}" in
    agent-*) echo "${agents}" | jq -e --arg v "${d##*/agent-}" 'any(.[]; .tag_name == "agent-v" + $v)' >/dev/null && keep=yes ;;
    desktop-*) echo "${releases}" | jq -e --arg v "${d##*/desktop-}" 'any(.[]; .tag_name == "desktop-v" + $v)' >/dev/null && keep=yes ;;
  esac
  [ "${keep}" = yes ] || rm -rf "${d}"
done
echo "${count} version(s) de l'agent."
