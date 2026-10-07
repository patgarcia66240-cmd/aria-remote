# ARIA Remote Desktop

L'application de bureau pour **contrôler** un PC à distance, sans passer par ARIA : on choisit un appareil, on saisit son code d'appairage, on voit son écran et on pilote souris et clavier.
Elle parle au même serveur de rendez-vous (`../rendezvous/`) et aux mêmes agents (`../agent/`) que l'onglet d'ARIA.

Technologie : [Tauri 2](https://tauri.app) (cœur en Rust, interface web), comme l'application de bureau de PC Assistant.

## Lancer
Prérequis : Rust, Node.js, et sous Linux les bibliothèques WebKitGTK (voir la page « Prerequisites » de Tauri).
```
cd desktop
npm install
npm run dev
```

## État
Fait : **aperçu de chaque appareil** (PC de bureau, mini PC ou portable, détecté automatiquement), **connexion au serveur** (adresse + clé du contrôleur, testées puis mémorisées), **liste des appareils**, **appairage par code** à 6 chiffres, et **prise de contrôle** : écran distant, souris, clavier, curseur distant, avec le choix des autorisations avant de se connecter.
Le contrôle n'est possible que si l'appareil l'autorise et que la personne devant l'appareil accepte (une fenêtre s'ouvre sur l'agent).

### Pendant une session (barre d'outils de l'en-tête)
| Outil | Ce qu'il fait |
|---|---|
| **Latence et images/s** | temps d'aller-retour avec l'appareil (vert < 80 ms, orange < 200 ms, rouge au-delà) et images envoyées par seconde (« écran fixe » quand rien ne change). L'infobulle détaille le débit, les temps de capture et d'encodage et les images abandonnées |
| **Qualité de l'image** | Auto (s'adapte à la connexion : baisse vite, remonte lentement ; fondée sur la latence, les images que l'agent a dû abandonner et l'état « écran fixe »), Économie, Équilibré, Haute qualité |
| **Écran** | choisir l'écran distant quand l'appareil en a plusieurs ; la souris suit l'écran choisi |
| **Raccourcis** | touche Windows, Alt + Tab, Ctrl + Maj + Échap (gestionnaire des tâches), Alt + F4. Ctrl + Alt + Suppr est réservé à Windows : impossible à distance |
| **Presse-papiers partagé** | le texte copié d'un côté se colle de l'autre (256 Ko max, sans écho). Il suit la permission **clavier** : sans elle, rien n'est lu ni écrit |
| **Capture d'écran** | enregistre l'écran distant en PNG dans le dossier Images (`ARIA-Remote-<appareil>-<date>.png`, jamais d'écrasement) |
| **Plein écran** | F11 (ou le bouton) ; en plein écran la barre se cache et revient quand la souris touche le haut de la fenêtre. F11 n'est pas envoyé à l'appareil |

### Réseau
Le canal d'images est **non ordonné à fiabilité partielle** (un paquet perdu est retransmis 300 ms puis abandonné) : une perte n'arrête plus toutes les images suivantes. Seule la **dernière** image est décodée et affichée (les intermédiaires sont sautées plutôt que de prendre du retard). Les déplacements de souris prennent un canal **non fiable** dédié ; clics et touches restent sur le canal fiable. Détails et mesures : `../agent/README.md`, section « Latence ».

Les outils n'apparaissent que si l'agent les annonce (message `info` > `features`) : avec un agent plus ancien, la session fonctionne comme avant, sans ces boutons.

Comment ça marche : la fenêtre négocie la connexion WebRTC avec l'agent (`web/session.js`, même logique que l'onglet d'ARIA) ; le serveur de rendez-vous ne relaie que le signaling. Les appels au serveur passent par la commande Rust `api_request`, qui n'autorise que les routes du contrôleur. Le presse-papiers local et l'enregistrement des captures passent aussi par Rust (la fenêtre n'a pas accès au système de fichiers).

Sécurité : la clé du contrôleur reste côté Rust (fichier de configuration de l'utilisateur) et n'est jamais renvoyée à la fenêtre ; une adresse `http://` n'est acceptée que sur un réseau local ; la fenêtre ne peut jamais appeler les routes d'agent ; le nom d'une capture est fabriqué côté Rust à partir du nom de l'appareil nettoyé (jamais un chemin choisi par la fenêtre).

## Tests
```
npm test                     # négociation, images, entrées, outils de session (Node.js)
cargo test                   # réseau, réglages, routes autorisées, captures (Rust)
cargo check --target x86_64-pc-windows-gnu   # vérifie la compilation Windows depuis Linux (sans WebKit)
```

## Feuille de route
1. ~~Réglages~~, ~~appareils et appairage~~, ~~session (écran, souris, clavier)~~ : faits.
2. ~~Presse-papiers partagé, plein écran, choix de l'écran distant, qualité automatique, latence, capture, raccourcis~~ : faits. Reste : transfert de fichiers, son de l'appareil.
3. Signature de l'exe Windows (comme l'agent) et icône d'application complète.
4. Compilation et publication Windows : faites (workflow `build-desktop.yml`, étiquette `desktop-vX.Y.Z`) ; la page de téléchargement propose la dernière version dès qu'elle existe.

Le protocole (messages, signatures Ed25519) est décrit dans `../docs/REMOTE_ARCHITECTURE.md` ; le code de référence est `../plugin/` (côté contrôleur) et `../agent/src/identity.rs`.
