// Écran distant en H.264 (docs/PLAN_H264.md, étape 4) : l'agent envoie l'écran sur une piste vidéo WebRTC au lieu d'images JPEG.
// On DESSINE chaque image décodée dans le même <canvas> que le JPEG : la souris, le curseur distant, la capture PNG et le reste ne changent pas.

/** Codecs H.264 en paquétisation 1 que ce navigateur sait décoder (ceux que l'agent encode) ; liste vide si aucun. */
export function h264Codecs(receiver = globalThis.RTCRtpReceiver) {
  try {
    const codecs = receiver?.getCapabilities?.('video')?.codecs || []
    return codecs.filter((codec) => String(codec.mimeType).toLowerCase() === 'video/h264' && /packetization-mode=1/.test(codec.sdpFmtpLine || ''))
  } catch {
    return []
  }
}

/** Ce navigateur peut-il afficher la vidéo dans le canvas ? Il faut décoder le H.264 ET recevoir un signal à chaque image (requestVideoFrameCallback). */
export function canShowVideo({ receiver = globalThis.RTCRtpReceiver, video = globalThis.HTMLVideoElement } = {}) {
  return h264Codecs(receiver).length > 0 && !!video && 'requestVideoFrameCallback' in video.prototype
}

/**
 * La vidéo est-elle bloquée ? Vrai si ni image ni démarrage (`since`) n'ont eu lieu depuis `stallMs`. L'appelant remet `since` à « maintenant » tant que la
 * page est cachée (fenêtre réduite : le navigateur ne dessine plus rien, ce n'est pas une panne).
 */
export function videoStalled({ now, lastFrameAt, since, stallMs }) {
  return now - Math.max(lastFrameAt ?? 0, since) >= stallMs
}

/**
 * Affiche un flux vidéo dans `canvas` : une <video> invisible le décode, chaque image est recopiée dans le canvas. `onFrame` est appelé à chaque image.
 * Renvoie { stop(), lastFrameAt() } ; `lastFrameAt()` donne l'heure (ms) de la dernière image affichée, null tant qu'aucune n'est arrivée.
 */
export function showVideo(stream, canvas, onFrame = () => {}, { doc = globalThis.document, now = () => performance.now() } = {}) {
  const video = doc.createElement('video')
  video.muted = true
  video.autoplay = true
  video.playsInline = true
  video.setAttribute('aria-hidden', 'true')
  Object.assign(video.style, { position: 'absolute', width: '1px', height: '1px', opacity: '0', pointerEvents: 'none' })
  canvas.parentNode?.append(video)
  video.srcObject = stream
  video.play?.().catch?.(() => {})

  let stopped = false
  let last = null
  const draw = () => {
    if (stopped) return
    if (video.videoWidth > 0 && video.videoHeight > 0) {
      if (canvas.width !== video.videoWidth || canvas.height !== video.videoHeight) {
        canvas.width = video.videoWidth
        canvas.height = video.videoHeight
      }
      canvas.getContext('2d').drawImage(video, 0, 0)
      last = now()
      onFrame()
    }
    video.requestVideoFrameCallback(draw)
  }
  video.requestVideoFrameCallback(draw)

  return {
    stop() {
      stopped = true
      video.srcObject = null
      video.remove()
    },
    lastFrameAt: () => last,
  }
}
