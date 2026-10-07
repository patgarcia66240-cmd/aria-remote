# Serveur de rendez-vous — PC Assistant Remote par Internet

Ce petit service public permet de contrôler un PC **par Internet** avec son ID et son code, comme TeamViewer : aucun port à ouvrir sur le PC
contrôlé, aucune adresse IP à connaître.

```text
PC contrôlé (agent)  ──connexion sortante──►  Serveur de rendez-vous  ◄──────  PC Assistant (contrôleur)
                                              (ce dossier, public)
        ◄────────── écran + commandes en direct (WebRTC chiffré, relais coturn si besoin) ──────────►
```

- **L'agent** garde une connexion sortante vers ce serveur et s'y enregistre avec son identifiant ; le **code à 6 chiffres** (30 minutes, une seule
  utilisation) lie cet appareil à ton PC Assistant.
- **Le serveur** retrouve l'appareil et relaie seulement la négociation. Il ne voit jamais l'écran ni les commandes.
- **coturn** (relais) n'intervient que si la connexion directe est impossible ; le contenu reste chiffré de bout en bout (DTLS).
- Le serveur est **isolé de PC Assistant** : il n'embarque que le plugin Remote. Ni tes fichiers, ni ton système, ni ton agenda ne sont exposés.

## Ce qu'il faut

- Un petit serveur Linux avec une **IP publique** et Docker (un VPS à quelques euros par mois suffit), ou un PC chez toi avec les ports redirigés.
- Un **nom de domaine** dont le DNS pointe vers ce serveur (nécessaire pour le HTTPS).
- Ports à ouvrir sur le serveur : **80 et 443** (TCP), **3478** (UDP et TCP) et **49152–65535** (UDP, relais coturn).

## Installation

```bash
cd remote-rendezvous
cp .env.example .env
# Génère trois secrets différents :   openssl rand -hex 32
# Remplis .env : DOMAIN, RENDEZVOUS_AGENT_KEY, RENDEZVOUS_CONTROLLER_KEY, TURN_SECRET
docker compose up -d --build
curl https://TON_DOMAINE/health        # {"status":"ok"}
```

Si le serveur est derrière un NAT (IP publique différente de celle de la machine), ajoute `--external-ip=<IP publique>` à la commande de `coturn`
dans `docker-compose.yml`.

## Sur un VPS Hostinger (avec n8n déjà installé)

Le modèle « Ubuntu 24.04 with n8n » utilise **Traefik sur les ports 80 et 443** : un second serveur HTTPS entrerait en conflit. `install-vps.sh`
**détecte Traefik**, lit ses réglages réels (réseau, résolveur de certificats, point d'entrée) et branche le serveur de rendez-vous dessus avec
`docker-compose.traefik.yml`. Rien n'est modifié dans le projet n8n : le serveur de rendez-vous est un projet Docker séparé.

1. **DNS** : crée un enregistrement **A** (par exemple `rendezvous.mondomaine.fr`) vers l'IP du VPS. Sans nom de domaine, `--sslip` utilise
   `<ip-avec-des-tirets>.sslip.io` ; son certificat est valide, mais la limite de certificats Let's Encrypt est partagée entre tous les utilisateurs de ce
   service.
2. **Terminal** : ouvre le terminal du VPS (terminal du navigateur dans hPanel, ou SSH).
3. **Récupérer le code** : le dépôt est privé. Crée sur GitHub un jeton à accès fin, **lecture seule** du contenu de ce dépôt, puis :
   ```bash
   git clone -b chore/frontend-audit-p1 https://<JETON>@github.com/patgarcia66240-cmd/pc-assistant.git
   cd pc-assistant/remote-rendezvous
   ```
   Révoque le jeton ensuite (ou supprime-le) : il n'est plus utile, sauf pour les mises à jour.
4. **Installer** : `sudo ./install-vps.sh --domain rendezvous.mondomaine.fr` (ou `--sslip`). Teste d'abord avec `--dry-run` : il montre tout ce qui serait fait sans rien exécuter.
5. Le script affiche à la fin les deux lignes à copier dans `backend/.env` (contrôleur) et la fenêtre de l'agent.

Ports : `3478` (TCP et UDP) et `49152–65535` (UDP) servent au relais. Ils doivent être joignables depuis Internet. Si tu actives un jour le pare-feu
Hostinger, ouvre-les, avec 80 et 443 (déjà utilisés par Traefik) et 22 (SSH).

Contrairement à la variante Render gratuite, les appareils appairés **sont conservés** (volume Docker) et le service ne s'endort pas.

## Variante sans serveur à louer : Render

Le dépôt contient un `render.yaml` à la racine : sur [Render](https://render.com), **New › Blueprint**, choisis ce dépôt (et la branche indiquée dans le
fichier), puis valide. Render construit l'image, fournit le **HTTPS** (`https://pc-assistant-rendezvous.onrender.com` ou similaire) et **génère deux
clés différentes**. Lis-les dans le service, onglet **Environment** : `RENDEZVOUS_AGENT_KEY` (pour les agents) et `RENDEZVOUS_CONTROLLER_KEY` (pour
`backend/.env`, voir ci-dessous). Vérifie : `https://<ton-service>.onrender.com/health` répond `{"status":"ok"}`.

Ce que Render ne fait pas, et ce que ça change :

| Limite | Conséquence | Que faire |
| --- | --- | --- |
| Pas de ports UDP, donc **pas de coturn** | Les connexions qui exigent un relais (certains réseaux d'entreprise, 4G stricte) échouent ; la plupart des réseaux domestiques passent grâce au STUN | Renseigner un service TURN externe : `REMOTE_TURN_URLS`, `REMOTE_TURN_USERNAME`, `REMOTE_TURN_CREDENTIAL` (variables du service) |
| Offre **gratuite** : le service s'endort sans trafic | La première requête après un temps d'inactivité attend le réveil (plusieurs dizaines de secondes) ; les agents se reconnectent seuls | Offre payante « Starter » pour un service permanent |
| Offre gratuite : **disque effacé** à chaque redémarrage | Les appareils appairés sont oubliés : l'agent affiche un nouveau code et il faut le ressaisir | Offre payante avec disque persistant monté sur `/app/backend/data` |

**Vercel ne convient pas** : ses fonctions sont sans état et de courte durée, alors que ce serveur garde des connexions WebSocket ouvertes (les agents) et
un état en mémoire (codes, sessions).

## Vérifier un déploiement (Render, VPS…)

```bash
python remote-rendezvous/check_server.py https://pc-assistant-rendezvous.onrender.com
```

L'outil lit les clés dans `RENDEZVOUS_AGENT_KEY` et `RENDEZVOUS_CONTROLLER_KEY` (ou les demande, sans les afficher) et vérifie : réponse de `/health` (il
attend jusqu'à deux minutes le réveil d'une offre gratuite), HTTPS, absence de documentation publique, **refus** sans clé et avec une mauvaise clé,
**séparation des rôles** (la clé des agents ne liste rien, celle du contrôleur est refusée sur le canal des agents), ouverture du WebSocket des agents, et
serveurs STUN/TURN fournis. Il ne modifie rien. Python suffit, sans dépendance ; à relancer à chaque changement de serveur.

**Essai de bout en bout** (dans cet ordre) :
1. `check_server.py` répond « Tout est conforme » (seul le TURN peut rester « non configuré » sur Render).
2. Sur le PC de contrôle : `backend/.env` avec `REMOTE_RENDEZVOUS_URL` et `REMOTE_RENDEZVOUS_KEY`, redémarrer le backend ; l'onglet Maintenance à distance
   affiche « Accès par Internet via le serveur de rendez-vous ».
3. Sur l'autre PC : l'agent, adresse du serveur + clé des agents ; un code à 6 chiffres s'affiche dans sa fenêtre.
4. Saisir le code dans l'onglet (« Appairer »), puis « Se connecter » ; accepter dans la fenêtre de l'agent.
5. Idéalement, **avec deux réseaux différents** (par exemple l'agent sur un partage de connexion 4G) : c'est ce qui prouve que l'accès par Internet marche.

## Brancher PC Assistant (le contrôleur)

Dans `backend/.env` du PC qui contrôle, puis **redémarre le backend** :

```text
REMOTE_RENDEZVOUS_URL=https://TON_DOMAINE
REMOTE_RENDEZVOUS_KEY=<RENDEZVOUS_CONTROLLER_KEY>
```

Le plugin relaie alors vers ce serveur ; l'onglet **Ordinateur › Maintenance à distance** ne change pas. La clé du serveur reste dans le backend : le
navigateur ne la voit jamais.

## Brancher un PC contrôlé (l'agent)

Lance l'agent (`remote-agent/`) ; au premier lancement, sa fenêtre demande :

- **Adresse du serveur** : `https://TON_DOMAINE`
- **Clé** : `RENDEZVOUS_AGENT_KEY`

Elles sont retenues. L'agent affiche ensuite un code à 6 chiffres : saisis-le dans PC Assistant (Maintenance à distance › Code d'appairage).

## Sécurité : ce qui protège quoi

| Menace | Protection |
| --- | --- |
| Quelqu'un trouve l'adresse du serveur | HTTPS ; toutes les routes exigent une clé ; pas de documentation publique ; `/health` ne dit rien |
| Une clé d'agent fuit (PC contrôlé compromis) | La clé d'agent ne sert qu'à enregistrer un appareil et ouvrir le canal d'agent : pas de liste, pas d'appairage, pas de session |
| Quelqu'un devine un code à 6 chiffres | 5 essais ratés = 1 minute de blocage, limite d'essais par client, code valable 30 minutes et à usage unique |
| Quelqu'un se fait passer pour un appareil | Chaque appareil prouve sa clé privée (signature) à l'enregistrement, à la connexion et pour chaque réponse de négociation |
| Prise de contrôle à l'insu de l'utilisateur | Chaque session exige l'accord de la personne devant l'appareil, qui choisit aussi les permissions (souris, clavier) |
| Le serveur est compromis | Il ne voit ni l'écran ni les commandes (WebRTC chiffré) ; il ne peut pas signer à la place d'un appareil |

**Limites assumées** : un serveur = un propriétaire (tous les contrôleurs ayant la clé voient tous les appareils) ; la négociation (SDP) transite en
clair **pour le serveur** — il peut la lire, pas la falsifier sans que la signature de l'appareil soit refusée. Étape suivante prévue : chiffrer la
négociation de bout en bout avec un échange à mot de passe court (PAKE). Un code de 6 chiffres seul ne suffirait pas à protéger une adresse : il n'a
que un million de valeurs.

## Maintenance

- Mettre à jour : `git pull && docker compose up -d --build`.
- Changer une clé : modifier `.env`, `docker compose up -d`, et mettre à jour `backend/.env` (contrôleur) ou la fenêtre de l'agent.
- Les appareils appairés et le journal sont dans le volume `rendezvous-data`.
