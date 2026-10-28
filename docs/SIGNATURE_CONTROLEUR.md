# Signature des offres du contrôleur (point 1 de l'audit)

## Problème
L'agent accepte une offre WebRTC pour une session déjà consentie (reprise après coupure) sans pouvoir vérifier qui l'a émise.
Un intermédiaire capable de modifier les messages (ou un détenteur de la clé d'API du serveur) peut substituer sa propre offre.

## Objectif
Aucune offre n'est acceptée par l'agent sans signature valide d'une clé de contrôleur qu'il a lui-même acceptée à l'appairage.

## Choix retenus
- **Q1-b** : l'application de bureau ET l'onglet ARIA peuvent contrôler. Chaque contrôleur a sa propre clé Ed25519.
- **Q2-a** : à l'appairage, l'agent affiche l'empreinte de la clé du contrôleur ; la personne la compare puis accepte.

## Clés
| Élément | Où il vit | Règle |
|---|---|---|
| Clé privée du contrôleur (bureau) | Stockage local de l'application | Ne quitte jamais l'appareil |
| Clé privée du contrôleur (onglet ARIA) | WebCrypto, clé non extractible, IndexedDB | Ne quitte jamais le navigateur |
| Clé publique du contrôleur | Serveur (appairage) et agent (après acceptation) | Publique |
| Empreinte | Affichée par l'agent | SHA-256 tronqué de la clé publique |

## Appairage (étendu)
1. Le contrôleur envoie son code à 6 chiffres ET sa clé publique (signée par la clé privée du contrôleur, pour prouver la possession).
2. L'agent calcule l'empreinte et l'affiche dans sa fenêtre, à côté du code.
3. La personne compare les empreintes (bureau / fenêtre de l'agent) puis accepte. Sans acceptation, l'appareil n'est pas appairé.
4. La clé publique acceptée est stockée côté agent (fichier local). Elle ne peut être changée qu'en réappairant.

## Offres (chaque session)
- Le contrôleur signe `pc-assistant-remote-v1|controller-offer|<session_id>|<sha256(offre)>` avec sa clé privée.
- Le serveur relaie la signature telle quelle (il ne l'utilise pas).
- L'agent vérifie la signature avec la clé publique acceptée à l'appairage. Échec = offre refusée, journalisée.
- Une offre de renégociation doit être signée ET correspondre à la session active.

## Découpage en phases (chaque phase garde les tests verts)
1. **Phase 1 (cette étape)** : spécification (ce document).
2. **Phase 2** : agent Rust — stockage de la clé publique acceptée, empreinte affichée, vérification des offres en mode « journalisation seule » (ne refuse pas encore).
3. **Phase 3** : serveur et plugin — relais de la clé publique et de la signature, tests d'appairage étendu.
4. **Phase 4 (faite)** : bureau — `desktop/src/controller_key.rs` génère la clé au premier usage (fichier `controller.seed` dans le dossier de configuration, lecture refusée si erreur autre que « absent » pour ne jamais écraser une clé), commandes `controller_identity` et `controller_sign_offer`, offres signées dans `session.js` (sans signature si la signature échoue, en attendant la phase 6), clé publique envoyée à `/pair` et empreinte affichée dans le formulaire d'appairage. Tests : `npm test` (51 verts) ; tests Rust (`cargo test`) à lancer par la CI Windows.
5. **Phase 5 (faite)** : onglet ARIA — `ui/src/controllerKey.js` crée une clé Ed25519 WebCrypto non extractible, rangée dans IndexedDB ; l'appairage envoie la clé publique et affiche l'empreinte ; les offres sont signées. Sans WebCrypto Ed25519 (navigateur ancien), l'appairage est refusé avec un message explicite. Vérifiée par `signature.js`-équivalent sous Node (scratchpad) ; les tests Vitest de `ui/` ne tournent pas dans ce dépôt (chemins d'import obsolètes, pas de `package.json`).
6. **Phase 6** : passage en mode strict (refus des offres non signées) une fois les contrôleurs à jour.

## Compatibilité
Le mode « journalisation seule » de la phase 2 permet de déployer l'agent avant les contrôleurs, sans couper les sessions existantes.

## Tests attendus (phase 6)
- offre signée valide : acceptée ;
- offre non signée : refusée ;
- signature d'une autre clé : refusée ;
- offre rejouée sur une autre session : refusée ;
- clé publique non acceptée à l'appairage : refusée.
