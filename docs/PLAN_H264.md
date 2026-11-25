# Chantier H.264 : envoyer l'écran comme une vidéo

Statut : préparation ; étapes 1 et 2 faites, voir « Résultats du banc d'essai » et « Résultats du prototype de transport ». Rien d'implémenté dans l'agent. Point de départ : agent 1.12.0, desktop 0.5.2.

## Objectif

Remplacer les JPEG envoyés un par un (canal de données `frames`) par un flux vidéo H.264. Gain attendu : 5 à 10 fois moins de débit à qualité égale, donc moins de latence sur une liaison montante limitée. Le critère de réussite doit être mesuré, pas supposé (voir « Mesures »).

## État actuel (à connaître avant de toucher)

- Agent (`agent/src`) : `session.rs::start_frames` enchaîne capture (thread, `xcap`, images RGBA) -> encodage JPEG (thread, `jpeg-encoder` + `fast_image_resize`) -> envoi (tâche tokio, `chunk_frame` sur le canal `frames`). Les étapes sont reliées par des `Slot` « dernière valeur » (`pipeline.rs`). Le `Governor` borne la file d'envoi, `Counters` produit les statistiques.
- Transport : `webrtc` 0.14 (webrtc-rs). Canaux `frames` (non ordonné, `maxPacketLifeTime` 300 ms), `control`, `pointer` (non fiable). Les offres du contrôleur sont signées (`docs/SIGNATURE_CONTROLEUR.md`).
- Contrôleur (`ui/src`) : `remoteApi.js` crée les canaux et réassemble les images (`FrameAssembler`) ; `RemoteScreen.jsx` décode avec `createImageBitmap` et dessine sur un `<canvas>`. La qualité automatique et les préréglages sont dans `remoteTools.js`. Le desktop (Tauri 2) héberge cette interface dans WebView2.

## Point à corriger dans l'hypothèse de départ

« Contrôle de congestion et correction d'erreurs intégrés à WebRTC » est vrai côté navigateur. Ici l'ÉMETTEUR est l'agent en Rust (webrtc-rs). Cette bibliothèque fournit NACK et TWCC (retours de réception), mais pas d'estimateur de bande passante de type GCC. Avec une vraie piste vidéo, l'agent devra donc piloter lui-même le débit de l'encodeur à partir des rapports RTCP/statistiques. Il faut le prévoir dans le chantier, pas l'attendre gratuitement.

## Deux options de transport

| | A. Piste vidéo WebRTC (RTP H.264) | B. H.264 dans le canal `frames` + WebCodecs |
|---|---|---|
| Affichage | `<video>` ou `requestVideoFrameCallback`, tampon de gigue du navigateur | `VideoDecoder` (WebCodecs) vers `<canvas>` |
| Pertes | NACK, PLI/FIR (demande d'image clé) natifs | À écrire : demande d'image clé sur `control`, resynchronisation |
| Débit | À piloter depuis RTCP (voir ci-dessus) | À piloter depuis `buffered_amount` et les stats existantes |
| Impact sur l'existant | Nouveau média dans le SDP (signature des offres à revoir), canvas remplacé | Garde canvas, curseur, capture PNG, signalisation inchangée |
| Latence | Tampon de gigue du navigateur à régler (`playoutDelayHint`) | Maîtrisée par nous |

Recommandation : commencer par A en prototype isolé (c'est le but du chantier : profiter de NACK/PLI/TWCC), et garder B comme repli si webrtc-rs ou le tampon de gigue posent problème. Ne pas coder les deux en parallèle avant d'avoir mesuré A.

## Encodeur côté agent (Windows)

- Premier choix : Media Foundation (MFT H.264), via le crate `windows`. Utilise l'encodeur matériel (NVENC, Quick Sync, AMF) quand il existe, sinon l'encodeur logiciel fourni par Windows. Aucune licence à gérer côté projet.
- Repli possible : `openh264` (logiciel). Attention aux brevets H.264 : seul le binaire Cisco est couvert, une compilation depuis les sources ne l'est pas.
- Réglages à fixer : profil Constrained Baseline ou Main (compatibilité des décodeurs), pas d'images B (latence), image clé à la demande + toutes les N secondes, débit cible dynamique (CBR/VBR plafonné), 1 tranche par image.
- Entrée : `xcap` renvoie du RGBA CPU. Il faut une conversion RGBA -> NV12 (rapide, SIMD) avant l'encodeur ; une capture DXGI/Windows.Graphics.Capture en texture GPU éviterait cette copie mais est un chantier distinct (à ne pas mélanger).
- Conserver la logique actuelle : empreinte d'écran fixe (ne pas encoder), pause de capture si saturé, un `Slot` entre chaque étape.

## Côté contrôleur

- Détection de support : `RTCRtpReceiver.getCapabilities('video')` (option A) ou `VideoDecoder.isConfigSupported` (option B) dans WebView2 et dans les navigateurs de la page `download`/web. Prévoir le repli JPEG si H.264 n'est pas décodable.
- Négociation : l'agent annonce une fonctionnalité (`features`, comme `stream`/`pointer` aujourd'hui) ; un ancien agent ou un ancien contrôleur reste en JPEG. Le protocole doit rester rétrocompatible.
- Qualité automatique : aujourd'hui elle ajuste fps, qualité JPEG et largeur. Pour H.264 elle devient : débit cible, fps, résolution, à partir de la latence, du débit mesuré et des images abandonnées.
- Curseur, souris (`pointer`), presse-papiers, choix d'écran, capture PNG : inchangés. La capture PNG lit le canvas ; avec `<video>` il faudra dessiner l'image courante sur un canvas.

## Étapes proposées

1. **Banc d'essai hors ligne** (sans WebRTC) : encoder la source synthétique et un enregistrement d'écran en H.264 via MFT, mesurer débit, temps d'encodage par image, qualité (PSNR/SSIM) face au JPEG actuel aux trois préréglages. Décision go/no-go sur le gain réel.
2. **Prototype transport** : une piste vidéo H.264 entre l'agent et une page de test, sur un réseau simulé (perte 2 %, délai, débit limité : outil à choisir, p. ex. `clumsy` sous Windows). Valider NACK/PLI et le pilotage du débit.
3. **Négociation et repli** : annonce de la fonctionnalité, choix automatique H.264/JPEG, signature des offres avec le nouveau média, tests de non-régression de l'ancien chemin.
4. **Intégration contrôleur** : affichage, curseur, capture, statistiques de l'infobulle (débit, images abandonnées, latence).
5. **Qualité automatique H.264** et réglages par préréglage.
6. **Test réel sur Internet** (c'est la première vraie mesure, voir ci-dessous), puis publication agent + desktop.

## Résultats du banc d'essai (étape 1, 25 nov. 2026)

Méthode : `ffmpeg` sur cette machine Windows, 8 s à 10 images/s en 1600 px, source sans perte. `h264_mf` de ffmpeg passe par Media Foundation, comme l'encodeur prévu pour l'agent. Aucun code de l'agent n'a servi. Deux scénarios : une capture du bureau réel (très peu de mouvement) et un défilement simulé de 8 px par image. Débit en kbit/s, qualité en SSIM (1 = identique).

| Scénario | JPEG q5 | JPEG q9 | Media Foundation logiciel (2 Mbit/s demandés) | `libx264` ultrafast (2 Mbit/s demandés) | NVENC p1 (2 Mbit/s demandés) |
|---|---|---|---|---|---|
| Bureau réel | 9 353 (0,986) | 7 043 (0,979) | 927 (0,996) | 771 (0,999) | 305 (0,999) |
| Défilement | 8 255 (0,992) | 6 227 (0,986) | 1 040 (0,998) | 980 (0,9997) | 454 (0,999) |

Lecture :
- H.264 à qualité égale ou supérieure : environ 8 à 10 fois moins de débit avec Media Foundation logiciel (sans GPU), jusqu'à environ 25 fois avec NVENC.
- Vitesse (80 images, 1600 px) : Media Foundation logiciel ~1,5 ms par image sur ce contenu, `libx264` ultrafast du même ordre, très en dessous des ~18 ms du JPEG actuel. NVENC a un coût de démarrage d'environ 0,8 s.
- `h264_mf` avec `-hw_encoding 1` n'a rien produit ici : échec ou absence de support sur cette machine, à élucider.

Limites (ne pas sur-interpréter) :
- Contenu trop statique : le gain réel dépendra de l'usage.
- `ffmpeg` encode toutes les images ; l'agent saute celles d'un écran fixe, donc l'écart réel avec JPEG sera plus petit.
- Les réglages JPEG ne sont pas ceux de l'agent (préréglages 35, 55 et 80) : les SSIM de ce tableau ne sont pas ceux des préréglages.
- Non mesuré : pertes de paquets, latence, pilotage du débit, conversion RGBA vers NV12.

Décision : go pour l'étape 2 (prototype de transport). Le gain en débit est net même sans GPU.

## Résultats du prototype de transport (étape 2, 25 nov. 2026)

Sonde : `agent/examples/h264_probe.rs` (+ page `h264_probe.html`). Elle envoie un fichier H.264 de `ffmpeg` (1600x900, 10 images/s, ~1,8 Mbit/s, image clé toutes les 2 s) sur une vraie piste vidéo WebRTC de `webrtc-rs` 0.14 vers le navigateur intégré (Chromium), et simule perte, délai et gigue juste avant le réseau (les retransmissions NACK y sont soumises aussi). Lancer : `cargo run --release --example h264_probe -- clip.h264 --loss 2 --delay 40 --jitter 10`, puis ouvrir `http://127.0.0.1:8099/`.

Mesures sur 30 s après 15 s de chauffe, 40 ms de délai + 10 ms de gigue (300 images envoyées par fenêtre) :

| Perte | Images décodées | Gels | Durée des gels |
|---|---|---|---|
| 0 % (témoin) | 260 | 1 | 2,2 s |
| 2 % | 260 | 0 | 0 |
| 5 % | 276 | 1 | 0,37 s |
| 10 % | 270 | 2 | 0,7 s |

Constats :
- Le navigateur décode du H.264 reçu en RTP depuis `webrtc-rs` : ~1 ms par image, tampon de réception de 11 à 83 ms.
- Le flux continue à ~9 images/s jusqu'à 10 % de perte simulée ; les gels restent sous la seconde (sauf un gel de 2,2 s dans le témoin sans perte, donc du bruit de mesure).
- Le nombre de NACK (~650 à 700 par 30 s) ne dépend pas du taux de perte : ils viennent surtout des paquets réordonnés par la gigue simulée (NACK inutiles), pas des vraies pertes.
- `interceptor` 0.15 n'a pas d'estimateur de bande passante (GCC) : confirmé dans le code. Le pilotage du débit de l'encodeur reste à écrire côté agent.
- `H264Reader` de `webrtc-media` 0.11 découpe mal ce flux (14 morceaux par image) : découpage Annex B écrit à la main dans la sonde, à reprendre dans l'agent. `libx264` en mode `zerolatency` produit 14 tranches par image ; l'encodeur Media Foundation produira autre chose. Envoyer chaque image entière en un seul échantillon.

Limites : un seul passage de 30 s par cas, aucune tendance nette (le témoin gèle aussi) ; tout en local, pertes simulées côté émetteur ; pas de limite de débit, pas de latence de bout en bout, pas de NAT ni de relais ; navigateur intégré (Chromium), pas WebView2 du desktop.

À faire ensuite (étape 3) : tester dans WebView2, mesurer la latence de bout en bout, passes plus longues et répétées, simulation d'un débit limité.

## Mesures à faire AVANT de se lancer

La latence réelle de l'agent 1.12.0 sur un vrai lien Internet n'a pas été mesurée (l'infobulle de la pastille de latence donne débit, temps de capture et d'encodage, images abandonnées). Si le goulot est le débit montant, H.264 règle le problème ; si c'est la latence réseau pure ou la capture, le gain sera plus faible. Relever ces valeurs d'abord.

## Risques

- Pas d'estimateur de bande passante dans webrtc-rs (voir plus haut) : travail de pilotage du débit à ajouter.
- Dépendances Windows lourdes (`windows` + Media Foundation) : temps de compilation, taille de l'exe, tests non exécutables sous Linux (la CI Windows existe : `Agent Windows`).
- Décodage H.264 non garanti partout (certaines versions de Chromium/Linux sans codec) : le repli JPEG est obligatoire.
- Latence de démarrage et reprise après perte : une image clé coûte cher en débit, à demander avec parcimonie.
- Compatibilité : anciens agents/contrôleurs en circulation (versions 1.x / 0.x) doivent continuer à fonctionner.

## Questions ouvertes

- Matériel cible de l'agent : GPU avec encodeur matériel sur les machines visées ?
- Faut-il aussi le web (page `download`/UI hébergée) ou seulement le desktop Tauri (WebView2) ?
- Une latence cible chiffrée (p. ex. < 150 ms de bout en bout sur 4G) pour décider de la réussite ?
