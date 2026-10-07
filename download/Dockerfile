# Petite page de téléchargement de l'agent Remote (remote-agent.exe), servie par nginx.
# L'exe n'est PAS dans le dépôt : il est compilé par GitHub Actions (.github/workflows/build-agent.yml) et attaché à une version « agent-vX.Y.Z ».
# À la construction de l'image, on télécharge la dernière version avec un jeton GitHub en LECTURE SEULE ; ce jeton reste dans l'étape intermédiaire
# et n'est jamais dans l'image finale.
FROM alpine:3.20 AS fetch
ARG GIT_TOKEN
ARG REPO=patgarcia66240-cmd/pc-assistant
RUN apk add --no-cache curl jq
WORKDIR /dl
RUN set -eu; \
    [ -n "${GIT_TOKEN}" ] || { echo "GIT_TOKEN manquant (jeton GitHub en lecture seule)"; exit 1; }; \
    api="https://api.github.com/repos/${REPO}/releases"; \
    release="$(curl -fsS -H "Authorization: Bearer ${GIT_TOKEN}" -H "Accept: application/vnd.github+json" "${api}" \
      | jq -c '[.[] | select(.tag_name | startswith("agent-v"))][0]')"; \
    [ "${release}" != "null" ] || { echo "Aucune version agent-v* publiée : lance git tag agent-v0.1.0 && git push origin agent-v0.1.0"; exit 1; }; \
    for name in remote-agent.exe remote-agent.exe.sha256; do \
      id="$(echo "${release}" | jq -r --arg n "${name}" '.assets[] | select(.name == $n) | .id')"; \
      [ -n "${id}" ] || { echo "Fichier ${name} absent de la version"; exit 1; }; \
      curl -fsSL -H "Authorization: Bearer ${GIT_TOKEN}" -H "Accept: application/octet-stream" -o "${name}" "${api}/assets/${id}"; \
    done; \
    echo "${release}" | jq -r '.tag_name | ltrimstr("agent-v")' > version.txt; \
    cut -d' ' -f1 remote-agent.exe.sha256 > sha256.txt; \
    [ "$(sha256sum remote-agent.exe | cut -d' ' -f1)" = "$(cat sha256.txt)" ] || { echo "Empreinte SHA-256 différente : fichier corrompu"; exit 1; }
COPY index.html /dl/index.html
RUN sed -i "s/__VERSION__/$(cat version.txt)/g; s/__SHA256__/$(cat sha256.txt)/g; s/__SIZE__/$(du -h remote-agent.exe | cut -f1)/g" index.html

FROM nginx:1.27-alpine
COPY nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=fetch /dl/index.html /dl/remote-agent.exe /dl/remote-agent.exe.sha256 /usr/share/nginx/html/
EXPOSE 80
