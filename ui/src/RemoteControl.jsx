import { useCallback, useEffect, useRef, useState } from 'react'
import { api, keyMessage, openSession, pointerMessage, post, wheelMessage } from './remoteApi'

// Sous-onglet « Maintenance à distance » de la page Ordinateur (plugin remote, docs/REMOTE_ARCHITECTURE.md) : appareils
// appairés, code d'appairage, ouverture d'une session et écran distant piloté à la souris et au clavier.
// Le contrôle n'est possible que si l'appareil l'autorise (c'est lui qui décide) et que sa personne accepte.
const CARD = 'rounded-2xl border border-gray-700 bg-gray-800/70 p-4'
const BUTTON = 'min-h-10 cursor-pointer rounded-lg px-4 text-sm font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50'
const PRIMARY = `${BUTTON} bg-blue-600 text-white hover:bg-blue-500`
const SECONDARY = `${BUTTON} border border-gray-600 text-gray-200 hover:bg-gray-700`

const STATE_TEXT = {
  asking: 'En attente de l\'acceptation sur l\'appareil…',
  connecting: 'Connexion en cours…',
  connected: 'Connecté',
  reconnecting: 'Connexion perdue, reconnexion…',
  lost: 'Connexion perdue',
  closed: 'Session terminée',
}
const PERMISSION_LABEL = { control_mouse: 'Souris', control_keyboard: 'Clavier' }
const MOVE_INTERVAL_MS = 30

function Screen({ link, sinkRef, cursorRef, info, onClose, state, permissions }) {
  const canvasRef = useRef(null)
  const pointerRef = useRef(null)
  const lastMove = useRef(0)
  const canMouse = permissions.includes('control_mouse')
  const canKeyboard = permissions.includes('control_keyboard')
  const ratio = info?.width && info?.height ? (info.width / info.height).toFixed(4) : '1.7778'

  // Les images arrivent par le canal « frames » : on les dessine dès qu'elles sont décodées, sans file d'attente.
  useEffect(() => {
    sinkRef.current = async (data) => {
      const canvas = canvasRef.current
      if (!canvas) return
      const bitmap = await createImageBitmap(new Blob([data], { type: 'image/jpeg' }))
      if (canvas.width !== bitmap.width || canvas.height !== bitmap.height) {
        canvas.width = bitmap.width
        canvas.height = bitmap.height
      }
      canvas.getContext('2d').drawImage(bitmap, 0, 0)
      bitmap.close?.()
    }
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

  const send = (message) => link.sendInput(message)
  const mouse = canMouse ? {
    onPointerMove: (event) => {
      const now = performance.now()
      if (now - lastMove.current < MOVE_INTERVAL_MS) return
      lastMove.current = now
      send(pointerMessage('move', event, event.currentTarget))
    },
    onPointerDown: (event) => { event.currentTarget.focus(); event.currentTarget.setPointerCapture?.(event.pointerId); send(pointerMessage('down', event, event.currentTarget)) },
    onPointerUp: (event) => send(pointerMessage('up', event, event.currentTarget)),
    onWheel: (event) => send(wheelMessage(event)),
    onContextMenu: (event) => event.preventDefault(),
  } : {}
  const keyboard = canKeyboard ? {
    onKeyDown: (event) => { event.preventDefault(); send(keyMessage(event, true)) },
    onKeyUp: (event) => { event.preventDefault(); send(keyMessage(event, false)) },
  } : {}

  return (
    <div className={CARD}>
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <p role="status" className="text-sm text-gray-300">
          <span className={state === 'connected' ? 'text-emerald-300' : 'text-amber-300'}>{STATE_TEXT[state] || state}</span>
          {info?.width ? ` · ${info.width}×${info.height}` : ''}
          {' · '}Souris : {canMouse ? 'oui' : 'non'} · Clavier : {canKeyboard ? 'oui' : 'non'}
        </p>
        <button type="button" onClick={onClose} className={`${SECONDARY} inline-flex items-center gap-2`}>
          <svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M18.4 6.6a9 9 0 1 1-12.8 0" /><path d="M12 2v10" /></svg>
          Déconnecter
        </button>
      </div>
      {/* Hauteur bornée (la barre du dessus reste visible) SANS bandes noires : le canvas garde le ratio de l'écran distant, ce qui garde
          aussi exacte la position de la souris (coordonnées normalisées sur toute la surface du canvas). */}
      <div className="relative mx-auto" style={{ aspectRatio: `${ratio}`, width: `min(100%, calc((100vh - 22rem) * ${ratio}))` }}>
        <canvas ref={canvasRef} tabIndex={0} aria-label="Écran distant" width="16" height="9" {...mouse} {...keyboard}
          className="block h-full w-full touch-none rounded-lg bg-black outline-none focus:ring-2 focus:ring-blue-400" />
        <svg ref={pointerRef} data-testid="remote-cursor" aria-hidden="true" width="18" height="26" viewBox="0 0 18 26"
          className="pointer-events-none absolute opacity-0 drop-shadow-[0_1px_2px_rgba(0,0,0,0.8)]" style={{ left: 0, top: 0 }}>
          <path d="M1 1 L1 20 L6 15.5 L9.5 24 L13 22.5 L9.5 14.5 L16.5 14.5 Z" fill="#fff" stroke="#111" strokeWidth="1.5" strokeLinejoin="round" />
        </svg>
      </div>
      {canKeyboard && <p className="mt-2 text-xs text-gray-500">Clique sur l'écran pour envoyer le clavier à l'appareil.</p>}
    </div>
  )
}

function DeviceRow({ device, busy, onConnect }) {
  const optional = ['control_mouse', 'control_keyboard'].filter((p) => device.granted.includes(p))
  const [chosen, setChosen] = useState([])
  const toggle = (permission) => setChosen((prev) => (prev.includes(permission) ? prev.filter((p) => p !== permission) : [...prev, permission]))
  return (
    <li className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-gray-700 bg-gray-900/40 p-3">
      <div className="min-w-0">
        <p className="font-medium text-white">{device.name}</p>
        <p className="text-xs text-gray-400">
          <span className={device.online ? 'text-emerald-300' : 'text-gray-500'}>● {device.online ? 'Disponible' : 'Hors ligne'}</span>
          {' · '}empreinte {device.fingerprint}
          {!device.paired && ' · pas encore appairé'}
        </p>
        {device.paired && optional.length > 0 && (
          <div className="mt-2 flex flex-wrap gap-4 text-sm text-gray-300">
            {optional.map((permission) => (
              <label key={permission} className="flex cursor-pointer items-center gap-2">
                <input type="checkbox" checked={chosen.includes(permission)} onChange={() => toggle(permission)} />
                {PERMISSION_LABEL[permission]}
              </label>
            ))}
          </div>
        )}
        {device.paired && optional.length === 0 && <p className="mt-1 text-xs text-gray-500">Cet appareil n'autorise que la consultation de l'écran.</p>}
      </div>
      <button type="button" disabled={busy || !device.paired || !device.online} onClick={() => onConnect(device, ['view_screen', ...chosen])}
        aria-label={`Se connecter à ${device.name}`} title="Se connecter" className={`${PRIMARY} inline-flex w-12 items-center justify-center px-0`}>
        <svg aria-hidden="true" width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <rect x="2" y="3" width="20" height="14" rx="2" /><path d="M8 21h8M12 17v4" /><path d="m10 8 5 3-5 3z" fill="currentColor" />
        </svg>
      </button>
    </li>
  )
}

export default function RemoteControl({ isActive = true }) {
  const [status, setStatus] = useState(null)
  const [devices, setDevices] = useState([])
  const [error, setError] = useState('')
  const [code, setCode] = useState('')
  const [busy, setBusy] = useState(false)
  const [live, setLive] = useState(null)       // { link, device, permissions }
  const [state, setState] = useState('')
  const [info, setInfo] = useState(null)
  const sinkRef = useRef(null)
  const cursorRef = useRef(null)

  const load = useCallback(async () => {
    try {
      const [s, d] = await Promise.all([api('/status'), api('/devices')])
      setStatus(s)
      setDevices(d.devices)
      setError('')
    } catch (e) {
      setError(e.message)
    }
  }, [])

  useEffect(() => {
    if (!isActive) return undefined
    load()
    const timer = setInterval(load, 5000)
    return () => clearInterval(timer)
  }, [isActive, load])

  const pair = async (event) => {
    event.preventDefault()
    setBusy(true)
    try {
      await post('/pair', { code })
      setCode('')
      await load()
    } catch (e) {
      setError(e.message)
    } finally {
      setBusy(false)
    }
  }

  const connect = async (device, permissions) => {
    setBusy(true)
    setError('')
    setInfo(null)
    try {
      const link = await openSession({
        deviceId: device.device_id, permissions,
        onState: setState, onInfo: setInfo, onFrame: (data) => sinkRef.current?.(data), onCursor: (message) => cursorRef.current?.(message),
      })
      setLive({ link, device, permissions })
    } catch (e) {
      setError(e.message)
      setState('')
    } finally {
      setBusy(false)
    }
  }

  const disconnect = async () => {
    const current = live
    setLive(null)
    await current?.link.close()
    load()
  }

  // Quitter l'onglet ou fermer la page coupe la session : on ne laisse jamais un contrôle ouvert sans écran.
  useEffect(() => () => { live?.link.close() }, [live])

  return (
    <div className="space-y-4">
      {error && <p role="alert" className="rounded-lg border border-red-800 bg-red-900/30 p-3 text-sm text-red-200">{error}</p>}

      {live ? (
        <Screen link={live.link} sinkRef={sinkRef} cursorRef={cursorRef} info={info} state={state} permissions={live.permissions} onClose={disconnect} />
      ) : (
        <>
          <section className={CARD}>
            <h3 className="text-sm font-semibold text-gray-200">Appareils</h3>
            {devices.length === 0 ? (
              <p className="mt-2 text-sm text-gray-400">
                Aucun appareil.{status?.mode === 'rendezvous'
                  ? ' Sur le PC à contrôler, lance l\'agent Remote avec l\'adresse du serveur de rendez-vous, puis saisis ici le code à 6 chiffres qu\'il affiche.'
                  : status && !status.agent_installed
                    ? ' Installe l\'agent Remote sur le PC à contrôler (dossier remote-agent/), puis saisis ici le code à 6 chiffres qu\'il affiche.'
                    : ' Saisis le code à 6 chiffres affiché par l\'agent.'}
              </p>
            ) : (
              <ul className="mt-3 space-y-2">
                {devices.map((device) => <DeviceRow key={device.device_id} device={device} busy={busy} onConnect={connect} />)}
              </ul>
            )}
            {busy && state === 'asking' && <p role="status" className="mt-3 text-sm text-amber-300">{STATE_TEXT.asking}</p>}
          </section>

          <form onSubmit={pair} className={CARD}>
            <label htmlFor="remote-code" className="text-sm font-semibold text-gray-200">Code d'appairage</label>
            <p className="mt-1 text-xs text-gray-400">Le code est affiché par l'agent sur l'appareil à contrôler ; il a une durée limitée (30 minutes par défaut) et ne sert qu'une fois.</p>
            <div className="mt-3 flex gap-2">
              <input id="remote-code" inputMode="numeric" autoComplete="off" value={code} placeholder="483921"
                onChange={(event) => setCode(event.target.value.replace(/\D/g, '').slice(0, 6))}
                className="min-h-10 w-40 rounded-lg border border-gray-600 bg-gray-900 px-3 text-center font-mono tracking-widest text-white" />
              <button type="submit" disabled={busy || code.length !== 6} className={PRIMARY}>Appairer</button>
            </div>
          </form>

          {status?.mode === 'rendezvous' ? (
            <p className="text-xs text-gray-500">
              Accès par Internet via le serveur de rendez-vous {status.server}. Les appareils s'y connectent d'eux-mêmes : aucun port à ouvrir chez eux.
            </p>
          ) : status && !status.turn_configured && (
            <p className="text-xs text-gray-500">
              Sur le même réseau, rien à régler. Via Internet, utilise un serveur de rendez-vous (REMOTE_RENDEZVOUS_URL dans backend/.env, voir remote-rendezvous/README.md),
              ou configure un serveur STUN/TURN (REMOTE_STUN_URLS, REMOTE_TURN_URLS).
            </p>
          )}
        </>
      )}
    </div>
  )
}
