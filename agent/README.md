# Remote Agent — PC Assistant

Agent à installer sur le **PC à contrôler** (voir `docs/REMOTE_ARCHITECTURE.md`). Il tourne dans une console visible, jamais caché, et ne fait
rien sans l'accord de la personne devant l'appareil.

```text
Navigateur (PC Assistant)  ──signaling──►  Backend FastAPI  ◄──WebSocket──  remote-agent
         ▲                                                                       │
         └──────────────── WebRTC (écran en JPEG + commandes, chiffré) ───────────┘
```

## Accès par Internet

Pour contrôler un PC situé ailleurs (autre réseau, autre ville), déploie le **serveur de rendez-vous** (`remote-rendezvous/README.md`) puis, dans la
fenêtre de l'agent, saisis son adresse (`https://ton-domaine`) et la clé des agents. L'agent s'y connecte de lui-même : aucun port à ouvrir sur ce PC.
Sur un même réseau, l'adresse du backend de PC Assistant (`192.168.1.20:8000`) suffit.

## Lancer en une commande

```powershell
.\remote-agent\start-agent.ps1          # Windows (PowerShell), depuis n'importe quel dossier
./remote-agent/start-agent.sh           # Linux / macOS
```

Le script se place dans le bon dossier, lit la clé d'API (`API_AUTH_TOKEN`) dans `backend/.env`, vérifie que Rust est installé et autorise la
souris et le clavier. Options PowerShell : `-Server`, `-ApiKey`, `-Allow ""` (regarder seulement), `-Name`, `-Dev` (compilation plus rapide).
Si PowerShell refuse d'exécuter le script : `powershell -ExecutionPolicy Bypass -File .\remote-agent\start-agent.ps1`.

## Lancer à la main

```bash
cd remote-agent
cargo run --release -- --server http://IP_DU_PC_ASSISTANT:8000 --api-key <API_AUTH_TOKEN> --allow mouse,keyboard
```

1. Au premier lancement l'agent crée son identité (clé Ed25519) et affiche un **code d'appairage à 6 chiffres**.
2. Dans PC Assistant : **Ordinateur › Maintenance à distance**, saisis le code, puis « Se connecter ».
3. Chaque session ouvre une fenêtre (Windows) ou une question (console) : **Autoriser ?** Sans réponse, rien ne s'ouvre.
4. `stop` dans la console coupe la session en cours, `quit` arrête l'agent.

| Option | Rôle |
| --- | --- |
| `--server`, `--api-key` | adresse du backend et sa clé d'API (aussi `REMOTE_SERVER`, `REMOTE_API_KEY`) |
| `--allow mouse,keyboard` | permissions accordées en plus de la vue ; **par défaut : regarder seulement** |
| `--name` | nom affiché dans PC Assistant (défaut : nom de l'ordinateur) |
| `--auto-accept` | accepte sans demander — **développement uniquement** |
| `--config` | fichier d'identité (défaut : dossier de configuration de l'utilisateur) |

Le code d'appairage vaut 30 minutes par défaut ; réglable côté backend par `REMOTE_PAIRING_CODE_TTL` (secondes, entre 60 et 86400) dans `backend/.env`.

## Compiler

- **Windows** (capture écran `xcap`, saisie `enigo`, fenêtre de consentement `rfd`) : `cargo build --release`.
- **Linux / macOS** : compile et passe les tests, mais l'écran est une image de test animée et la saisie est seulement journalisée
  (`[saisie simulée]`) : utile pour développer, pas pour contrôler un vrai poste.
- Vérifié ici : `cargo test` (Linux) et `cargo check --target x86_64-pc-windows-gnu`. Le contrôle d'un vrai bureau Windows n'a **pas** été
  essayé : à valider sur un PC.

## Sécurité

- La **clé privée ne quitte jamais l'appareil** (fichier de configuration en 0600 sous Unix). Le serveur ne connaît que la clé publique.
- L'enregistrement est **signé** (preuve de possession, horodatage + nonce : non rejouable). La réponse WebRTC est signée **sur l'offre reçue**,
  empreinte DTLS comprise : même avec la clé d'API, on ne peut pas se faire passer pour l'agent. À la connexion, l'agent signe un défi du serveur.
- Les permissions sont **vérifiées par l'agent à chaque commande** (`src/input/mod.rs`), indépendamment du serveur et du contrôleur.
- Pas de contournement de l'UAC ni de Ctrl+Alt+Suppr ; une session s'arrête à `stop`, `quit`, ou quand le contrôleur se déconnecte.
- Limites connues : pas d'audio, un seul écran (le principal), pas de presse-papiers ni de fichiers (V2 du doc), mise à l'échelle DPI de Windows
  non traitée (coordonnées normalisées sur l'écran principal).

## Protocole WebSocket (`/api/remote/agent/ws?device_id=…`, en-tête `x-api-key`)

| Sens | Message |
| --- | --- |
| serveur → agent | `challenge{nonce}` · `ready{ice_servers}` · `open_session{session_id, permissions, ice_servers}` · `offer{session_id, sdp}` · `ice{session_id, candidate}` · `reconnect{session_id}` · `close_session{session_id}` · `error{detail}` |
| agent → serveur | `auth{signature}` · `grant{permissions}` · `session_reply{session_id, accepted, reason}` · `answer{session_id, sdp, signature}` · `ice{session_id, candidate}` · `resume{session_id}` · `bye{session_id}` · `ping` |

Canaux de données WebRTC ouverts par le navigateur : `frames` (agent → navigateur, images JPEG découpées en morceaux de 16 Ko avec un en-tête de
8 octets : numéro d'image u32, rang u16, nombre u16) et `control` (JSON : `info`, `move`, `down`, `up`, `wheel`, `key`, `denied`).

## STUN / TURN (via Internet)

Sur le même réseau, rien à régler. Au-delà d'un routeur, renseigner dans `backend/.env` : `REMOTE_STUN_URLS`, et si la connexion directe échoue
`REMOTE_TURN_URLS` avec `REMOTE_TURN_SECRET` (coturn `use-auth-secret` : identifiants éphémères par session). Exemple de `turnserver.conf` :

```text
listening-port=3478
fingerprint
use-auth-secret
static-auth-secret=<le même que REMOTE_TURN_SECRET>
realm=pc-assistant
no-cli
```

## Tests

```bash
cargo test    # identité, configuration, permissions, mapping clavier/souris, JPEG, découpage des images, messages du protocole
```
