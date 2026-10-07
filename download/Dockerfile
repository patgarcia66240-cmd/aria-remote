# Petite page de téléchargement de l'agent Remote (remote-agent.exe), servie par nginx.
# L'exe n'est PAS dans le dépôt : il est compilé par GitHub Actions (.github/workflows/build-agent.yml) et attaché à une version « agent-vX.Y.Z ».
# À la construction de l'image, on télécharge TOUTES les versions (la plus récente en bouton principal, les autres listées en bas de page) avec un jeton GitHub en LECTURE SEULE ; ce jeton reste dans l'étape intermédiaire
# et n'est jamais dans l'image finale.
FROM alpine:3.20 AS fetch
ARG GIT_TOKEN
ARG REPO=patgarcia66240-cmd/aria-remote
RUN apk add --no-cache curl jq
WORKDIR /dl
COPY fetch.sh /dl/fetch.sh
RUN sh /dl/fetch.sh
COPY index.html /dl/index.html
RUN sed -i "s/__VERSION__/$(cat version.txt)/g; s/__SHA256__/$(cat sha256.txt)/g; s/__SIZE__/$(cat size.txt)/g" index.html \
 && sed -i -e "/__VERSIONS__/{r rows.html" -e "d}" index.html

FROM nginx:1.27-alpine
COPY nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=fetch /dl/index.html /dl/remote-agent.exe /dl/remote-agent.exe.sha256 /usr/share/nginx/html/
COPY --from=fetch /dl/versions /usr/share/nginx/html/versions
EXPOSE 80
