# Remote Agent — PC Assistant

Agent à installer sur le **PC à contrôler** (voir `docs/REMOTE_ARCHITECTURE.md`). Il tourne dans une console visible, jamais caché, et ne fait
rien sans l'accord de la personne devant l'appareil.

```text
Navigateur (PC Assistant)  ──signaling──►  Backend FastAPI  ◄──WebSocket──  remote-agent
         ▲                                                                       │
         └──────────────── WebRTC (écran en JPEG + commandes, chiffré) ───────────┘
```

## Accès par Internet

Pour contrôler un PC situé ailleurs (autre réseau, autre ville), déploie le **serveur de rendez-vous** (`rendezvous/README.md`) puis, dans la
fenêtre de l'agent, saisis son adresse (`https://ton-domaine`) et la clé des agents. L'agent s'y connecte de lui-même : aucun port à ouvrir sur ce PC.
Sur un même réseau, l'adresse du backend de PC Assistant (`192.168.1.20:8000`) suffit.

## Installer en une commande (Windows)
Sur le PC à contrôler, dans PowerShell, sans compiler ni copier de fichier :
```powershell
irm https://raw.githubusercontent.com/patgarcia66240-cmd/aria-remote/main/agent/install-agent.ps1 | iex
```
Le script télécharge la dernière version, **vérifie son empreinte SHA-256**, l'installe dans `%LOCALAPPDATA%\ARIA Remote` (sans droits administrateur), crée un raccourci dans le menu Démarrer et lance l'agent. Rien ne tourne en arrière-plan.

Avec des options (télécharge d'abord le script, ou utilise la forme ci-dessous) :
```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/patgarcia66240-cmd/aria-remote/main/agent/install-agent.ps1))) -Desktop -Autostart
```
| Option | Effet |
|---|---|
| `-Version 1.5.0` | installe une version précise (sinon la dernière) |
| `-Desktop` / `-Autostart` | raccourci sur le Bureau / ouverture de l'agent à l'ouverture de session |
| `-Server` `-ApiKey` `-Name` `-Allow` | pré-remplissent l'agent (retenus ensuite) ; la clé peut aussi être saisie dans la fenêtre |
| `-NoLaunch` | installe sans lancer |
| `-Check` | indique seulement la dernière version |
| `-Uninstall` (`-Purge`) | retire le programme et les raccourcis (`-Purge` : aussi l'identité de l'appareil, qu'il faudra réappairer) |

Réexécuter le script **met à jour** l'agent. Par prudence, tu peux lire le script avant de l'exécuter : il tient en un fichier, `agent/install-agent.ps1`.

## Lancer en une commande

```powershell
.\remote-agent\start-agent.ps1          # Windows (PowerShell), depuis n'importe quel dossier
./agent/start-agent.sh           # Linux / macOS
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

## Canal « control » d'une session
JSON dans les deux sens (le canal « frames » porte les images JPEG). L'agent applique les permissions de la session à chaque message.
| Sens | Message | Permission |
|---|---|---|
| contrôleur → agent | `move` `down` `up` `wheel` (souris) · `key` (clavier) | souris / clavier |
| contrôleur → agent | `ping{id}` → réponse `pong{id}` (latence) | aucune |
| contrôleur → agent | `stream{fps?, quality?, width?}` (images par seconde 2–30, qualité JPEG 20–90, largeur 640–3840) | voir l'écran |
| contrôleur → agent | `screen{index}` : écran à afficher et à piloter (la souris suit) | voir l'écran |
| contrôleur → agent | `clip{text}` : texte à mettre dans le presse-papiers (256 Ko max) | clavier |
| agent → contrôleur | `info{width, height, permissions, screens[], screen, features[]}` : à l'ouverture et à chaque changement d'écran | |
| agent → contrôleur | `cursor{x, y}` (position 0..1 sur l'écran affiché) · `clip{text}` (texte copié sur l'appareil, permission clavier) · `denied{reason}` | |

`features` (`ping`, `stream`, `screens`, `clipboard`) dit au contrôleur ce que cet agent sait faire ; `clipboard` n'y figure que si la session a la permission clavier.

## Latence : ce qui a été fait, et pourquoi
Sur Internet, la latence vient de trois endroits : le **traitement** de l'image avant l'envoi, les **files d'attente** (une image qui attend est une image périmée) et le **transport** (une perte de paquet qui bloque tout ce qui suit).

| Avant | Maintenant |
|---|---|
| Mise à l'échelle + JPEG : **~122 ms** par image en 1600 px (moins de 8 images/s sur un coeur, avant même l'envoi) | **~18 ms** (mise à l'échelle SIMD, encodeur JPEG vectoriel, chrominance 4:2:0, pas de conversion intermédiaire) |
| Capture, encodage et envoi l'un après l'autre | **Trois étapes en parallèle** reliées par des boîtes « dernière valeur » (`pipeline.rs`) : aucune file, la suivante prend toujours l'image la plus récente |
| Jusqu'à **1 Mo** pouvait attendre dans le canal d'envoi (~1,6 s de retard à 5 Mbit/s) avant de sauter des images | Environ **2,5 images** (96 à 512 Ko, `Governor`) ; la capture est même suspendue quand le canal est saturé (inutile d'encoder pour jeter) |
| Un écran fixe était encodé et envoyé 10 fois par seconde | Rien n'est envoyé si l'écran n'a pas changé (empreinte 64 bits), sauf un rappel toutes les 1,5 s qui rattrape toute image perdue |
| Canal d'images fiable **et ordonné** : une seule perte bloquait toutes les images suivantes | Canal **non ordonné à fiabilité partielle** (retransmission 300 ms puis abandon) ; l'assembleur tolère désordre et pertes, ignore ce qui est plus ancien que l'image affichée |
| Déplacements de souris sur le canal fiable (retardés derrière une perte) | Canal **pointeur non fiable** pour les déplacements (la dernière position suffit) ; clics et touches restent sur le canal fiable |
| Qualité automatique fondée sur les images reçues (faux sur un écran fixe) | Fondée sur la latence, les **images abandonnées par l'agent** et l'état « écran fixe » |
| Sondage de la négociation toutes les 400 ms | 150 ms |

L'agent envoie `stats{fps, kbps, capture_ms, encode_ms, dropped, idle}` toutes les 2 s, **seulement** aux contrôleurs qui utilisent les messages récents (`ping`, `stream`, `screen`) : un ancien contrôleur prendrait ce message pour les informations de l'écran. L'application de bureau les affiche dans l'infobulle de la pastille de latence.

**Mesurer sur ta machine :** `cargo test --release encode_speed -- --ignored --nocapture` donne le temps de traitement d'une image 1080p.

**Prochaine étape qui changerait vraiment la donne :** envoyer l'écran comme une **piste vidéo H.264/VP8** (RTP) au lieu d'images JPEG séparées. Compression entre images (5 à 10 fois moins de débit à qualité égale), contrôle de congestion et correction d'erreurs intégrés à WebRTC, décodage matériel chez le contrôleur. Le coût : un encodeur vidéo côté agent (Media Foundation sous Windows, ou OpenH264) et une refonte de l'affichage du contrôleur.

## Sécurité

- La **clé privée ne quitte jamais l'appareil** (fichier de configuration en 0600 sous Unix). Le serveur ne connaît que la clé publique.
- L'enregistrement est **signé** (preuve de possession, horodatage + nonce : non rejouable). La réponse WebRTC est signée **sur l'offre reçue**,
  empreinte DTLS comprise : même avec la clé d'API, on ne peut pas se faire passer pour l'agent. À la connexion, l'agent signe un défi du serveur.
- Les permissions sont **vérifiées par l'agent à chaque commande** (`src/input/mod.rs`), indépendamment du serveur et du contrôleur.
- Pas de contournement de l'UAC ni de Ctrl+Alt+Suppr ; une session s'arrête à `stop`, `quit`, ou quand le contrôleur se déconnecte.
- Limites connues : pas d'audio, un seul écran (le principal), pas de presse-papiers ni de fichiers (V2 du doc)
  non traitée (coordonnées normalisées sur l'écran principal).

## Protocole WebSocket (`/api/remote/agent/ws?device_id=…`, en-tête `x-api-key`)

| Sens | Message |
| --- | --- |
| serveur → agent | `challenge{nonce}` · `ready{ice_servers}` · `open_session{session_id, permissions, ice_servers}` · `offer{session_id, sdp}` · `ice{session_id, candidate}` · `reconnect{session_id}` · `close_session{session_id}` · `error{detail}` |
| agent → serveur | `auth{signature}` · `grant{permissions, platform?}` (platform : type de machine détecté, « windows », « windows-laptop », « windows-mini ») · `session_reply{session_id, accepted, reason}` · `answer{session_id, sdp, signature}` · `ice{session_id, candidate}` · `resume{session_id}` · `bye{session_id}` · `ping` |

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

## Identité et signature de l'exe Windows

**Identité (déjà en place).** `build.rs` intègre à l'exe, quand on le compile sous Windows, une icône (`assets/icon.ico`, la marque ARIA), le nom du produit, une description et la version de `Cargo.toml`. Windows les affiche dans l'explorateur (Propriétés › Détails), le gestionnaire de tâches et la fenêtre d'avertissement. Pour changer l'éditeur ou le copyright affichés, modifie les lignes `CompanyName` et `LegalCopyright` de `build.rs`. Le numéro de version suit `version` dans `Cargo.toml` : change-le avant de publier une étiquette `agent-vX.Y.Z`.

**Signature (à activer).** Le message SmartScreen « Éditeur inconnu » ne disparaît qu'avec une signature de code. Le workflow `.github/workflows/build-agent.yml` signe l'exe tout seul dès que ces deux secrets existent (GitHub › Settings › Secrets and variables › Actions) :

| Secret | Contenu |
| --- | --- |
| `WINDOWS_CERT_PFX_BASE64` | le certificat de signature de code (`.pfx`) encodé en base64 |
| `WINDOWS_CERT_PASSWORD` | le mot de passe du `.pfx` |

Pour encoder le certificat, dans PowerShell : `[Convert]::ToBase64String([IO.File]::ReadAllBytes("certificat.pfx")) | Set-Clipboard`, puis colle le contenu dans le secret. La signature est faite avant le calcul de l'empreinte SHA-256, donc l'empreinte publiée est celle du fichier signé. Sans ces secrets, l'exe est produit non signé et le résumé de l'exécution l'indique.

Une signature n'efface pas toujours l'avertissement dès le premier jour : Windows tient aussi compte de la réputation du fichier. Un service de signature en ligne (Azure Trusted Signing) demanderait d'autres étapes dans le workflow, à ajouter le jour où le compte existe.
