// Négociation WebRTC et traduction des entrées pour le contrôle à distance (docs/REMOTE_ARCHITECTURE.md).
// Même logique que l'onglet d'ARIA (ui/src/remoteApi.js). La fenêtre est le CONTRÔLEUR : elle propose l'offre,
// l'agent (agent/) répond. Le serveur de rendez-vous ne fait que relayer le signaling ; l'écran et les commandes passent
// par deux canaux de données WebRTC :
//   « frames »  : l'agent envoie une image JPEG par message (binaire) ;
//   « control » : JSON dans les deux sens (infos écran, souris, clavier).
export const POLL_MS = 150          // sondage de la signalisation pendant la négociation (400 ms avant : jusqu'à 0,4 s perdue pour rien à l'ouverture)
// Canal d'images : non ordonné, à fiabilité partielle. Un paquet perdu est retransmis pendant ce délai (assez pour 2 allers-retours à 150 ms) puis
// abandonné, au lieu de bloquer TOUTES les
// images suivantes derrière lui (« blocage en tête de file » : sur Internet, une perte de 1 % se traduisait par des à-coups de plusieurs centaines de ms).
export const FRAME_LIFETIME_MS = 300
// Canal pointeur : un déplacement de souris perdu n'a aucun intérêt (la position suivante le remplace), on ne le retransmet jamais.
export const POINTER_CHANNEL = { ordered: false, maxRetransmits: 0 }
export const FRAME_CHANNEL = { ordered: false, maxPacketLifeTime: FRAME_LIFETIME_MS }
export const CONNECT_TIMEOUT_MS = 30000
export const MAX_RECONNECTS = 3
const DISCONNECTED_GRACE_MS = 5000

/**
 * Transport vers le serveur de rendez-vous : chaque appel passe par la commande Rust `api_request` (qui détient la clé du contrôleur et
 * n'autorise que les routes du contrôleur). `invoke` est celui de Tauri ; injectable pour les tests.
 */
export function createRequest(invoke) {
  const api = (path, options = {}) => invoke('api_request', { method: options.method || 'GET', path, body: null })
  const post = (path, body) => invoke('api_request', { method: 'POST', path, body: body ?? {} })
  return { api, post }
}

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
// Le canal n'est NI ordonné NI fiable : les morceaux arrivent dans le désordre, certains jamais. L'assembleur remet donc chaque image dans l'ordre,
// ignore ce qui est plus ancien que la dernière image affichée (« la plus récente gagne » : jamais de retour en arrière) et abandonne les images
// restées incomplètes quand de plus récentes arrivent.
const MAX_PENDING_FRAMES = 4
const newer = (a, b) => ((a - b) | 0) > 0         // comparaison de numéros d'image qui repart de zéro après 2^32

export class FrameAssembler {
  constructor() { this.pending = new Map(); this.shown = null; this.completed = 0; this.discarded = 0 }

  /** Ajoute un message ; renvoie l'image complète (Uint8Array) quand son dernier morceau arrive, sinon null. */
  push(buffer) {
    if (!buffer || buffer.byteLength < 8) return null
    const view = new DataView(buffer)
    const id = view.getUint32(0)
    const index = view.getUint16(4)
    const count = view.getUint16(6)
    if (count === 0 || index >= count) return null
    if (this.shown !== null && !newer(id, this.shown)) return null            // plus ancienne que l'image déjà affichée (ou la même) : inutile
    let entry = this.pending.get(id)
    if (!entry || entry.count !== count) {
      entry = { count, parts: new Array(count), have: 0 }
      this.pending.set(id, entry)
      this.evict()
    }
    if (entry.parts[index]) return null                                       // morceau en double
    entry.parts[index] = new Uint8Array(buffer, 8)
    entry.have += 1
    if (entry.have < count) return null
    const frame = new Uint8Array(entry.parts.reduce((total, part) => total + part.length, 0))
    let offset = 0
    for (const part of entry.parts) { frame.set(part, offset); offset += part.length }
    this.shown = id
    this.completed += 1
    for (const other of [...this.pending.keys()]) {
      if (!newer(other, id)) { if (other !== id) this.discarded += 1; this.pending.delete(other) }       // les images plus anciennes ne serviront plus
    }
    return frame
  }

  /** Garde au plus quelques images en cours d'assemblage : les plus anciennes sont abandonnées. */
  evict() {
    while (this.pending.size > MAX_PENDING_FRAMES) {
      let oldest = null
      for (const id of this.pending.keys()) if (oldest === null || newer(oldest, id)) oldest = id
      this.pending.delete(oldest)
      this.discarded += 1
    }
  }
}

// --- Session ----------------------------------------------------------------------------------------------------

/**
 * Ouvre une session de contrôle : la création ATTEND que la personne devant l'appareil accepte (jusqu'à une minute),
 * puis négocie WebRTC. Renvoie { sessionId, sendInput, usePointerChannel, close }.
 * États publiés par onState : asking, connecting, connected, reconnecting, lost, closed.
 * `request` (voir createRequest) est obligatoire ; `makePeer` et `sleep` sont injectables pour les tests.
 */
export async function openSession({
  deviceId, permissions, onState = () => {}, onFrame = () => {}, onInfo = () => {}, onCursor = () => {},
  makePeer = (config) => new RTCPeerConnection(config), request, sleep = sleepMs, timeoutMs = CONNECT_TIMEOUT_MS,
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
  let pointer = null
  let pointerEnabled = false       // activé par l'interface quand l'agent annonce « pointer » (un ancien agent ignore ce canal)
  let closed = false
  let generation = 0       // une renégociation invalide les écouteurs de la précédente

  async function negotiate() {
    const mine = ++generation
    pc?.close()
    onState('connecting')
    const peer = makePeer({ iceServers })
    pc = peer
    const frames = peer.createDataChannel('frames', FRAME_CHANNEL)
    frames.binaryType = 'arraybuffer'
    const assembler = new FrameAssembler()
    frames.onmessage = (event) => {
      if (mine !== generation) return
      const frame = assembler.push(event.data)
      if (frame) onFrame(frame)
    }
    control = peer.createDataChannel('control')
    pointer = peer.createDataChannel('pointer', POINTER_CHANNEL)
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

  /** Envoie un message à l'agent. Les déplacements de souris prennent le canal rapide non fiable quand l'agent le gère ; tout le reste (clics, touches,
   *  commandes) passe par le canal fiable et ordonné. */
  function sendInput(message) {
    const text = JSON.stringify(message)
    if (message.t === 'move' && pointerEnabled && pointer?.readyState === 'open') { pointer.send(text); return }
    if (control?.readyState === 'open') control.send(text)
  }

  const usePointerChannel = (enabled) => { pointerEnabled = !!enabled }

  async function close() {
    if (closed) return
    closed = true
    generation += 1
    pc?.close()
    try { await request.api(`/sessions/${sessionId}`, { method: 'DELETE' }) } catch { /* déjà terminée côté serveur */ }
    onState('closed')
  }

  return { sessionId, sendInput, usePointerChannel, close }
}
