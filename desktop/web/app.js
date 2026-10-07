// Interface d'ARIA Remote Desktop. Tout passe par les commandes Rust (clé du contrôleur, appels réseau) : la page ne voit jamais la clé.
const $ = (id) => document.getElementById(id)
const invoke = window.__TAURI__?.core?.invoke
const LABEL = { view_screen: 'Écran', control_mouse: 'Souris', control_keyboard: 'Clavier' }

let settings = { server: '', configured: false }
let editing = false
let timer = null

const show = (el, on) => { el.hidden = !on }
const message = (error) => (typeof error === 'string' ? error : error?.message || 'Erreur inattendue.')

function pill(tone, text) {
  show($('pill'), !!text)
  $('pill').className = 'pill ' + (tone || '')
  $('pill-text').textContent = text || ''
}

function showError(el, text) { el.textContent = text || ''; show(el, !!text) }

function render() {
  const needSetup = !settings.configured || editing
  show($('setup'), needSetup)
  show($('home'), !needSetup)
  show($('open-settings'), settings.configured && !editing)
  show($('cancel-settings'), settings.configured)
  $('key-opt').textContent = settings.configured ? '(laisse vide pour garder la clé enregistrée)' : ''
  if (needSetup) { pill(null, ''); stopPolling() } else startPolling()
}

function deviceRow(device) {
  const li = document.createElement('li')
  const info = document.createElement('div'); info.className = 'info'
  const name = document.createElement('div'); name.className = 'name'; name.textContent = device.name || device.device_id
  const state = document.createElement('div'); state.className = 'state' + (device.online ? ' on' : '')
  const dot = document.createElement('span'); dot.className = 'dot'
  state.append(dot, device.online ? 'Disponible' : 'Hors ligne')
  const chips = document.createElement('div'); chips.className = 'chips'
  for (const permission of device.granted || []) {
    const chip = document.createElement('span'); chip.className = 'chip'; chip.textContent = LABEL[permission] || permission; chips.append(chip)
  }
  info.append(name, state, chips)
  const connect = document.createElement('button')
  connect.textContent = 'Se connecter'; connect.disabled = true
  connect.title = 'La prise de contrôle arrive à l\'étape suivante'
  li.append(info, connect)
  return li
}

async function refresh() {
  if (!settings.configured || editing) return
  try {
    const { devices = [] } = await invoke('list_devices')
    const paired = devices.filter((d) => d.paired !== false)
    $('devices').replaceChildren(...paired.map(deviceRow))
    show($('empty'), paired.length === 0)
    showError($('list-err'), '')
    pill('ok', 'Connecté au serveur')
  } catch (error) {
    showError($('list-err'), message(error))
    pill('bad', 'Serveur injoignable')
  }
}

function startPolling() { if (!timer) { refresh(); timer = setInterval(() => { if (document.visibilityState === 'visible') refresh() }, 4000) } }
function stopPolling() { clearInterval(timer); timer = null }

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

function pairingDigits(value) { return value.replace(/\D/g, '').slice(0, 6) }

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
    const { device } = await invoke('pair_device', { code })
    $('code').value = ''
    $('pair-ok').textContent = `${device?.name || 'Appareil'} est appairé.`; show($('pair-ok'), true)
    refresh()
  } catch (error) {
    showError($('pair-err'), message(error))
    $('pair-btn').disabled = false
  }
})

$('save').addEventListener('click', save)
$('srv').addEventListener('keydown', (e) => { if (e.key === 'Enter') save() })
$('key').addEventListener('keydown', (e) => { if (e.key === 'Enter') save() })
$('open-settings').addEventListener('click', () => { editing = true; $('srv').value = settings.server; showError($('setup-err'), ''); render() })
$('cancel-settings').addEventListener('click', () => { editing = false; render() })

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
