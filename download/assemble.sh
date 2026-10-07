#!/bin/sh
# Remplit la page (modèle $TEMPLATE) avec les valeurs préparées par fetch.sh dans le dossier courant ; écrit index.html.
set -eu
cp "${TEMPLATE:-/app/index.html}" index.html
sed -i "s/__VERSION__/$(cat version.txt)/g; s/__SHA256__/$(cat sha256.txt)/g; s/__SIZE__/$(cat size.txt)/g" index.html
sed -i -e "/__VERSIONS__/{r rows.html" -e "d}" index.html
sed -i -e "/__DESKTOP__/{r desktop.html" -e "d}" index.html
! grep -q '__[A-Z]*__' index.html || { echo "Valeur non remplacée dans la page" >&2; exit 1; }
