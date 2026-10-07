# Page de téléchargement (processxen.com)

Une page nginx qui propose la dernière version de l'**agent** et de l'**application de bureau**, avec toutes les anciennes versions et leur empreinte SHA-256.

## Mise à jour automatique
Le conteneur regarde les Releases GitHub toutes les 10 minutes (`REFRESH_SECONDS`). Quand une version est publiée (étiquette `agent-vX.Y.Z` ou `desktop-vX.Y.Z`), il télécharge le nouvel exe, **vérifie son empreinte SHA-256**, reconstruit la page à part, puis l'échange d'un coup : la page est en ligne sans coupure, **sans reconstruire l'image ni se connecter au serveur**.

- Un exe déjà vu n'est jamais retéléchargé (cache), et chaque mise à jour ne fait qu'un appel à l'API GitHub : la limite anonyme (60 par heure) n'est pas un souci.
- Si GitHub est injoignable ou qu'un fichier est corrompu, la page précédente reste en ligne.
- Dépôt public : aucun jeton. Au tout premier démarrage sans aucune version publiée, une page d'attente s'affiche.
- Les journaux (`docker logs`) indiquent chaque mise à jour : `[refresh] page à jour : agent 1.5.0`.

Sur ton PC, l'ordre est donc : pousser l'étiquette → attendre la fin de la compilation → la page se met à jour dans les 10 minutes.

## Déployer (une seule fois)
`docker-compose.yml` convient à un serveur qui a déjà Traefik. L'image n'est reconstruite que si les fichiers de ce dossier changent (`fetch.sh`, `index.html`, `nginx.conf`...), pas à chaque version de l'agent.

Fichiers : `fetch.sh` (télécharge et vérifie les versions), `assemble.sh` (remplit la page), `entrypoint.sh` (met à jour en boucle puis sert), `index.html` (modèle de la page), `nginx.conf`.
