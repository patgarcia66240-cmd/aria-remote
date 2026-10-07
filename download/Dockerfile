# Page de téléchargement d'ARIA Remote : nginx + un petit script qui la tient à jour tout seul.
# Rien n'est compilé ici : les exe viennent des Releases GitHub (étiquettes agent-vX.Y.Z et desktop-vX.Y.Z), publiées par .github/workflows/.
# Le conteneur regarde GitHub toutes les REFRESH_SECONDS (600 par défaut) et met la page à jour dès qu'une version sort : plus de reconstruction à la main.
# Dépôt public : aucun jeton n'est nécessaire.
FROM nginx:1.27-alpine
RUN apk add --no-cache curl jq
ENV REPO=patgarcia66240-cmd/aria-remote REFRESH_SECONDS=600
WORKDIR /app
COPY fetch.sh assemble.sh entrypoint.sh index.html /app/
COPY nginx.conf /etc/nginx/conf.d/default.conf
VOLUME ["/srv/cache"]
EXPOSE 80
HEALTHCHECK --interval=30s --timeout=5s --start-period=40s CMD wget -qO- http://127.0.0.1/health >/dev/null || exit 1
ENTRYPOINT ["sh", "/app/entrypoint.sh"]
