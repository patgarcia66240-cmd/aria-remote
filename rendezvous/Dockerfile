# Serveur de rendez-vous : construit depuis la RACINE du dépôt (docker compose le fait : voir docker-compose.yml).
FROM python:3.12-slim

WORKDIR /app
COPY rendezvous/requirements.txt /tmp/requirements.txt
RUN pip install --no-cache-dir -r /tmp/requirements.txt

# Uniquement ce dont le plugin Remote a besoin : modules d'ARIA copiés dans rendezvous/vendor/ (voir son README), puis le plugin lui-même.
COPY rendezvous/vendor/backend /app/backend
COPY plugin /app/backend/plugins/remote
COPY rendezvous/app.py /app/app.py

# Utilisateur non privilégié ; /app/backend/data (appareils appairés, journal) est un volume.
RUN useradd --create-home --uid 10002 rendezvous && mkdir -p /app/backend/data && chown -R rendezvous:rendezvous /app
USER rendezvous
VOLUME ["/app/backend/data"]

# Port : 8080 par défaut ; certains hébergeurs imposent le leur dans la variable PORT.
ENV PORT=8080
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s \
  CMD python -c "import os, urllib.request; urllib.request.urlopen('http://127.0.0.1:%s/health' % os.environ.get('PORT', '8080'), timeout=4)" || exit 1

# --proxy-headers : derrière Caddy ou l'hébergeur, l'adresse du client vient de X-Forwarded-For (limite d'essais d'appairage par client, pas par proxy).
CMD ["sh", "-c", "exec uvicorn app:app --host 0.0.0.0 --port ${PORT} --proxy-headers --forwarded-allow-ips '*'"]
