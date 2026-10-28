# Page d'accueil de xendev.eu : un fichier HTML statique servi par nginx (aucune dépendance, aucun cookie, aucune requête externe).
FROM nginx:1.27-alpine
COPY nginx.conf /etc/nginx/conf.d/default.conf
COPY index.html /usr/share/nginx/html/index.html
COPY img /usr/share/nginx/html/img
HEALTHCHECK --interval=30s --timeout=3s CMD wget -q -O /dev/null http://127.0.0.1/ || exit 1
