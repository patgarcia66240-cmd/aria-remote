import { useCallback, useEffect, useRef, useState } from 'react'
import { keyMessage, pointerMessage, wheelMessage } from './remoteApi'
import {
  AutoQuality, ClipboardSync, FpsMeter, LatencyMeter, PRESET_ORDER, PRESETS, SHORTCUTS,
  latencyTone, screenLabel, shortcutMessages, statsTitle, streamMessage, stuckKeys,
} from './remoteTools'

// Écran distant d'une session : image, curseur de l'appareil, souris/clavier et barre d'outils (qualité, écran, raccourcis, presse-papiers,
// capture, plein écran). Mêmes outils et mêmes réglages que l'application de bureau (desktop/web/app.js) ; ils n'apparaissent que si l'agent
// les annonce (message « info » > « features »).
const MOVE_INTERVAL_MS = 30
const STATE_TEXT = {
  asking: 'Acceptation sur l\'appareil…',
  connecting: 'Connexion en cours…',
  connected: 'Connecté',
  reconnecting: 'Connexion perdue, reconnexion…',
  lost: 'Connexion perdue',
  closed: 'Session terminée',
}
const TONE = {
  ok: 'border-emerald-500/40 bg-emerald-500/10 text-emerald-300',
  warn: 'border-amber-500/40 bg-amber-500/10 text-amber-300',
  bad: 'border-red-500/40 bg-red-500/10 text-red-300',
  idle: 'border-gray-600 bg-gray-800 text-gray-400',
}
const ICON = { width: 18, height: 18, viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', strokeWidth: 2, strokeLinecap: 'round', strokeLinejoin: 'round', 'aria-hidden': true }
const PATHS = {
  quality: <><path d="M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3" /><path d="M1 14h6M9 8h6M17 16h6" /></>,
  screens: <><rect x="2" y="4" width="14" height="10" rx="2" /><rect x="8" y="10" width="14" height="10" rx="2" fill="none" /></>,
  shortcuts: <><rect x="2" y="5" width="20" height="14" rx="2" /><path d="M6 9h.01M10 9h.01M14 9h.01M18 9h.01M7 13h10" /></>,
  clipboard: <><rect x="8" y="2" width="8" height="4" rx="1" /><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2" /></>,
  shot: <><path d="M23 19a2 2 0 0 1-2 2H3a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4l2-3h6l2 3h4a2 2 0 0 1 2 2z" /><circle cx="12" cy="13" r="4" /></>,
  expand: <path d="M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7" />,
  shrink: <path d="M4 14h6v6M20 10h-6V4M14 10l7-7M3 21l7-7" />,
  power: <><path d="M18.4 6.6a9 9 0 1 1-12.8 0" /><path d="M12 2v10" /></>,
  more: <><circle cx="5" cy="12" r="1.2" /><circle cx="12" cy="12" r="1.2" /><circle cx="19" cy="12" r="1.2" /></>,
  check: <path d="m5 12 5 5 9-10" />,
}
const Icon = ({ name, ...props }) => <svg {...ICON} {...props}>{PATHS[name]}</svg>

const pref = (key, fallback) => { try { return localStorage.getItem(`aria-remote-${key}`) ?? fallback } catch { return fallback } }
const setPref = (key, value) => { try { localStorage.setItem(`aria-remote-${key}`, value) } catch { /* stockage indisponible : le choix vaut pour cette session */ } }
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function ToolButton({ icon, label, onClick, pressed, expanded, ...props }) {
  return (
    <button type="button" title={label} aria-label={label} aria-pressed={pressed} aria-expanded={expanded} aria-haspopup={expanded === undefined ? undefined : 'menu'} onClick={onClick}
      className={`inline-flex size-11 cursor-pointer items-center justify-center rounded-lg border transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-400 ${pressed ? 'border-blue-400 bg-blue-600/30 text-blue-100' : 'border-gray-600 text-gray-300 hover:bg-gray-700 hover:text-white'}`} {...props}>
      <Icon name={icon} />
    </button>
  )
}

/** Menu sous un bouton de la barre : items { head } | { note } | { label, hint, checked, run }. Se ferme au clic ailleurs et sur Échap. */
function ToolMenu({ icon, label, items, onClose, open, onToggle }) {
  const ref = useRef(null)
  useEffect(() => {
    if (!open) return undefined
    const outside = (event) => { if (!ref.current?.contains(event.target)) onClose() }
    const key = (event) => { if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); onClose() } }
    document.addEventListener('pointerdown', outside, true)
    document.addEventListener('keydown', key, true)
    return () => { document.removeEventListener('pointerdown', outside, true); document.removeEventListener('keydown', key, true) }
  }, [open, onClose])
  return (
    <div ref={ref} className="relative">
      <ToolButton icon={icon} label={label} expanded={open} onClick={onToggle} />
      {open && (
        <div role="menu" aria-label={label} className="absolute right-0 top-12 z-30 w-72 overflow-hidden rounded-xl border border-gray-600 bg-gray-900 p-1.5 shadow-xl shadow-black/40">
          {items.map((item, index) => {
            if (item.head) return <p key={index} className="px-3 pb-1 pt-2 text-xs font-semibold uppercase tracking-wide text-gray-500">{item.head}</p>
            if (item.note) return <p key={index} className="px-3 py-2 text-xs text-gray-500">{item.note}</p>
            return (
              <button key={index} type="button" role={item.checked === undefined ? 'menuitem' : 'menuitemradio'} aria-checked={item.checked}
                onClick={() => { onClose(); item.run() }}
                className="flex min-h-11 w-full cursor-pointer items-center gap-2 rounded-lg px-3 py-2 text-left text-sm text-gray-200 hover:bg-gray-700 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-400">
                <span className="inline-flex size-4 shrink-0 text-blue-300">{item.checked ? <Icon name="check" width={16} height={16} /> : null}</span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate">{item.label}</span>
                  {item.hint && <span className="block truncate text-xs text-gray-500">{item.hint}</span>}
                </span>
              </button>
            )
          })}
        </div>
      )}
    </div>
  )
}

export default function RemoteScreen({ link, device, permissions, info, state, sinkRef, cursorRef, controlRef, onClose }) {
  const stageRef = useRef(null)
  const canvasRef = useRef(null)
  const pointerRef = useRef(null)
  const lastMove = useRef(0)
  const [menu, setMenu] = useState('')
  const [toast, setToast] = useState(null)
  const [fullscreen, setFullscreen] = useState(false)
  const [quality, setQualityState] = useState(() => pref('quality', 'auto'))
  const [clipOn, setClipOnState] = useState(() => pref('clip', '1') === '1')
  const [autoLabel, setAutoLabel] = useState(PRESETS.balanced.label)
  const [stats, setStats] = useState({ rtt: null, fps: null, agent: null })

  const canMouse = permissions.includes('control_mouse')
  const canKeyboard = permissions.includes('control_keyboard')
  const features = new Set(info?.features || [])
  const screens = info?.screens || []
  const ratio = info?.width && info?.height ? (info.width / info.height).toFixed(4) : '1.7778'
  const send = useCallback((message) => link.sendInput(message), [link])

  // Valeurs lues par les minuteries : on évite de les relancer à chaque changement de réglage.
  const live = useRef({})
  live.current = { quality, clipOn, canKeyboard, features, send }
  const meters = useRef(null)
  if (!meters.current) meters.current = { latency: new LatencyMeter(), fps: new FpsMeter(), auto: new AutoQuality('balanced'), agent: null, freshAgent: false, lastFps: null, sync: null, lastDenied: 0 }

  const notify = useCallback((text, tone = '') => {
    setToast({ text, tone, id: Date.now() })
  }, [])
  useEffect(() => {
    if (!toast) return undefined
    const timer = setTimeout(() => setToast(null), toast.tone === 'bad' ? 6000 : 3500)
    return () => clearTimeout(timer)
  }, [toast])

  const currentPreset = () => {
    const { quality: q } = live.current
    return q === 'auto' ? meters.current.auto.preset : PRESETS[q] || PRESETS.balanced
  }
  const applyQuality = useCallback(() => {
    const { features: f, send: s } = live.current
    if (f.has('stream')) s(streamMessage(currentPreset()))
    setAutoLabel(meters.current.auto.preset.label)
  }, [])
  const setQuality = (value) => { setQualityState(value); setPref('quality', value); live.current.quality = value; applyQuality() }

  // Images : si une image arrive pendant qu'une autre est décodée, seule la plus récente est affichée (mieux vaut moins fluide qu'en retard).
  useEffect(() => {
    let decoding = false
    let pending = null
    const draw = async (data) => {
      if (decoding) { pending = data; return }
      decoding = true
      try {
        let next = data
        while (next) {
          pending = null
          const canvas = canvasRef.current
          const bitmap = await createImageBitmap(new Blob([next], { type: 'image/jpeg' })).catch(() => null)
          if (bitmap && canvas) {
            if (canvas.width !== bitmap.width || canvas.height !== bitmap.height) { canvas.width = bitmap.width; canvas.height = bitmap.height }
            canvas.getContext('2d').drawImage(bitmap, 0, 0)
            meters.current.fps.hit()
          }
          bitmap?.close?.()
          next = pending
        }
      } finally { decoding = false }
    }
    sinkRef.current = draw
    return () => { sinkRef.current = null }
  }, [sinkRef])

  // Le curseur de l'appareil n'est pas dans l'image capturée : l'agent envoie sa position (0..1), on la dessine par-dessus.
  useEffect(() => {
    cursorRef.current = ({ x, y }) => {
      const pointer = pointerRef.current
      if (!pointer || !Number.isFinite(x) || !Number.isFinite(y)) return
      pointer.style.left = `${Math.min(1, Math.max(0, x)) * 100}%`
      pointer.style.top = `${Math.min(1, Math.max(0, y)) * 100}%`
      pointer.style.opacity = '1'
    }
    return () => { cursorRef.current = null }
  }, [cursorRef])

  // Autres messages de l'agent : réponses de ping, statistiques, presse-papiers, refus.
  useEffect(() => {
    controlRef.current = (msg) => {
      const m = meters.current
      if (msg.t === 'pong') {
        if (m.latency.pong(msg.id, performance.now()) !== null) setStats((s) => ({ ...s, rtt: m.latency.average }))
      } else if (msg.t === 'stats') {
        m.agent = msg
        m.freshAgent = true
        setStats((s) => ({ ...s, agent: msg }))
      } else if (msg.t === 'clip') {
        const text = live.current.clipOn && m.sync ? m.sync.received(msg.text) : null
        if (text) navigator.clipboard?.writeText(text).then(() => notify('Texte copié reçu de l\'appareil')).catch(() => {})
      } else if (msg.t === 'denied' && Date.now() - m.lastDenied > 4000) {
        m.lastDenied = Date.now()
        notify(`L'appareil a refusé : ${msg.reason || 'commande non autorisée'}`, 'bad')
      }
    }
    return () => { controlRef.current = null }
  }, [controlRef, notify])

  // Mesures et échanges réguliers : démarrés une fois, quand l'agent a annoncé ses fonctions.
  const ready = !!info
  useEffect(() => {
    if (!ready) return undefined
    const m = meters.current
    const timers = []
    const every = (ms, fn) => timers.push(setInterval(fn, ms))
    const { features: f } = live.current
    link.usePointerChannel?.(f.has('pointer'))
    applyQuality()
    every(1000, () => {
      const value = m.fps.tick(performance.now())
      if (value !== null) { m.lastFps = value; setStats((s) => ({ ...s, fps: value })) }
    })
    if (f.has('ping')) {
      const ping = () => live.current.send(m.latency.ping(performance.now()))
      ping()
      every(2000, ping)
    }
    if (f.has('stream')) {
      every(3000, () => {
        if (live.current.quality !== 'auto') return
        const agent = m.freshAgent ? m.agent : null          // chaque rapport de l'agent ne compte qu'une fois
        if (!agent && m.lastFps === null) return
        m.freshAgent = false
        const next = m.auto.sample({ rtt: m.latency.average, fps: agent ? agent.fps : m.lastFps, dropped: agent ? agent.dropped : 0, idle: agent ? agent.idle : false })
        if (next) { applyQuality(); notify(`Qualité ajustée automatiquement : ${next.label}`) }
      })
    }
    return () => timers.forEach(clearInterval)
  }, [ready, link, applyQuality, notify])

  // Presse-papiers partagé (texte) : ce qui est copié d'un côté devient collable de l'autre, tant que l'icône est activée.
  const clipboardActive = ready && clipOn && canKeyboard && features.has('clipboard') && !!navigator.clipboard?.readText
  useEffect(() => {
    if (!clipboardActive) return undefined
    let stopped = false
    const m = meters.current
    const read = () => navigator.clipboard.readText().catch(() => null)
    let timer
    read().then((initial) => {
      if (stopped) return
      m.sync = new ClipboardSync(initial)
      timer = setInterval(async () => {
        const text = m.sync.changed(await read())
        if (text && !stopped) { live.current.send({ t: 'clip', text }); notify('Texte copié envoyé à l\'appareil') }
      }, 1000)
    })
    return () => { stopped = true; clearInterval(timer); m.sync = null }
  }, [clipboardActive, notify])

  const sendShortcut = async (id) => {
    const sent = []
    try {
      for (const key of shortcutMessages(id)) { send(key); sent.push(key); await sleep(25) }
    } finally {
      for (const key of stuckKeys(sent)) send(key)       // jamais de touche restée enfoncée à distance
    }
  }

  const capture = async () => {
    const canvas = canvasRef.current
    if (!canvas?.width) return
    try {
      const blob = await new Promise((resolve, reject) => canvas.toBlob((b) => (b ? resolve(b) : reject(new Error('image vide'))), 'image/png'))
      const name = String(device.name || 'appareil').replace(/[^\w.-]+/g, '-').slice(0, 40)
      const link_ = document.createElement('a')
      link_.href = URL.createObjectURL(blob)
      link_.download = `ARIA-Remote-${name}-${new Date().toISOString().slice(0, 19).replace(/[:T]/g, '-')}.png`
      link_.click()
      setTimeout(() => URL.revokeObjectURL(link_.href), 10_000)
      notify('Capture enregistrée dans les téléchargements')
    } catch (error) {
      notify(`Capture impossible : ${error.message}`, 'bad')
    }
  }

  const toggleFullscreen = useCallback(async () => {
    try {
      if (document.fullscreenElement) await document.exitFullscreen()
      else await stageRef.current?.requestFullscreen()
    } catch { /* plein écran refusé : on reste en fenêtre */ }
  }, [])
  useEffect(() => {
    const change = () => setFullscreen(!!document.fullscreenElement)
    document.addEventListener('fullscreenchange', change)
    return () => document.removeEventListener('fullscreenchange', change)
  }, [])
  useEffect(() => () => { if (document.fullscreenElement) document.exitFullscreen?.().catch(() => {}) }, [])

  const closeMenu = useCallback(() => { setMenu(''); canvasRef.current?.focus() }, [])
  const toggleMenu = (name) => setMenu((current) => (current === name ? '' : name))

  const mouse = canMouse ? {
    onPointerMove: (event) => {
      const now = performance.now()
      if (now - lastMove.current < MOVE_INTERVAL_MS) return
      lastMove.current = now
      send(pointerMessage('move', event, event.currentTarget))
    },
    onPointerDown: (event) => { event.currentTarget.setPointerCapture?.(event.pointerId); send(pointerMessage('down', event, event.currentTarget)) },
    onPointerUp: (event) => send(pointerMessage('up', event, event.currentTarget)),
    onWheel: (event) => send(wheelMessage(event)),
  } : {}
  // F11 reste à l'interface (plein écran) : il n'est pas envoyé à l'appareil.
  const keyboard = canKeyboard ? {
    onKeyDown: (event) => { if (event.key === 'F11') { event.preventDefault(); toggleFullscreen(); return } event.preventDefault(); send(keyMessage(event, true)) },
    onKeyUp: (event) => { if (event.key === 'F11') return; event.preventDefault(); send(keyMessage(event, false)) },
  } : {}

  const rtt = stats.rtt
  const tone = latencyTone(rtt)
  const fpsText = stats.agent ? (stats.agent.idle ? 'écran fixe' : `${stats.agent.fps} i/s`) : stats.fps === null ? '' : `${stats.fps} i/s`
  const preset = quality === 'auto' ? PRESETS[PRESET_ORDER.find((id) => PRESETS[id].label === autoLabel) || 'balanced'] : PRESETS[quality] || PRESETS.balanced
  const connected = state === 'connected'

  const qualityItems = [
    { head: 'Qualité de l\'image' },
    { label: 'Auto', hint: `s'adapte à la connexion (actuellement ${autoLabel.toLowerCase()})`, checked: quality === 'auto', run: () => setQuality('auto') },
    ...PRESET_ORDER.map((id) => ({ label: PRESETS[id].label, hint: `${PRESETS[id].hint} · ${PRESETS[id].fps} images/s`, checked: quality === id, run: () => setQuality(id) })),
  ]
  const screenItems = [
    { head: 'Écran à afficher' },
    ...screens.map((screen) => ({ label: screenLabel(screen, screens.length), hint: screen.name, checked: screen.index === (info?.screen ?? 0), run: () => send({ t: 'screen', index: screen.index }) })),
  ]
  const shortcutItems = [
    { head: 'Envoyer à l\'appareil' },
    ...SHORTCUTS.map((shortcut) => ({ label: shortcut.label, hint: shortcut.hint, run: () => sendShortcut(shortcut.id) })),
    { note: 'Ctrl + Alt + Suppr est réservé à Windows : il ne peut pas être envoyé à distance.' },
  ]
  const toggleClipboard = () => { const next = !clipOn; setClipOnState(next); setPref('clip', next ? '1' : '0'); notify(next ? 'Presse-papiers partagé : activé' : 'Presse-papiers partagé : désactivé') }
  const moreItems = [
    { head: 'Outils' },
    ...(canKeyboard && features.has('clipboard') ? [{ label: 'Presse-papiers partagé', hint: clipOn ? 'activé : le texte copié passe d’un côté à l’autre' : 'désactivé', checked: clipOn, run: toggleClipboard }] : []),
    { label: 'Capture d’écran', hint: 'enregistre l’écran distant en PNG', run: capture },
    ...(canKeyboard ? [{ head: 'Raccourcis' }, ...shortcutItems] : []),
  ]

  return (
    <section ref={stageRef} className={`relative flex flex-col overflow-hidden border border-gray-700 bg-gray-800/70 ${fullscreen ? 'h-screen rounded-none border-0' : 'rounded-2xl'}`}>
      <header className="flex flex-wrap items-center gap-2 border-b border-gray-700 px-3 py-2">
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-semibold text-white">{device.name}</p>
          <p role="status" className="flex flex-wrap items-center gap-x-2 text-xs text-gray-400">
            <span className={`inline-flex items-center gap-1.5 ${connected ? 'text-emerald-300' : 'text-amber-300'}`}>
              <span className={`size-1.5 rounded-full ${connected ? 'bg-emerald-400' : 'animate-pulse motion-reduce:animate-none bg-amber-400'}`} />{STATE_TEXT[state] || state}
            </span>
            {info?.width ? <span>{info.width}×{info.height}</span> : null}
            <span>Souris : {canMouse ? 'oui' : 'non'}</span>
            <span>Clavier : {canKeyboard ? 'oui' : 'non'}</span>
          </p>
        </div>
        {(rtt !== null || fpsText) && (
          <span title={statsTitle({ rtt, agent: stats.agent })} className={`inline-flex h-11 items-center gap-2 rounded-lg border px-3 text-xs font-medium tabular-nums ${TONE[tone]}`}>
            {rtt !== null && <span>{rtt} ms</span>}
            {fpsText && <span className="opacity-80">{fpsText}</span>}
          </span>
        )}
        <div className="flex items-center gap-1.5">
          {features.has('stream') && <ToolMenu icon="quality" label={`Qualité de l'image : ${quality === 'auto' ? `auto (${preset.label})` : preset.label}`} items={qualityItems} open={menu === 'quality'} onToggle={() => toggleMenu('quality')} onClose={closeMenu} />}
          {features.has('screens') && screens.length > 1 && <ToolMenu icon="screens" label="Écran distant" items={screenItems} open={menu === 'screens'} onToggle={() => toggleMenu('screens')} onClose={closeMenu} />}
          <ToolMenu icon="more" label="Plus d'outils" items={moreItems} open={menu === 'more'} onToggle={() => toggleMenu('more')} onClose={closeMenu} />
          <ToolButton icon={fullscreen ? 'shrink' : 'expand'} label={fullscreen ? 'Quitter le plein écran (F11)' : 'Plein écran (F11)'} onClick={toggleFullscreen} />
          <button type="button" onClick={onClose} className="ml-1 inline-flex min-h-11 cursor-pointer items-center gap-2 rounded-lg border border-red-700 px-3 text-sm font-medium text-red-200 transition-colors hover:bg-red-900/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-red-400">
            <Icon name="power" width={16} height={16} />
            Déconnecter
          </button>
        </div>
      </header>

      {/* Hauteur bornée (la barre du dessus reste visible) SANS bandes noires : le canvas garde le ratio de l'écran distant, ce qui garde
          aussi exacte la position de la souris (coordonnées normalisées sur toute la surface du canvas). */}
      <div className={`flex flex-1 items-center justify-center bg-gray-900/60 ${fullscreen ? 'p-0' : 'p-3'}`}>
        <div className="relative" style={{ aspectRatio: `${ratio}`, width: fullscreen ? `min(100vw, calc((100vh - 3.5rem) * ${ratio}))` : `min(100%, calc((100vh - 20rem) * ${ratio}))` }}>
          <canvas ref={canvasRef} tabIndex={0} aria-label="Écran distant" width="16" height="9" onContextMenu={(event) => event.preventDefault()} {...mouse} {...keyboard}
            className="block h-full w-full touch-none rounded-lg bg-black outline-none focus:ring-2 focus:ring-blue-400" />
          <svg ref={pointerRef} data-testid="remote-cursor" aria-hidden="true" width="18" height="26" viewBox="0 0 18 26"
            className="pointer-events-none absolute opacity-0 drop-shadow-[0_1px_2px_rgba(0,0,0,0.8)]" style={{ left: 0, top: 0 }}>
            <path d="M1 1 L1 20 L6 15.5 L9.5 24 L13 22.5 L9.5 14.5 L16.5 14.5 Z" fill="#fff" stroke="#111" strokeWidth="1.5" strokeLinejoin="round" />
          </svg>
        </div>
      </div>

      {canKeyboard && !fullscreen && <p className="px-4 pb-3 text-center text-xs text-gray-500">Clique sur l'écran pour envoyer le clavier à l'appareil.</p>}

      {toast && (
        <div role="status" aria-live="polite" key={toast.id}
          className={`pointer-events-none absolute bottom-4 left-1/2 z-40 max-w-[90%] -translate-x-1/2 rounded-xl border px-4 py-2 text-sm shadow-lg ${toast.tone === 'bad' ? 'border-red-700 bg-red-950 text-red-100' : 'border-gray-600 bg-gray-900 text-gray-100'}`}>
          {toast.text}
        </div>
      )}
    </section>
  )
}
