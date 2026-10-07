import { useCallback, useEffect, useRef, useState } from 'react'
import { api, openSession, post } from './remoteApi'
import { MachineArt, guessKind } from './remoteMachines'
import RemoteScreen from './RemoteScreen'

// Sous-onglet « Maintenance à distance » de la page Ordinateur (plugin remote, docs/REMOTE_ARCHITECTURE.md) : appareils
// appairés, code d'appairage, ouverture d'une session et écran distant piloté à la souris et au clavier (RemoteScreen.jsx).
// Le contrôle n'est possible que si l'appareil l'autorise (c'est lui qui décide) et que sa personne accepte.
const CARD = 'rounded-2xl border border-gray-700 bg-gray-800/70 p-4'
const BUTTON = 'min-h-11 cursor-pointer rounded-lg px-4 text-sm font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50'
const PRIMARY = `${BUTTON} bg-blue-600 text-white hover:bg-blue-500`

const STATE_TEXT = {
  asking: 'Acceptation sur l\'appareil…',
  connecting: 'Connexion en cours…',
  connected: 'Connecté',
  reconnecting: 'Connexion perdue, reconnexion…',
  lost: 'Connexion perdue',
  closed: 'Session terminée',
}
const PERMISSION_LABEL = { control_mouse: 'Souris', control_keyboard: 'Clavier' }
const ICON = { width: 20, height: 20, viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', strokeWidth: 2, strokeLinecap: 'round', strokeLinejoin: 'round', 'aria-hidden': true }
const PERMISSION_ICON = {
  control_mouse: <svg {...ICON}><rect x="6" y="2" width="12" height="20" rx="6" /><path d="M12 2v7" /></svg>,
  control_keyboard: <svg {...ICON}><rect x="2" y="5" width="20" height="14" rx="2" /><path d="M6 9h.01M10 9h.01M14 9h.01M18 9h.01M7 13h.01M17 13h.01M9 16h6" /></svg>,
}

function DeviceRow({ device, busy, waiting, onConnect }) {
  const optional = ['control_mouse', 'control_keyboard'].filter((p) => device.granted.includes(p))
  // Tout ce que l'appareil autorise est activé par défaut ; on retient seulement ce que la personne désactive.
  const [off, setOff] = useState([])
  const chosen = optional.filter((permission) => !off.includes(permission))
  const toggle = (permission) => setOff((prev) => (prev.includes(permission) ? prev.filter((p) => p !== permission) : [...prev, permission]))
  return (
    <li className={`flex flex-col gap-3 rounded-xl border bg-gray-900/40 p-3 transition-colors sm:flex-row sm:items-center ${device.online ? 'border-gray-700 hover:border-blue-400/50' : 'border-gray-800 opacity-80'}`}>
      <MachineArt kind={guessKind(device)} online={device.online} className="h-20 w-32 shrink-0 self-center" />
      <div className="min-w-0 flex-1">
        <p className="truncate text-base font-semibold text-white">{device.name}</p>
        <p className="mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-gray-400">
          <span className={`inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 ${device.online ? 'bg-emerald-500/10 text-emerald-300' : 'bg-gray-700/40 text-gray-500'}`}>
            <span className={`size-1.5 rounded-full ${device.online ? 'bg-emerald-400' : 'bg-gray-500'}`} />{device.online ? 'Disponible' : 'Hors ligne'}
          </span>
          <span title={`Empreinte ${device.fingerprint}`} className="font-mono">{device.fingerprint.slice(0, 8)}</span>
          {!device.paired && <span>pas appairé</span>}
        </p>
      </div>
      {waiting && (
        <span role="status" className="inline-flex items-center gap-2 whitespace-nowrap text-sm text-amber-300">
          <svg {...ICON} className="animate-spin motion-reduce:animate-none" strokeOpacity="0.9"><path d="M21 12a9 9 0 1 1-6.2-8.55" /></svg>
          <span className="animate-pulse motion-reduce:animate-none">{STATE_TEXT[waiting] || waiting}</span>
        </span>
      )}
      <div className="flex items-center gap-2 self-end sm:self-center">
        {device.paired && optional.map((permission) => {
          const on = chosen.includes(permission)
          const label = PERMISSION_LABEL[permission]
          return (
            <button key={permission} type="button" aria-pressed={on} aria-label={label} disabled={busy} title={`${label} : ${on ? 'activée' : 'désactivée'}`}
              onClick={() => toggle(permission)}
              className={`inline-flex size-11 cursor-pointer items-center justify-center rounded-lg border transition-colors disabled:cursor-not-allowed disabled:opacity-60 ${on ? 'border-blue-400 bg-blue-600 text-white shadow-[0_0_0_3px_rgba(96,165,250,0.25)]' : 'border-gray-600 text-gray-500 hover:bg-gray-700 hover:text-gray-300'}`}>
              {PERMISSION_ICON[permission]}
            </button>
          )
        })}
        {device.paired && optional.length === 0 && <span className="text-xs text-gray-500" title="Cet appareil n'autorise que la consultation de l'écran">Vue seule</span>}
        <button type="button" disabled={busy || !device.paired || !device.online} onClick={() => onConnect(device, ['view_screen', ...chosen])}
          aria-label={`Se connecter à ${device.name}`} title="Se connecter" className={`${PRIMARY} inline-flex w-12 items-center justify-center px-0`}>
          <svg {...ICON} width="22" height="22">
            <rect x="2" y="3" width="20" height="14" rx="2" /><path d="M8 21h8M12 17v4" /><path d="m10 8 5 3-5 3z" fill="currentColor" />
          </svg>
        </button>
      </div>
    </li>
  )
}

export default function RemoteControl({ isActive = true }) {
  const [status, setStatus] = useState(null)
  const [devices, setDevices] = useState([])
  const [error, setError] = useState('')
  const [code, setCode] = useState('')
  const [busy, setBusy] = useState(false)
  const [connectingId, setConnectingId] = useState('')
  const [live, setLive] = useState(null)       // { link, device, permissions }
  const [state, setState] = useState('')
  const [info, setInfo] = useState(null)
  const sinkRef = useRef(null)
  const cursorRef = useRef(null)
  const controlRef = useRef(null)      // messages de l'agent autres que l'écran : ping, statistiques, presse-papiers…

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

  const onControl = useCallback((message) => {
    if (message?.t === 'info') setInfo(message)
    else if (message) controlRef.current?.(message)
  }, [])

  const connect = async (device, permissions) => {
    setBusy(true)
    setConnectingId(device.device_id)
    setState('')
    setError('')
    setInfo(null)
    try {
      const link = await openSession({
        deviceId: device.device_id, permissions,
        onState: setState, onInfo: onControl, onFrame: (data) => sinkRef.current?.(data), onCursor: (message) => cursorRef.current?.(message),
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
        <RemoteScreen link={live.link} device={live.device} sinkRef={sinkRef} cursorRef={cursorRef} controlRef={controlRef} info={info} state={state} permissions={live.permissions} onClose={disconnect} />
      ) : (
        <>
          <section className={CARD}>
            <h3 className="text-sm font-semibold uppercase tracking-wide text-gray-300">Appareils</h3>
            {devices.length === 0 ? (
              <p className="mt-2 text-sm text-gray-400">
                Aucun appareil.{status?.mode === 'rendezvous'
                  ? ' Sur le PC à contrôler, lance l\'agent Remote avec l\'adresse du serveur de rendez-vous, puis saisis ici le code à 6 chiffres qu\'il affiche.'
                  : status && !status.agent_installed
                    ? ' Installe l\'agent Remote sur le PC à contrôler (dossier agent/), puis saisis ici le code à 6 chiffres qu\'il affiche.'
                    : ' Saisis le code à 6 chiffres affiché par l\'agent.'}
              </p>
            ) : (
              <ul className="mt-3 space-y-3">
                {devices.map((device) => <DeviceRow key={device.device_id} device={device} busy={busy} waiting={busy && connectingId === device.device_id ? state : ''} onConnect={connect} />)}
              </ul>
            )}
          </section>

          <form onSubmit={pair} className={`${CARD} flex flex-wrap items-center justify-between gap-3`}>
            <div className="min-w-0">
              <label htmlFor="remote-code" className="text-sm font-semibold text-gray-200">Code d'appairage</label>
              <p className="text-xs text-gray-400">Affiché par l'agent sur l'appareil à contrôler · valable 30 min · usage unique</p>
            </div>
            <div className="flex gap-2">
              <input id="remote-code" inputMode="numeric" autoComplete="off" value={code} placeholder="483921"
                onChange={(event) => setCode(event.target.value.replace(/\D/g, '').slice(0, 6))}
                className="min-h-11 w-36 rounded-lg border border-gray-600 bg-gray-900 px-3 text-center font-mono tracking-widest text-white placeholder:text-gray-600 focus:border-blue-400 focus:outline-none focus:ring-2 focus:ring-blue-400/40" />
              <button type="submit" disabled={busy || code.length !== 6} className={PRIMARY}>Appairer</button>
            </div>
          </form>

          {status?.mode === 'rendezvous' ? (
            <p className="text-xs text-gray-500">
              Accès par Internet via le serveur de rendez-vous {status.server} · aucun port à ouvrir chez les appareils.
            </p>
          ) : status && !status.turn_configured && (
            <p className="text-xs text-gray-500">
              Sur le même réseau, rien à régler. Via Internet, utilise un serveur de rendez-vous (REMOTE_RENDEZVOUS_URL dans backend/.env, voir rendezvous/README.md),
              ou configure un serveur STUN/TURN (REMOTE_STUN_URLS, REMOTE_TURN_URLS).
            </p>
          )}
        </>
      )}
    </div>
  )
}
