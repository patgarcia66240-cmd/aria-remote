# Une seule application ARIA Remote (contrôler + être contrôlé)

## Objectif
Avant : `agent/` (hôte, page web locale ouverte dans Edge), `desktop/` (contrôleur Tauri) et `ui/` (contrôleur dans ARIA) étaient trois surfaces.
Maintenant : **le desktop fait les deux** (onglets **Contrôler** et **Cet appareil**, réglage **Mode**). L'exe `remote-agent` seul reste pour les PC sans fenêtre.
L'onglet « Maintenance à distance » d'ARIA ne fait que **prendre le contrôle** : il ne change pas de rôle.

## Invariants à ne jamais casser
- L'hôte n'est jamais caché : fenêtre ou icône près de l'horloge visible tant que l'hôte est actif ; accord demandé pour CHAQUE session (la fenêtre remonte au premier plan).
- La clé du contrôleur et la clé privée de l'appareil restent côté Rust, jamais envoyées à la fenêtre.
- Un agent ancien et un contrôleur récent (et inversement) continuent de fonctionner (message `info` > `features`).
- Une seule instance d'agent par appareil (verrou `config.lock`).

## Ce qui est fait
1. **Moteur en bibliothèque** (agent 1.10.0) : `agent/src/lib.rs` (`launch`, `Launched`, `Settings`, `Interface::{Console, Web, Embedded}`) ; `remote-agent.exe` n'est que la ligne de commande et l'ouverture de la fenêtre. Verrou d'instance (`instance.rs`).
2. **Desktop 0.5.0** : l'agent tourne dans le desktop (`desktop/src/host.rs`), onglets, mode (`config.rs`), icône près de l'horloge et fermeture vers l'icône, démarrage avec Windows (`autostart.rs`).
3. **Desktop 0.5.1** : pastille d'état dans l'en-tête (« Disponible », « Code 483 921 », « Contrôlé »), compte à rebours de la demande d'accord.
4. **Logo ARIA** partout (icônes, pages, illustrations) ; DPI Windows (agent 1.8.1) ; réduction de la fenêtre de l'agent seul pendant une session (agent 1.9.0).

## Reste à faire
- **Session JS partagée** : `ui/src/remoteApi.js` et `remoteTools.js` sont des copies de `desktop/web/session.js` et `tools.js`. Les faire ré-exporter une seule version demande d'autoriser ce chemin côté ARIA (`frontend/remoteIntegration.js`, `server.fs.allow`) : à faire avec une mise à jour du sous-module dans pc-assistant.
- **État poussé** : aujourd'hui la fenêtre sonde l'agent toutes les 700 ms (`agent_state`). Remplaçable par un événement Tauri, sans changement visible.
- **Cas limites** : le PC se contrôle lui-même (masquer l'appareil local ou le marquer « Ce PC »), session sortante et entrante en même temps (la réduction automatique ne doit viser que la fenêtre de l'hôte).
- **Test réel** sur un PC : icône, fermeture vers l'icône, démarrage avec Windows, écrans multiples et mises à l'échelle.
- Séparer l'état et les ordres (`UiShared`, `UiCommand`, `UiState`) de la page web locale dans un crate à part (`host-core`) si le desktop et l'agent divergent un jour ; aujourd'hui la bibliothèque unique suffit.
