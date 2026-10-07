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
Squelette : la fenêtre s'ouvre et affiche sa version (`app_version`). Rien d'autre n'est branché.

## Feuille de route
1. Réglages : adresse du serveur et clé contrôleur (mémorisées).
2. Appareils : liste, état en ligne, appairage par code à 6 chiffres.
3. Session : demande d'accord, écran distant, souris et clavier, curseur distant.
4. Compilation et publication Windows (étiquette `desktop-vX.Y.Z`, sa propre version), puis page de téléchargement.

Le protocole (messages, signatures Ed25519) est décrit dans `../docs/REMOTE_ARCHITECTURE.md` ; le code de référence est `../plugin/` (côté contrôleur) et `../agent/src/identity.rs`.
