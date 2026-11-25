// Vérifie le chemin d'affichage du contrôleur de bureau (desktop/web/video.js) avec la vraie vidéo de la sonde, sous la même politique de sécurité
// (CSP) que la fenêtre Tauri : voir la route /canvas de h264_probe.rs. Résultat dans window.__canvas et dans #out.
import { h264Codecs, showVideo } from '/video.js'

const out = document.getElementById('out')
const canvas = document.getElementById('screen')
const state = { codecs: h264Codecs().length, frames: 0, lastFrameAgeMs: null, size: null, litPixels: 0, sampled: 0, error: null }
window.__canvas = state

function sample() {
  try {
    const ctx = canvas.getContext('2d')
    const { data } = ctx.getImageData(0, 0, canvas.width, canvas.height)
    let lit = 0
    let total = 0
    for (let i = 0; i < data.length; i += 4 * 997) {
      total += 1
      if (data[i] + data[i + 1] + data[i + 2] > 60) lit += 1
    }
    state.litPixels = lit
    state.sampled = total
  } catch (error) {
    state.error = String(error)
  }
}

try {
  const pc = new RTCPeerConnection()
  const transceiver = pc.addTransceiver('video', { direction: 'recvonly' })
  transceiver.setCodecPreferences(h264Codecs())
  let shown = null
  pc.ontrack = (event) => {
    shown = showVideo(event.streams[0] || new MediaStream([event.track]), canvas, () => { state.frames += 1 })
    setInterval(() => {
      const last = shown.lastFrameAt()
      state.lastFrameAgeMs = last === null ? null : Math.round(performance.now() - last)
      state.size = [canvas.width, canvas.height]
      sample()
      out.textContent = JSON.stringify(state, null, 2)
    }, 500)
  }
  await pc.setLocalDescription(await pc.createOffer())
  await new Promise((resolve) => { if (pc.iceGatheringState === 'complete') resolve(); else pc.onicegatheringstatechange = () => pc.iceGatheringState === 'complete' && resolve() })
  const response = await fetch('/offer', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ sdp: pc.localDescription.sdp }) })
  await pc.setRemoteDescription({ type: 'answer', sdp: (await response.json()).sdp })
} catch (error) {
  state.error = String(error)
  out.textContent = JSON.stringify(state, null, 2)
}
