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
