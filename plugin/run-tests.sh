#!/bin/sh
# Lance les tests du plugin (et du serveur de rendez-vous) hors d'ARIA : reconstitue l'arborescence backend/ que le plugin attend
# (modules d'ARIA copiés dans rendezvous/vendor/backend, plugin dans backend/plugins/remote), dans un dossier temporaire.
# Usage : sh plugin/run-tests.sh [options pytest]   (nécessite : pip install -r rendezvous/requirements.txt pytest)
set -eu
root="$(cd "$(dirname "$0")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "${tmp}"' EXIT
mkdir -p "${tmp}/backend/plugins" "${tmp}/backend/tests" "${tmp}/rendezvous"
cp -r "${root}/rendezvous/vendor/backend/." "${tmp}/backend/"
cp -r "${root}/plugin" "${tmp}/backend/plugins/remote"
mv "${tmp}/backend/plugins/remote/tests/"* "${tmp}/backend/tests/" && rmdir "${tmp}/backend/plugins/remote/tests"
cp "${root}/rendezvous/app.py" "${root}/rendezvous/requirements.txt" "${tmp}/rendezvous/"
cd "${tmp}/backend"
python3 -m pytest -q tests "$@"
