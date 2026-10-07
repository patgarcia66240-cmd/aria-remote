// Interface d'ARIA Remote Desktop. Tout passe par les commandes Rust (clé du contrôleur, appels réseau) : la page ne voit jamais la clé.
import { KINDS, guessKind, loadKinds, machineSvg, saveKind } from './machines.js'
import { createRequest, keyMessage, openSession, pointerMessage, wheelMessage } from './session.js'
import { AutoQuality, ClipboardSync, FpsMeter, LatencyMeter, PRESETS, PRESET_ORDER, SHORTCUTS, latencyTone, screenLabel, shortcutMessages, statsTitle, streamMessage, stuckKeys } from './tools.js'

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
let tools = null             // outils de la session en cours (mesures, qualité, presse-papiers)
let selectedId = null        // appareil montré dans l'aperçu du bas de la carte
const kinds = loadKinds()    // type de machine choisi par appareil (bureau, mini PC, portable)

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
  // En session, l'en-tête porte tout : appareil, état, autorisations et « Déconnecter » (pas de second bandeau).
  const inSession = !needSetup && !!live
  document.body.classList.toggle('live', inSession)
  show($('sub-idle'), !inSession); show($('sub-live'), inSession)
  show($('v-caps'), inSession); show($('v-tools'), inSession); show($('disconnect'), inSession)
  if (inSession) pill(null, '')
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
  const li = el('li', 'device' + (device.device_id === selectedId ? ' selected' : ''))
  li.onclick = (event) => { if (!event.target.closest('button')) { selectedId = device.device_id; renderDevices() } }
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
  if (!paired.some((d) => d.device_id === selectedId)) selectedId = paired[0]?.device_id ?? null
  $('devices').replaceChildren(...paired.map(deviceRow))
  show($('empty'), paired.length === 0)
  renderPreview(paired.find((d) => d.device_id === selectedId))
}

// Aperçu de l'appareil sélectionné : dessin de sa machine (écran allumé s'il est en ligne) et choix du type.
function renderPreview(device) {
  show($('preview'), !!device)
  if (!device) return
  const kind = kinds[device.device_id] || guessKind(device)
  $('art').innerHTML = machineSvg(kind, device.online)
  $('p-name').textContent = device.name || device.device_id
  const state = $('p-state')
  state.className = 'pstate' + (device.online ? ' on' : '')
  state.replaceChildren(el('span', 'dot'), el('span', '', `${KINDS.find((k) => k.id === kind).title} · ${device.online ? 'Disponible' : 'Hors ligne'}`))
  $('p-kind').replaceChildren(...KINDS.map((k) => {
    const button = el('button', '', k.label)
    button.type = 'button'
    button.setAttribute('role', 'radio')
    button.setAttribute('aria-checked', String(k.id === kind))
    button.onclick = () => { kinds[device.device_id] = k.id; saveKind(device.device_id, k.id); renderPreview(device) }
    return button
  }))
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
  tools = makeTools()
  showError($('list-err'), '')
  renderDevices()
  try {
    const link = await openSession({
      deviceId: device.device_id, permissions, request,
      onState: (state) => { if (connecting) { connecting.state = state; renderDevices() } else { viewerState(state) } },
      onInfo: onControl, onFrame: drawFrame, onCursor: moveCursor,
    })
    live = { link, device, permissions }
    connecting = null
    openViewer()
  } catch (error) {
    connecting = null
    tools = null
    showError($('list-err'), message(error))
    renderDevices()
  }
}

function viewerState(state) {
  const text = STATE_TEXT[state] || state
  $('v-state').textContent = text
  $('v-state').className = 'status' + (state === 'connected' ? ' good' : state === 'lost' ? ' bad' : '')
}

// --- Outils de la session -------------------------------------------------------------------------------------------------

const pref = (key, fallback) => { try { return localStorage.getItem('aria-remote-' + key) ?? fallback } catch { return fallback } }
const setPref = (key, value) => { try { localStorage.setItem('aria-remote-' + key, value) } catch { /* stockage indisponible : le choix vaut pour cette session */ } }
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function makeTools() {
  return {
    features: new Set(), screens: [], screen: 0, infoSeen: false, started: false, timers: [], clipTimer: null, sync: null, lastToast: 0,
    latency: new LatencyMeter(), fps: new FpsMeter(), lastFps: null, agent: null, freshAgent: false,
    quality: pref('quality', 'auto'), auto: new AutoQuality('balanced'),
    clipOn: pref('clip', '1') === '1',
  }
}

let toastTimer = null
function toast(text, tone = '') {
  const node = $('toast')
  node.textContent = text
  node.className = 'toast' + (tone ? ` ${tone}` : '')
  show(node, true)
  clearTimeout(toastTimer)
  toastTimer = setTimeout(() => show(node, false), tone === 'bad' ? 6000 : 3500)
}

/** Messages de l'agent sur le canal « control » (tout sauf la position du curseur). */
function onControl(msg) {
  if (!tools || !msg) return
  switch (msg.t) {
    case 'info': return applyInfo(msg)
    case 'pong': return onPong(msg.id)
    case 'stats': tools.agent = msg; tools.freshAgent = true; return updateStats()
    case 'clip': return receiveClipboard(msg.text)
    case 'denied': {
      if (Date.now() - tools.lastToast > 4000) { tools.lastToast = Date.now(); toast(`L'appareil a refusé : ${msg.reason || 'commande non autorisée'}`, 'bad') }
      return undefined
    }
    default: return undefined
  }
}

function applyInfo(info) {
  if (info.width && info.height) {
    $('stage').style.setProperty('--ratio', (info.width / info.height).toFixed(4))
    $('v-size').textContent = `${info.width}×${info.height}`
  }
  tools.features = new Set(info.features || [])
  tools.screens = info.screens || []
  tools.screen = info.screen ?? 0
  tools.infoSeen = true
  refreshToolbar()
  startTools()
  syncPointerChannel()
}

/** Les déplacements de souris prennent le canal rapide non fiable quand cet agent le gère (un ancien agent l'ignore). */
function syncPointerChannel() {
  if (live && tools?.infoSeen) live.link.usePointerChannel(tools.features.has('pointer'))
}

function refreshToolbar() {
  if (!live || !tools) return
  const keyboard = live.permissions.includes('control_keyboard')
  show($('t-quality'), tools.features.has('stream'))
  show($('t-screens'), tools.features.has('screens') && tools.screens.length > 1)
  show($('t-shortcuts'), keyboard)
  show($('t-clip'), keyboard && tools.features.has('clipboard'))
  $('t-clip').setAttribute('aria-pressed', String(tools.clipOn))
  $('t-clip').title = tools.clipOn ? 'Presse-papiers partagé : activé' : 'Presse-papiers partagé : désactivé'
  const preset = currentPreset()
  $('t-quality').title = `Qualité de l'image : ${tools.quality === 'auto' ? `auto (${preset.label})` : preset.label}`
}

const currentPreset = () => (tools.quality === 'auto' ? tools.auto.preset : PRESETS[tools.quality] || PRESETS.balanced)
const sendToDevice = (message_) => live?.link.sendInput(message_)

/** Démarre les mesures et les échanges réguliers, une fois la session ouverte ET les fonctions de l'agent connues. */
function startTools() {
  if (!tools || tools.started || !live || !tools.infoSeen) return
  tools.started = true
  const every = (ms, fn) => tools.timers.push(setInterval(fn, ms))
  applyQuality()
  every(1000, tickFps)
  if (tools.features.has('ping')) { pingNow(); every(2000, pingNow) }
  if (tools.features.has('stream')) every(3000, autoSample)
  startClipboard()
  refreshToolbar()
  syncPointerChannel()
}

function stopTools() {
  tools?.timers.forEach(clearInterval)
  clearInterval(tools?.clipTimer)
  tools = null
  show($('v-stats'), false)
  closeMenu()
}

function pingNow() { sendToDevice(tools.latency.ping(performance.now())) }

function onPong(id) {
  if (tools.latency.pong(id, performance.now()) !== null) updateStats()
}

function tickFps() {
  const value = tools.fps.tick(performance.now())
  if (value !== null) tools.lastFps = value
  updateStats()
}

/** Affichage : la latence mesurée ici, et les images/s ENVOYÉES par l'agent (plus justes que celles reçues : un écran fixe n'en envoie pas). */
function updateStats() {
  const average = tools.latency.average
  const agent = tools.agent
  $('v-lat').textContent = average === null ? '' : `${average} ms`
  $('v-fps').textContent = agent ? (agent.idle ? 'écran fixe' : `${agent.fps} i/s`) : tools.lastFps === null ? '' : `${tools.lastFps} i/s`
  $('v-stats').dataset.tone = latencyTone(average)
  $('v-stats').title = statsTitle({ rtt: average, agent })
  show($('v-stats'), average !== null || tools.lastFps !== null || !!agent)
}

function applyQuality() {
  if (tools.features.has('stream')) sendToDevice(streamMessage(currentPreset()))
  refreshToolbar()
}

function autoSample() {
  if (tools.quality !== 'auto') return
  const agent = tools.freshAgent ? tools.agent : null          // chaque rapport de l'agent ne compte qu'une fois
  if (!agent && tools.lastFps === null) return
  tools.freshAgent = false
  const next = tools.auto.sample({ rtt: tools.latency.average, fps: agent ? agent.fps : tools.lastFps, dropped: agent ? agent.dropped : 0, idle: agent ? agent.idle : false })
  if (next) { applyQuality(); toast(`Qualité ajustée automatiquement : ${next.label}`) }
}

// Presse-papiers partagé (texte) : ce qui est copié d'un côté devient collable de l'autre, tant que l'icône est activée.
async function startClipboard() {
  clearInterval(tools.clipTimer)
  if (!tools.clipOn || !live.permissions.includes('control_keyboard') || !tools.features.has('clipboard')) return
  const mine = tools
  mine.sync = new ClipboardSync(await invoke('clipboard_read').catch(() => null))
  if (tools !== mine) return
  mine.clipTimer = setInterval(async () => {
    const text = mine.sync.changed(await invoke('clipboard_read').catch(() => null))
    if (text && tools === mine) { sendToDevice({ t: 'clip', text }); toast('Texte copié envoyé à l\'appareil') }
  }, 1000)
}

function receiveClipboard(text) {
  if (!tools.clipOn || !tools.sync) return
  const accepted = tools.sync.received(text)
  if (accepted) invoke('clipboard_write', { text: accepted }).then(() => toast('Texte copié reçu de l\'appareil')).catch(() => {})
}

async function sendShortcut(id) {
  const sent = []
  try {
    for (const key of shortcutMessages(id)) { sendToDevice(key); sent.push(key); await sleep(25) }
  } finally {
    for (const key of stuckKeys(sent)) sendToDevice(key)       // jamais de touche restée enfoncée à distance
  }
}

async function capture() {
  const canvas = $('screen')
  if (!live || !canvas.width || !invoke) return
  try {
    const blob = await new Promise((resolve, reject) => canvas.toBlob((b) => (b ? resolve(b) : reject(new Error('image vide'))), 'image/png'))
    const dataUrl = await new Promise((resolve, reject) => { const reader = new FileReader(); reader.onload = () => resolve(reader.result); reader.onerror = reject; reader.readAsDataURL(blob) })
    const path = await invoke('save_capture', { pngBase64: String(dataUrl).split(',')[1], device: live.device.name })
    toast(`Capture enregistrée : ${path}`)
  } catch (error) {
    toast(`Capture impossible : ${message(error)}`, 'bad')
  }
}

// --- Menus et plein écran -----------------------------------------------------------------------------------------------

let menu = null
function closeMenu() {
  if (!menu) return
  menu.node.remove()
  menu.anchor.setAttribute('aria-expanded', 'false')
  document.removeEventListener('pointerdown', menu.outside, true)
  document.removeEventListener('keydown', menu.key, true)
  menu = null
}

/** Menu sous un bouton de l'en-tête. items : { head } | { note } | { label, hint, checked, run }. */
function openMenu(anchor, items) {
  const wasOpen = menu?.anchor === anchor
  closeMenu()
  if (wasOpen) return
  const node = el('div', 'menu')
  node.setAttribute('role', 'menu')
  for (const item of items) {
    if (item.head) { node.append(el('div', 'menu-head', item.head)); continue }
    if (item.note) { node.append(el('div', 'menu-note', item.note)); continue }
    const row = el('button', 'menu-item')
    row.type = 'button'
    row.setAttribute('role', item.checked === undefined ? 'menuitem' : 'menuitemradio')
    if (item.checked !== undefined) { row.setAttribute('aria-checked', String(item.checked)); row.append(icon('i-check', 'tick')) }
    const text = el('span', 'txt')
    text.append(el('span', '', item.label))
    if (item.hint) text.append(el('span', 'hint', item.hint))
    row.append(text)
    row.onclick = () => { closeMenu(); canvas.focus(); item.run() }
    node.append(row)
  }
  document.body.append(node)
  const box = anchor.getBoundingClientRect()
  node.style.top = `${box.bottom + 8}px`
  node.style.right = `${Math.max(8, window.innerWidth - box.right)}px`
  anchor.setAttribute('aria-expanded', 'true')
  menu = {
    node, anchor,
    outside: (event) => { if (!node.contains(event.target) && !anchor.contains(event.target)) closeMenu() },
    key: (event) => { if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); closeMenu(); canvas.focus() } },
  }
  document.addEventListener('pointerdown', menu.outside, true)
  document.addEventListener('keydown', menu.key, true)
  node.querySelector('button')?.focus()
}

let fullscreen = false
async function setFullscreen(on) {
  try {
    const window_ = window.__TAURI__?.window?.getCurrentWindow?.()
    if (window_) await window_.setFullscreen(on)
    else if (on) await document.documentElement.requestFullscreen()
    else if (document.fullscreenElement) await document.exitFullscreen()
  } catch { /* plein écran refusé : on reste en fenêtre */ return }
  fullscreen = on
  document.body.classList.toggle('fullscreen', on)
  $('t-full').querySelector('use').setAttribute('href', on ? '#i-shrink' : '#i-expand')
  $('t-full').title = on ? 'Quitter le plein écran (F11)' : 'Plein écran (F11)'
  $('t-full').setAttribute('aria-label', $('t-full').title)
  if (!on) document.querySelector('header').classList.remove('peek')
  if (on) toast('Plein écran : F11 pour quitter, ou place la souris en haut de l\'écran pour retrouver la barre.')
}

let peekTimer = null
function peek(on) {
  clearTimeout(peekTimer)
  const header = document.querySelector('header')
  if (on) header.classList.add('peek')
  else peekTimer = setTimeout(() => { if (!menu) header.classList.remove('peek') }, 700)
}

// --- Écran distant ----------------------------------------------------------------------------------------------------

// Décodage d'image : si une image arrive pendant qu'une autre est en cours de décodage, seule la plus récente sera affichée (les intermédiaires sont
// sautées : mieux vaut une image un peu moins fluide qu'un affichage qui prend du retard).
let decoding = false
let pendingFrame = null
async function drawFrame(data) {
  if (decoding) { pendingFrame = data; return }
  decoding = true
  try {
    let next = data
    while (next) {
      pendingFrame = null
      const canvas_ = $('screen')
      const bitmap = await createImageBitmap(new Blob([next], { type: 'image/jpeg' })).catch(() => null)
      if (bitmap) {
        if (canvas_.width !== bitmap.width || canvas_.height !== bitmap.height) { canvas_.width = bitmap.width; canvas_.height = bitmap.height }
        canvas_.getContext('2d').drawImage(bitmap, 0, 0)
        bitmap.close?.()
        tools?.fps.hit()
      }
      next = pendingFrame
    }
  } finally {
    decoding = false
  }
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
  $('v-lat').textContent = ''
  $('v-fps').textContent = ''
  const caps = $('v-caps')
  caps.replaceChildren(...PERMISSIONS.map((p) => {
    const on = permissions.includes(p.id)
    const badge = el('span', 'badge' + (on ? ' on' : ''))
    badge.title = `${p.label} : ${on ? 'activée' : 'désactivée'}`
    badge.setAttribute('aria-label', badge.title)
    badge.append(icon(p.icon))
    return badge
  }))
  $('v-hint').textContent = permissions.includes('control_keyboard') ? 'Clique sur l\'écran pour envoyer le clavier à l\'appareil.' : ''
  viewerState('connected')
  render()
  refreshToolbar()
  startTools()
  canvas.focus()
}

async function disconnect() {
  const current = live
  live = null
  stopTools()
  if (fullscreen) await setFullscreen(false)
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
// F11 reste à cette application (plein écran) : il n'est pas envoyé à l'appareil.
canvas.addEventListener('keydown', (event) => { if (event.key !== 'F11' && can('control_keyboard')) { event.preventDefault(); send(keyMessage(event, true)) } })
canvas.addEventListener('keyup', (event) => { if (event.key !== 'F11' && can('control_keyboard')) { event.preventDefault(); send(keyMessage(event, false)) } })
window.addEventListener('keydown', (event) => { if (event.key === 'F11' && live) { event.preventDefault(); setFullscreen(!fullscreen) } })

// Boutons de la barre d'outils.
$('t-quality').addEventListener('click', (event) => {
  const items = [{ head: 'Qualité de l\'image' }, { label: 'Auto', hint: `s'adapte à la connexion (actuellement ${tools.auto.preset.label.toLowerCase()})`, checked: tools.quality === 'auto', run: () => { tools.quality = 'auto'; setPref('quality', 'auto'); applyQuality() } }]
  for (const id of PRESET_ORDER) {
    const preset = PRESETS[id]
    items.push({ label: preset.label, hint: `${preset.hint} · ${preset.fps} images/s`, checked: tools.quality === id, run: () => { tools.quality = id; setPref('quality', id); applyQuality() } })
  }
  openMenu(event.currentTarget, items)
})
$('t-screens').addEventListener('click', (event) => {
  const items = [{ head: 'Écran à afficher' }, ...tools.screens.map((screen) => ({ label: screenLabel(screen, tools.screens.length), hint: screen.name, checked: screen.index === tools.screen, run: () => sendToDevice({ t: 'screen', index: screen.index }) }))]
  openMenu(event.currentTarget, items)
})
$('t-shortcuts').addEventListener('click', (event) => {
  const items = [{ head: 'Envoyer à l\'appareil' }, ...SHORTCUTS.map((shortcut) => ({ label: shortcut.label, hint: shortcut.hint, run: () => sendShortcut(shortcut.id) })), { note: 'Ctrl + Alt + Suppr est réservé à Windows : il ne peut pas être envoyé à distance.' }]
  openMenu(event.currentTarget, items)
})
$('t-clip').addEventListener('click', () => {
  tools.clipOn = !tools.clipOn
  setPref('clip', tools.clipOn ? '1' : '0')
  if (tools.clipOn) startClipboard(); else clearInterval(tools.clipTimer)
  refreshToolbar()
  toast(tools.clipOn ? 'Presse-papiers partagé : activé' : 'Presse-papiers partagé : désactivé')
})
$('t-shot').addEventListener('click', capture)
$('t-full').addEventListener('click', () => setFullscreen(!fullscreen))
$('hotzone').addEventListener('mouseenter', () => peek(true))
document.querySelector('header').addEventListener('mouseenter', () => peek(true))
document.querySelector('header').addEventListener('mouseleave', () => peek(false))

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
