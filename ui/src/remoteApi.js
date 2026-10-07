import { apiErrorMessage } from '../apiRuntime'

// Appels, négociation WebRTC et traduction des entrées pour le contrôle à distance
// (backend/plugins/remote, docs/REMOTE_ARCHITECTURE.md). Le navigateur est le CONTRÔLEUR : il propose l'offre,
// l'agent (agent/) répond. Le backend ne fait que relayer le signaling ; l'écran et les commandes passent
// par deux canaux de données WebRTC :
//   « frames »  : l'agent envoie une image JPEG par message (binaire) ;
//   « control » : JSON dans les deux sens (infos écran, souris, clavier).
const BASE = '/api/remote'
export const POLL_MS = 400
export const CONNECT_TIMEOUT_MS = 30000
export const MAX_RECONNECTS = 3
const DISCONNECTED_GRACE_MS = 5000

export async function api(path, options) {
  const response = await fetch(`${BASE}${path}`, options)
  const data = await response.json().catch(() => ({}))
  if (!response.ok) throw new Error(apiErrorMessage(data.detail, `Erreur HTTP ${response.status}`))
  return data
}

export const post = (path, body) => api(path, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body ?? {}) })

const sleepMs = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

// --- Entrées : coordonnées normalisées (0..1) pour être indépendantes de la taille d'affichage -----------------

export function pointerPosition(event, element) {
  const rect = element.getBoundingClientRect()
  if (!rect.width || !rect.height) return { x: 0, y: 0 }
  const clamp = (value) => Math.min(1, Math.max(0, value))
  return { x: clamp((event.clientX - rect.left) / rect.width), y: clamp((event.clientY - rect.top) / rect.height) }
}

export function pointerMessage(type, event, element) {
  const message = { t: type, ...pointerPosition(event, element) }
  if (type === 'down' || type === 'up') message.button = event.button
  return message
}

export const wheelMessage = (event) => ({ t: 'wheel', dx: Math.sign(event.deltaX) * Math.min(Math.abs(event.deltaX), 400), dy: Math.sign(event.deltaY) * Math.min(Math.abs(event.deltaY), 400) })

// `code` = touche physique (indépendante de la disposition), `key` = caractère produit, pour que l'agent choisisse.
export const keyMessage = (event, down) => ({ t: 'key', down, code: event.code, key: event.key })

// --- Images : un JPEG est découpé en morceaux (la taille d'un message WebRTC est limitée) ---------------------------
// En-tête de 8 octets, grand-boutiste : numéro d'image (u32), rang du morceau (u16), nombre de morceaux (u16).
// Le canal est fiable et ordonné : les morceaux d'une image arrivent dans l'ordre ; une nouvelle image abandonne l'incomplète.
export class FrameAssembler {
  constructor() { this.id = null; this.parts = []; this.count = 0 }

  /** Ajoute un message ; renvoie l'image complète (Uint8Array) quand son dernier morceau arrive, sinon null. */
  push(buffer) {
    if (!buffer || buffer.byteLength < 8) return null
    const view = new DataView(buffer)
    const id = view.getUint32(0)
    const index = view.getUint16(4)
    const count = view.getUint16(6)
    if (count === 0 || index >= count) return null
    if (id !== this.id || count !== this.count) { this.id = id; this.count = count; this.parts = [] }
    if (index !== this.parts.length) { this.id = null; return null }      // morceau manquant ou hors rang : image abandonnée
    this.parts.push(new Uint8Array(buffer, 8))
    if (this.parts.length < count) return null
    const frame = new Uint8Array(this.parts.reduce((total, part) => total + part.length, 0))
    let offset = 0
    for (const part of this.parts) { frame.set(part, offset); offset += part.length }
    this.id = null
    this.parts = []
    return frame
  }
}

// --- Session ----------------------------------------------------------------------------------------------------

/**
 * Ouvre une session de contrôle : la création ATTEND que la personne devant l'appareil accepte (jusqu'à une minute),
 * puis négocie WebRTC. Renvoie { sessionId, sendInput, close }.
 * États publiés par onState : asking, connecting, connected, reconnecting, lost, closed.
 * `makePeer`, `request` et `sleep` sont injectables pour les tests.
 */
export async function openSession({
  deviceId, permissions, onState = () => {}, onFrame = () => {}, onInfo = () => {}, onCursor = () => {},
  makePeer = (config) => new RTCPeerConnection(config), request = { api, post }, sleep = sleepMs, timeoutMs = CONNECT_TIMEOUT_MS,
}) {
  onState('asking')
  const session = await request.post('/sessions', { device_id: deviceId, permissions })
  const sessionId = session.session_id
  let iceServers = []
  try {
    iceServers = (await request.api(`/ice-servers?session_id=${encodeURIComponent(sessionId)}`)).ice_servers || []
  } catch { /* réseau local : pas de serveur STUN/TURN nécessaire */ }

  let pc = null
  let control = null
  let closed = false
  let generation = 0       // une renégociation invalide les écouteurs de la précédente

  async function negotiate() {
    const mine = ++generation
    pc?.close()
    onState('connecting')
    const peer = makePeer({ iceServers })
    pc = peer
    const frames = peer.createDataChannel('frames')
    frames.binaryType = 'arraybuffer'
    const assembler = new FrameAssembler()
    frames.onmessage = (event) => {
      if (mine !== generation) return
      const frame = assembler.push(event.data)
      if (frame) onFrame(frame)
    }
    control = peer.createDataChannel('control')
    control.onmessage = (event) => {
      if (mine !== generation) return
      try {
        const message = JSON.parse(event.data)
        if (message.t === 'cursor') onCursor(message)     // position du pointeur distant : ne remplace pas les infos de l'écran
        else onInfo(message)
      } catch { /* message de contrôle illisible : ignoré */ }
    }

    const queued = []            // candidats locaux produits avant que l'offre soit partie
    let offerSent = false
    const sendIce = (candidate) => request.post('/signaling/ice', { session_id: sessionId, candidate: JSON.stringify(candidate) }).catch(() => {})
    peer.onicecandidate = (event) => {
      if (!event.candidate) return
      if (offerSent) sendIce(event.candidate)
      else queued.push(event.candidate)
    }
    await peer.setLocalDescription(await peer.createOffer())
    await request.post('/signaling/offer', { session_id: sessionId, sdp: peer.localDescription.sdp })
    offerSent = true
    queued.splice(0).forEach(sendIce)

    let answered = false
    let next = 0
    const pending = []           // candidats de l'agent reçus avant la réponse
    const deadline = Date.now() + timeoutMs
    while (!closed && mine === generation && peer.connectionState !== 'connected') {
      if (Date.now() > deadline) throw new Error('Connexion impossible : aucune réponse de l\'appareil.')
      const view = await request.api(`/sessions/${sessionId}/signaling?after=${next}`)
      if (view.state === 'DISCONNECTED') throw new Error('La session a été fermée.')
      next = view.next
      pending.push(...view.ice)
      if (view.answer && !answered) {
        await peer.setRemoteDescription({ type: 'answer', sdp: view.answer })
        answered = true
      }
      if (answered) {
        for (const raw of pending.splice(0)) await peer.addIceCandidate(JSON.parse(raw)).catch(() => {})
      }
      if (peer.connectionState !== 'connected') await sleep(POLL_MS)
    }
    if (closed || mine !== generation) return
    onState('connected')
    watch(peer, mine)
  }

  let reconnects = 0
  function watch(peer, mine) {
    let timer = null
    peer.onconnectionstatechange = () => {
      if (mine !== generation || closed) return
      clearTimeout(timer)
      if (peer.connectionState === 'failed') reconnect()
      else if (peer.connectionState === 'disconnected') timer = setTimeout(() => { if (peer.connectionState !== 'connected') reconnect() }, DISCONNECTED_GRACE_MS)
    }
  }

  async function reconnect() {
    if (closed) return
    if (reconnects >= MAX_RECONNECTS) { onState('lost'); return }
    reconnects += 1
    onState('reconnecting')
    try {
      await request.post(`/sessions/${sessionId}/reconnect`)
      await negotiate()
    } catch {
      if (!closed) reconnect()
    }
  }

  try {
    await negotiate()
  } catch (error) {
    await close()
    throw error
  }

  function sendInput(message) {
    if (control?.readyState === 'open') control.send(JSON.stringify(message))
  }

  async function close() {
    if (closed) return
    closed = true
    generation += 1
    pc?.close()
    try { await request.api(`/sessions/${sessionId}`, { method: 'DELETE' }) } catch { /* déjà terminée côté serveur */ }
    onState('closed')
  }

  return { sessionId, sendInput, close }
}
