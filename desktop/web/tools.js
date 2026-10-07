// Logique des outils de la session (latence, images par seconde, qualité automatique, raccourcis, presse-papiers) : sans écran ni réseau,
// donc testable à part (tests/tools.test.mjs).

// --- Qualité de l'image -----------------------------------------------------------------------------------------------
// Mêmes bornes que l'agent (session.rs) : il les applique de toute façon, ces valeurs ne servent qu'à ne pas être rognées.
export const PRESETS = {
  economy: { id: 'economy', label: 'Économie', hint: 'connexion lente', fps: 5, quality: 35, width: 1024 },
  balanced: { id: 'balanced', label: 'Équilibré', hint: 'par défaut', fps: 10, quality: 55, width: 1600 },
  high: { id: 'high', label: 'Haute qualité', hint: 'réseau local ou fibre', fps: 15, quality: 75, width: 2560 },
}
export const PRESET_ORDER = ['economy', 'balanced', 'high']

export const streamMessage = (preset) => ({ t: 'stream', fps: preset.fps, quality: preset.quality, width: preset.width })

/**
 * Qualité automatique : baisse vite quand la connexion peine, remonte lentement quand elle est à l'aise (hystérésis : trois mauvais échantillons
 * de suite pour baisser, six bons pour remonter), pour ne jamais osciller.
 *
 * Un échantillon = { rtt (ms ou null), fps (images réellement envoyées par seconde), dropped (images que l'agent a dû abandonner faute de place
 * dans le canal d'envoi), idle (écran fixe : rien à envoyer) }. « Écran fixe » n'est PAS un signe de connexion lente : sans cette précision, un écran
 * qui ne bouge pas faisait baisser la qualité (peu d'images reçues). Les images abandonnées sont le signal le plus fiable : l'agent les jette quand
 * le réseau ne suit pas le débit demandé.
 */
export class AutoQuality {
  constructor(start = 'balanced') { this.level = PRESET_ORDER.indexOf(start); if (this.level < 0) this.level = 1; this.bad = 0; this.good = 0 }

  get preset() { return PRESETS[PRESET_ORDER[this.level]] }

  /** Renvoie le nouveau préréglage s'il change, sinon null. */
  sample({ rtt, fps = 0, dropped = 0, idle = false }) {
    const target = this.preset.fps
    const slow = (rtt !== null && rtt > 250) || dropped >= 2 || (!idle && fps < target * 0.5)
    const comfortable = (rtt === null || rtt < 100) && dropped === 0 && (idle || fps >= target * 0.85)
    this.bad = slow ? this.bad + 1 : 0
    this.good = comfortable ? this.good + 1 : 0
    if (this.bad >= 3 && this.level > 0) { this.level -= 1; this.bad = 0; this.good = 0; return this.preset }
    if (this.good >= 6 && this.level < PRESET_ORDER.length - 1) { this.level += 1; this.bad = 0; this.good = 0; return this.preset }
    return null
  }
}

// --- Latence et images par seconde ------------------------------------------------------------------------------------
export class LatencyMeter {
  constructor() { this.next = 1; this.sent = new Map(); this.samples = [] }

  /** Message ping à envoyer ; les ping restés sans réponse sont oubliés (au plus dix en attente). */
  ping(now) {
    const id = this.next++
    this.sent.set(id, now)
    if (this.sent.size > 10) this.sent.delete(this.sent.keys().next().value)
    return { t: 'ping', id }
  }

  /** Réponse de l'agent : renvoie le temps d'aller-retour de CE ping (ms), null si on ne le connaît pas. */
  pong(id, now) {
    const at = this.sent.get(id)
    if (at === undefined) return null
    this.sent.delete(id)
    const rtt = Math.max(0, Math.round(now - at))
    this.samples.push(rtt)
    if (this.samples.length > 5) this.samples.shift()
    return rtt
  }

  /** Moyenne des cinq dernières mesures (lisse les à-coups) ; null tant qu'il n'y en a pas. */
  get average() { return this.samples.length ? Math.round(this.samples.reduce((a, b) => a + b, 0) / this.samples.length) : null }
}

export class FpsMeter {
  constructor() { this.frames = 0; this.since = null }

  hit() { this.frames += 1 }

  /** Images par seconde depuis le dernier appel (une décimale) ; null au premier appel (rien à comparer). */
  tick(now) {
    const previous = this.since
    this.since = now
    const count = this.frames
    this.frames = 0
    if (previous === null || now <= previous) return null
    return Math.round((count * 10_000) / (now - previous)) / 10
  }
}

export function latencyTone(ms) {
  if (ms === null || ms === undefined) return 'idle'
  return ms < 80 ? 'ok' : ms < 200 ? 'warn' : 'bad'
}

/** Texte de l'infobulle des statistiques (ce que l'agent mesure de son côté + la latence mesurée ici). */
export function statsTitle({ rtt, agent }) {
  const parts = []
  if (rtt !== null && rtt !== undefined) parts.push(`latence ${rtt} ms`)
  if (agent) {
    parts.push(agent.idle ? 'écran fixe (rien à envoyer)' : `${agent.fps} images/s envoyées`)
    parts.push(`${agent.kbps} kb/s`)
    parts.push(`capture ${agent.capture_ms} ms`)
    parts.push(`encodage ${agent.encode_ms} ms`)
    if (agent.dropped > 0) parts.push(`${agent.dropped} abandonnée${agent.dropped > 1 ? 's' : ''} (réseau trop lent pour ce réglage)`)
  }
  return parts.length ? parts.join(' · ') : 'Latence et images par seconde'
}

// --- Raccourcis clavier -----------------------------------------------------------------------------------------------
// Touches envoyées à l'appareil (mêmes codes que le navigateur : voir keyboard.rs côté agent). Ctrl+Alt+Suppr est réservé à Windows et ne peut pas être envoyé.
const press = (code, key) => [{ down: true, code, key }]
const release = (code, key) => [{ down: false, code, key }]
const tap = (code, key) => [...press(code, key), ...release(code, key)]
const chord = (modifiers, code, key) => [...modifiers.flatMap(([c, k]) => press(c, k)), ...tap(code, key), ...[...modifiers].reverse().flatMap(([c, k]) => release(c, k))]
const ALT = ['AltLeft', 'Alt']
const CTRL = ['ControlLeft', 'Control']
const SHIFT = ['ShiftLeft', 'Shift']

export const SHORTCUTS = [
  { id: 'win', label: 'Touche Windows', steps: tap('MetaLeft', 'Meta') },
  { id: 'alttab', label: 'Alt + Tab', steps: chord([ALT], 'Tab', 'Tab') },
  { id: 'taskmgr', label: 'Ctrl + Maj + Échap', hint: 'gestionnaire des tâches', steps: chord([CTRL, SHIFT], 'Escape', 'Escape') },
  { id: 'altf4', label: 'Alt + F4', hint: 'fermer la fenêtre', steps: chord([ALT], 'F4', 'F4') },
]

export const shortcutMessages = (id) => (SHORTCUTS.find((s) => s.id === id)?.steps || []).map((step) => ({ t: 'key', ...step }))

/** Touches encore enfoncées à la fin de `messages` : à relâcher si l'envoi est interrompu (sinon une touche resterait bloquée à distance). */
export function stuckKeys(messages) {
  const down = new Map()
  for (const m of messages) { if (m.down) down.set(m.code, m); else down.delete(m.code) }
  return [...down.values()].reverse().map((m) => ({ ...m, down: false }))
}

// --- Presse-papiers ---------------------------------------------------------------------------------------------------
export const CLIPBOARD_MAX_BYTES = 256 * 1024
const byteLength = (text) => new TextEncoder().encode(text).length

/** Même règle que l'agent : on n'envoie que ce qui CHANGE après l'ouverture, jamais ce qui vient d'être reçu (pas d'écho), jamais l'excès. */
export class ClipboardSync {
  constructor(initial = null) { this.last = initial }

  /** Contenu actuel du presse-papiers local : renvoie le texte à envoyer, sinon null. */
  changed(current) {
    if (typeof current !== 'string' || current === this.last) return null
    this.last = current
    return current.length > 0 && byteLength(current) <= CLIPBOARD_MAX_BYTES ? current : null
  }

  /** Texte venu de l'appareil : renvoie le texte à écrire localement (et le retient pour ne pas le renvoyer), null s'il est refusé. */
  received(text) {
    if (typeof text !== 'string' || text.length === 0 || byteLength(text) > CLIPBOARD_MAX_BYTES) return null
    this.last = text
    return text
  }
}

// --- Écrans -----------------------------------------------------------------------------------------------------------
export function screenLabel(screen, count) {
  const name = count > 1 ? `Écran ${screen.index + 1}` : 'Écran'
  return `${name}${screen.primary ? ' (principal)' : ''} · ${screen.width}×${screen.height}`
}
