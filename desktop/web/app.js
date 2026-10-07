// Interface d'ARIA Remote Desktop. Tout passe par les commandes Rust (clé du contrôleur, appels réseau) : la page ne voit jamais la clé.
import { createRequest, keyMessage, openSession, pointerMessage, wheelMessage } from './session.js'

const $ = (id) => document.getElementById(id)
const invoke = window.__TAURI__?.core?.invoke
const request = invoke ? createRequest(invoke) : null
const NS = 'http://www.w3.org/2000/svg'

const STATE_TEXT = {
  asking: 'Acceptation sur l\'appareil…',
  connecting: 'Connexion en cours…',
  connected: 'Connecté',
  reconnecting: 'Connexion perdue, reconnexion…',
  lost: 'Connexion perdue',
  closed: 'Session terminée',
}
const PERMISSIONS = [
  { id: 'control_mouse', label: 'Souris', icon: 'i-mouse' },
  { id: 'control_keyboard', label: 'Clavier', icon: 'i-keyboard' },
]
const MOVE_INTERVAL_MS = 30

let settings = { server: '', configured: false }
let editing = false
let devices = []
let timer = null
const off = new Map()        // appareil -> permissions désactivées par la personne (tout est activé par défaut)
let connecting = null        // { id, state } pendant l'attente d'acceptation
let live = null              // { link, device, permissions }

const show = (el, on) => { el.hidden = !on }
const message = (error) => (typeof error === 'string' ? error : error?.message || 'Erreur inattendue.')
const showError = (el, text) => { el.textContent = text || ''; show(el, !!text) }

function icon(id, extra = '') {
  const svg = document.createElementNS(NS, 'svg')
  svg.setAttribute('class', `i ${extra}`.trim())
  svg.setAttribute('aria-hidden', 'true')
  const use = document.createElementNS(NS, 'use')
  use.setAttribute('href', `#${id}`)
  svg.append(use)
  return svg
}

function el(tag, className, text) {
  const node = document.createElement(tag)
  if (className) node.className = className
  if (text !== undefined) node.textContent = text
  return node
}

function pill(tone, text) {
  show($('pill'), !!text)
  $('pill').className = 'pill ' + (tone || '')
  $('pill-text').textContent = text || ''
}

// --- Affichage général ----------------------------------------------------------------------------------------------

function render() {
  const needSetup = !settings.configured || editing
  show($('setup'), needSetup)
  show($('home'), !needSetup && !live)
  show($('viewer'), !needSetup && !!live)
  show($('open-settings'), settings.configured && !editing && !live)
  show($('cancel-settings'), settings.configured)
  $('key-opt').textContent = settings.configured ? '(laisse vide pour garder la clé enregistrée)' : ''
  if (needSetup) { pill(null, ''); stopPolling() } else startPolling()
  if (!needSetup && !live) renderDevices()
}

// --- Appareils --------------------------------------------------------------------------------------------------------

function chosenPermissions(device) {
  const skipped = off.get(device.device_id) || new Set()
  return PERMISSIONS.filter((p) => (device.granted || []).includes(p.id) && !skipped.has(p.id))
}

function deviceRow(device) {
  const li = el('li', 'device')
  const info = el('div')
  info.append(el('div', 'name', device.name || device.device_id))
  const meta = el('div', 'meta' + (device.online ? ' on' : ''))
  meta.append(el('span', 'dot'), el('span', '', device.online ? 'Disponible' : 'Hors ligne'))
  if (device.fingerprint) { meta.append(el('span', 'sep', '·'), el('span', 'fp', device.fingerprint.slice(0, 8))) }
  info.append(meta)

  const caps = el('div', 'caps')
  const waiting = connecting && connecting.id === device.device_id
  if (waiting) {
    const wait = el('span', 'wait')
    wait.setAttribute('role', 'status')
    wait.append(icon('i-spin', 'spin'), el('span', '', STATE_TEXT[connecting.state] || connecting.state))
    caps.append(wait)
  } else {
    const optional = PERMISSIONS.filter((p) => (device.granted || []).includes(p.id))
    for (const permission of optional) {
      const on = chosenPermissions(device).some((p) => p.id === permission.id)
      const button = el('button', 'cap')
      button.type = 'button'
      button.setAttribute('aria-pressed', String(on))
      button.setAttribute('aria-label', permission.label)
      button.title = `${permission.label} : ${on ? 'activée' : 'désactivée'}`
      button.disabled = !!connecting
      button.append(icon(permission.icon))
      button.onclick = () => {
        const skipped = off.get(device.device_id) || new Set()
        if (skipped.has(permission.id)) skipped.delete(permission.id); else skipped.add(permission.id)
        off.set(device.device_id, skipped)
        renderDevices()
      }
      caps.append(button)
    }
    if (optional.length === 0) {
      const view = el('span', 'viewonly')
      view.title = 'Cet appareil n\'autorise que la consultation de l\'écran'
      view.append(icon('i-eye'), 'Vue seule')
      caps.append(view)
    }
  }

  const connect = el('button', 'connect')
  connect.type = 'button'
  connect.setAttribute('aria-label', `Se connecter à ${device.name}`)
  connect.disabled = !!connecting || device.paired === false || !device.online
  connect.title = device.online ? 'Se connecter' : 'Cet appareil est hors ligne'
  connect.append(icon('i-play'), 'Se connecter')
  connect.onclick = () => connectTo(device)

  li.append(info, caps, connect)
  return li
}

function renderDevices() {
  const paired = devices.filter((d) => d.paired !== false)
  $('devices').replaceChildren(...paired.map(deviceRow))
  show($('empty'), paired.length === 0)
}

async function refresh() {
  if (!settings.configured || editing || live) return
  try {
    const result = await request.api('/devices')
    devices = result.devices || []
    if (!connecting) renderDevices()
    showError($('list-err'), '')
    pill('ok', 'Connecté au serveur')
  } catch (error) {
    showError($('list-err'), message(error))
    pill('bad', 'Serveur injoignable')
  }
}

function startPolling() { if (!timer) { refresh(); timer = setInterval(() => { if (document.visibilityState === 'visible') refresh() }, 4000) } }
function stopPolling() { clearInterval(timer); timer = null }

// --- Réglages ---------------------------------------------------------------------------------------------------------

async function save() {
  showError($('setup-err'), '')
  $('save').disabled = true; $('save').textContent = 'Connexion…'
  try {
    settings = await invoke('save_settings', { server: $('srv').value, key: $('key').value })
    $('key').value = ''; editing = false
    render()
  } catch (error) {
    showError($('setup-err'), message(error))
  } finally {
    $('save').disabled = false; $('save').textContent = 'Se connecter'
  }
}

// --- Appairage --------------------------------------------------------------------------------------------------------

const pairingDigits = (value) => value.replace(/\D/g, '').slice(0, 6)

$('code').addEventListener('input', () => {
  const digits = pairingDigits($('code').value)
  $('code').value = digits
  $('pair-btn').disabled = digits.length !== 6
})

$('pair-form').addEventListener('submit', async (event) => {
  event.preventDefault()
  const code = pairingDigits($('code').value)
  if (code.length !== 6) return
  showError($('pair-err'), ''); show($('pair-ok'), false)
  $('pair-btn').disabled = true
  try {
    const { device } = await request.post('/pair', { code })
    $('code').value = ''
    $('pair-ok').textContent = `${device?.name || 'Appareil'} est appairé.`; show($('pair-ok'), true)
    refresh()
  } catch (error) {
    showError($('pair-err'), message(error))
    $('pair-btn').disabled = false
  }
})

// --- Prise de contrôle ----------------------------------------------------------------------------------------------

async function connectTo(device) {
  const permissions = ['view_screen', ...chosenPermissions(device).map((p) => p.id)]
  connecting = { id: device.device_id, state: 'asking' }
  showError($('list-err'), '')
  renderDevices()
  try {
    const link = await openSession({
      deviceId: device.device_id, permissions, request,
      onState: (state) => { if (connecting) { connecting.state = state; renderDevices() } else { viewerState(state) } },
      onInfo: viewerInfo, onFrame: drawFrame, onCursor: moveCursor,
    })
    live = { link, device, permissions }
    connecting = null
    openViewer()
  } catch (error) {
    connecting = null
    showError($('list-err'), message(error))
    renderDevices()
  }
}

function viewerState(state) {
  const text = STATE_TEXT[state] || state
  $('v-state').textContent = text
  $('v-state').className = 'status' + (state === 'connected' ? ' good' : '')
  pill(state === 'connected' ? 'ok' : state === 'lost' ? 'bad' : 'warn', text)
}

function viewerInfo(info) {
  if (!info?.width || !info?.height) return
  $('stage').style.setProperty('--ratio', (info.width / info.height).toFixed(4))
  $('v-state').dataset.size = `${info.width}×${info.height}`
}

async function drawFrame(data) {
  const canvas = $('screen')
  const bitmap = await createImageBitmap(new Blob([data], { type: 'image/jpeg' }))
  if (canvas.width !== bitmap.width || canvas.height !== bitmap.height) { canvas.width = bitmap.width; canvas.height = bitmap.height }
  canvas.getContext('2d').drawImage(bitmap, 0, 0)
  bitmap.close?.()
}

function moveCursor({ x, y }) {
  const pointer = $('cursor')
  if (!Number.isFinite(x) || !Number.isFinite(y)) return
  pointer.style.left = `${Math.min(1, Math.max(0, x)) * 100}%`
  pointer.style.top = `${Math.min(1, Math.max(0, y)) * 100}%`
  pointer.style.opacity = '1'
}

function openViewer() {
  const { device, permissions } = live
  $('v-name').textContent = device.name
  $('cursor').style.opacity = '0'
  const caps = $('v-caps')
  caps.replaceChildren(...PERMISSIONS.map((p) => {
    const on = permissions.includes(p.id)
    const badge = el('span', 'badge' + (on ? ' on' : ''))
    badge.title = `${p.label} : ${on ? 'oui' : 'non'}`
    badge.append(icon(p.icon), p.label)
    return badge
  }))
  $('v-hint').textContent = permissions.includes('control_keyboard') ? 'Clique sur l\'écran pour envoyer le clavier à l\'appareil.' : ''
  viewerState('connected')
  render()
  $('screen').focus()
}

async function disconnect() {
  const current = live
  live = null
  pill(null, '')
  await current?.link.close()
  render()
  refresh()
}

// Entrées envoyées à l'appareil (seulement ce qui a été autorisé).
let lastMove = 0
const canvas = $('screen')
const send = (message_) => live?.link.sendInput(message_)
const can = (permission) => !!live?.permissions.includes(permission)
canvas.addEventListener('pointermove', (event) => {
  if (!can('control_mouse')) return
  const now = performance.now()
  if (now - lastMove < MOVE_INTERVAL_MS) return
  lastMove = now
  send(pointerMessage('move', event, canvas))
})
canvas.addEventListener('pointerdown', (event) => {
  canvas.focus()
  if (!can('control_mouse')) return
  canvas.setPointerCapture?.(event.pointerId)
  send(pointerMessage('down', event, canvas))
})
canvas.addEventListener('pointerup', (event) => { if (can('control_mouse')) send(pointerMessage('up', event, canvas)) })
canvas.addEventListener('wheel', (event) => { if (can('control_mouse')) { event.preventDefault(); send(wheelMessage(event)) } }, { passive: false })
canvas.addEventListener('contextmenu', (event) => event.preventDefault())
canvas.addEventListener('keydown', (event) => { if (can('control_keyboard')) { event.preventDefault(); send(keyMessage(event, true)) } })
canvas.addEventListener('keyup', (event) => { if (can('control_keyboard')) { event.preventDefault(); send(keyMessage(event, false)) } })

// Fermer la fenêtre coupe la session : on ne laisse jamais un contrôle ouvert sans écran.
window.addEventListener('beforeunload', () => { live?.link.close() })

// --- Démarrage --------------------------------------------------------------------------------------------------------

$('save').addEventListener('click', save)
$('srv').addEventListener('keydown', (e) => { if (e.key === 'Enter') save() })
$('key').addEventListener('keydown', (e) => { if (e.key === 'Enter') save() })
$('open-settings').addEventListener('click', () => { editing = true; $('srv').value = settings.server; showError($('setup-err'), ''); render() })
$('cancel-settings').addEventListener('click', () => { editing = false; render() })
$('disconnect').addEventListener('click', disconnect)

async function start() {
  if (!invoke) {
    // Aperçu dans un navigateur : l'API de la fenêtre n'existe pas.
    showError($('setup-err'), 'Ouvre ce programme avec « npm run dev » : la connexion au serveur n\'est pas disponible dans un navigateur.')
    show($('setup'), true); $('save').disabled = true
    return
  }
  invoke('app_version').then((v) => { $('ver').textContent = 'v' + v; show($('ver'), true) }).catch(() => {})
  settings = await invoke('load_settings')
  $('srv').value = settings.server
  render()
}
start()
