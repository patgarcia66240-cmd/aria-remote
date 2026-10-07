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
Fait : **connexion au serveur** (adresse + clé du contrôleur, testées puis mémorisées), **liste des appareils** (mise à jour toutes les 4 s), **appairage par code** à 6 chiffres et **prise de contrôle** : écran distant, souris, clavier, curseur distant, avec le choix des autorisations (souris / clavier) avant de se connecter.
Le contrôle n'est possible que si l'appareil l'autorise et que la personne devant l'appareil accepte (une fenêtre s'ouvre sur l'agent).

Comment ça marche : la fenêtre négocie la connexion WebRTC avec l'agent (`web/session.js`, même logique que l'onglet d'ARIA) ; le serveur de rendez-vous ne relaie que le signaling. Les appels au serveur passent par la commande Rust `api_request`, qui n'autorise que les routes du contrôleur.

Sécurité : la clé du contrôleur reste côté Rust (fichier de configuration de l'utilisateur) et n'est jamais renvoyée à la fenêtre ; une adresse `http://` n'est acceptée que sur un réseau local ; la fenêtre ne peut jamais appeler les routes d'agent.

## Tests
```
npm test                     # négociation, images, entrées (Node.js)
cargo test                   # réseau, réglages, routes autorisées (Rust)
```

## Feuille de route
1. ~~Réglages~~, ~~appareils et appairage~~, ~~session (écran, souris, clavier)~~ : faits.
2. Presse-papiers partagé, transfert de fichiers, plein écran et choix de l'écran distant.
3. Signature de l'exe Windows (comme l'agent) et icône d'application complète.
4. Compilation et publication Windows : faites (workflow `build-desktop.yml`, étiquette `desktop-vX.Y.Z`) ; la page de téléchargement propose la dernière version dès qu'elle existe.

Le protocole (messages, signatures Ed25519) est décrit dans `../docs/REMOTE_ARCHITECTURE.md` ; le code de référence est `../plugin/` (côté contrôleur) et `../agent/src/identity.rs`.
