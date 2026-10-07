#!/bin/sh
# Télécharge TOUTES les versions publiées de l'agent (étiquettes agent-v*) depuis les Releases GitHub, vérifie leur empreinte SHA-256, et prépare :
#   versions/<x.y.z>/remote-agent.exe(.sha256)  : chaque version, téléchargeable ;
#   remote-agent.exe, remote-agent.exe.sha256   : la plus récente (le bouton principal de la page) ;
#   version.txt, sha256.txt, size.txt           : ses valeurs, pour remplir la page ;
#   rows.html                                   : la liste de toutes les versions (section « Toutes les versions »).
# Variables : REPO (propriétaire/dépôt) ; GIT_TOKEN facultatif (inutile pour un dépôt public, évite seulement la limite de requêtes anonymes).
set -eu
REPO="${REPO:-patgarcia66240-cmd/aria-remote}"
api="https://api.github.com/repos/${REPO}/releases"
get() { if [ -n "${GIT_TOKEN:-}" ]; then curl -fsSL -H "Authorization: Bearer ${GIT_TOKEN}" "$@"; else curl -fsSL "$@"; fi; }

releases="$(get -H "Accept: application/vnd.github+json" "${api}?per_page=50" \
  | jq -c '[.[] | select(.draft | not) | select(.tag_name | startswith("agent-v"))]')"
count="$(echo "${releases}" | jq 'length')"
[ "${count}" -gt 0 ] || { echo "Aucune version agent-v* publiée : lance git tag agent-v0.1.0 && git push origin agent-v0.1.0"; exit 1; }

mkdir -p versions
: > rows.html
i=0
while [ "${i}" -lt "${count}" ]; do
  rel="$(echo "${releases}" | jq -c ".[${i}]")"
  tag="$(echo "${rel}" | jq -r '.tag_name | ltrimstr("agent-v")')"
  day="$(echo "${rel}" | jq -r '.published_at[0:10]')"
  dir="versions/${tag}"; mkdir -p "${dir}"
  for name in remote-agent.exe remote-agent.exe.sha256; do
    id="$(echo "${rel}" | jq -r --arg n "${name}" '[.assets[] | select(.name == $n)][0].id // empty')"
    [ -n "${id}" ] || { echo "Fichier ${name} absent de la version ${tag}"; exit 1; }
    get -H "Accept: application/octet-stream" -o "${dir}/${name}" "${api}/assets/${id}"
  done
  sum="$(cut -d' ' -f1 "${dir}/remote-agent.exe.sha256")"
  [ "$(sha256sum "${dir}/remote-agent.exe" | cut -d' ' -f1)" = "${sum}" ] || { echo "Empreinte SHA-256 différente pour ${tag} : fichier corrompu"; exit 1; }
  size="$(du -h "${dir}/remote-agent.exe" | cut -f1)"
  badge=""
  if [ "${i}" -eq 0 ]; then
    badge='<span class="tag">Dernière</span>'
    cp "${dir}/remote-agent.exe" remote-agent.exe; cp "${dir}/remote-agent.exe.sha256" remote-agent.exe.sha256
    echo "${tag}" > version.txt; echo "${sum}" > sha256.txt; echo "${size}" > size.txt
  fi
  printf '<li class="ver"><span class="vn"><b>%s</b>%s</span><span class="vd">%s</span><span class="vs">%s</span><code class="vh" title="Empreinte SHA-256">%s</code><a class="vl" href="/versions/%s/remote-agent.exe" download>Télécharger</a></li>\n' \
    "${tag}" "${badge}" "${day}" "${size}" "${sum}" "${tag}" >> rows.html
  i=$((i + 1))
done
echo "${count} version(s) récupérée(s)."
