// Logique de « Cet appareil » (l'agent intégré) : modes, écran à montrer, code d'appairage. Sans écran ni réseau, donc testable à part
// (tests/host.test.mjs).

export const MODES = [
  { id: 'control', icon: 'i-monitor', label: 'Contrôler', hint: 'Prendre le contrôle d\'autres PC.' },
  { id: 'host', icon: 'i-shield', label: 'Être contrôlé', hint: 'Laisser quelqu\'un prendre le contrôle de ce PC. Le programme reste près de l\'horloge.' },
  { id: 'both', icon: 'i-swap', label: 'Les deux', hint: 'Contrôler d\'autres PC et pouvoir être contrôlé.' },
]

export const canControl = (mode) => mode !== 'host'
export const canHost = (mode) => mode === 'host' || mode === 'both'
/** Les deux onglets (Contrôler | Cet appareil) n'existent que si ce PC fait les deux. */
export const hasTabs = (mode) => mode === 'both'

/** Onglet à montrer : celui demandé s'il existe dans ce mode, sinon celui que le mode permet. */
export function activeTab(mode, wanted) {
  if (!canHost(mode)) return 'control'
  if (!canControl(mode)) return 'host'
  return wanted === 'host' ? 'host' : 'control'
}

/** « 483921 » -> « 483 921 » (deux groupes lisibles d'un coup d'œil). */
export const formatCode = (code) => (typeof code === 'string' && /^\d{6}$/.test(code) ? `${code.slice(0, 3)} ${code.slice(3)}` : code || '')

/** Secondes avant l'expiration du code d'appairage (0 s'il a expiré ou s'il n'y en a pas). */
export const secondsLeft = (expiresMs, now) => (Number.isFinite(expiresMs) ? Math.max(0, Math.ceil((expiresMs - now) / 1000)) : 0)

export function clockText(seconds) {
  const minutes = Math.floor(seconds / 60)
  return `${minutes}:${String(seconds % 60).padStart(2, '0')}`
}

/**
 * Ce que « Cet appareil » montre d'après l'état de l'agent (null = l'agent ne tourne pas) :
 * off | setup (adresse et clé à saisir) | pairing (code à donner) | ready | error | connecting.
 */
export function hostScreen(state, now = Date.now()) {
  if (!state) return 'off'
  if (state.link === 'setup') return 'setup'
  const codeLive = !!state.pairing_code && secondsLeft(state.pairing_expires_ms, now) > 0
  if (state.pairing_code && (state.link === 'unpaired' || state.link === 'connecting' || (state.link === 'online' && codeLive))) return 'pairing'
  if (state.link === 'online') return 'ready'
  if (state.link === 'error') return 'error'
  return 'connecting'
}

const PERMISSION_TEXT = { view_screen: 'voir l\'écran', control_mouse: 'utiliser la souris', control_keyboard: 'utiliser le clavier' }

/** « voir l'écran, utiliser la souris » : ce que la personne qui demande veut faire, en clair. */
export const permissionText = (permissions) => (permissions || []).map((p) => PERMISSION_TEXT[p] || p).join(', ')

const LINK = { connecting: ['warn', 'Connexion…'], unpaired: ['warn', 'À appairer'], online: ['ok', 'Disponible'], error: ['bad', 'Hors ligne'], setup: ['warn', 'À configurer'] }

/** Pastille de l'en-tête, visible sur tous les onglets : [ton, texte] ; null si l'agent ne tourne pas. */
export function hostBadge(state) {
  if (!state) return null
  if (state.session) return ['bad', 'Contrôlé']
  const [tone, text] = LINK[state.link] || ['warn', '…']
  return [tone, state.link === 'unpaired' && state.pairing_code ? `Code ${formatCode(state.pairing_code)}` : text]
}

export const CONSENT_SECONDS = 60      // sans réponse, l'agent refuse (agent.rs : CONSENT_TIMEOUT)
export const consentSecondsLeft = (sinceMs, nowMs) => Math.max(0, CONSENT_SECONDS - Math.round((nowMs - sinceMs) / 1000))
