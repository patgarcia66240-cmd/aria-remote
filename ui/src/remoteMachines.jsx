// Illustrations des machines (PC de bureau, mini PC, portable) : dessins fixes, aucune donnée de l'appareil n'y entre.
// L'écran est allumé (dégradé et logo) quand l'appareil est en ligne, éteint sinon. Même dessin que l'application de bureau.
const BODY = '#232a37'
const LINE = '#3b4a66'
const BEZEL = '#1b2130'

export const KINDS = {
  desktop: 'PC de bureau',
  mini: 'Mini PC',
  laptop: 'PC portable',
}

/** Type annoncé par l'agent (platform « …-laptop », « …-mini »), sinon PC de bureau. */
export function guessKind(device) {
  const text = `${device?.platform || ''} ${device?.kind || ''}`.toLowerCase()
  if (/laptop|notebook|portable/.test(text)) return 'laptop'
  if (/mini|nuc|stick/.test(text)) return 'mini'
  return 'desktop'
}

function Screen({ x, y, w, h, on }) {
  const cx = x + w / 2
  const cy = y + h / 2
  return (
    <>
      <rect x={x} y={y} width={w} height={h} rx="5" fill={on ? 'url(#rm-on)' : 'url(#rm-off)'} />
      {on && <circle cx={cx} cy={cy} r={Math.min(w, h) * 0.16} fill="none" stroke="#e0f2fe" strokeWidth="2.2" />}
      {on && <circle cx={cx} cy={cy} r={Math.min(w, h) * 0.06} fill="#e0f2fe" />}
    </>
  )
}

const Led = ({ cx, cy, on, r = 2.4 }) => <circle cx={cx} cy={cy} r={r} fill={on ? '#34d399' : '#5b6578'} />
const Shadow = ({ cx, rx }) => <ellipse cx={cx} cy="152" rx={rx} ry="6" fill="#000" opacity=".28" />

function Monitor({ x, y, w, h, on }) {
  return (
    <>
      <rect x={x} y={y} width={w} height={h} rx="10" fill={BEZEL} stroke={LINE} strokeWidth="1.5" />
      <Screen x={x + 8} y={y + 8} w={w - 16} h={h - 16} on={on} />
      <rect x={x + w / 2 - 11} y={y + h} width="22" height="16" fill={BODY} stroke={LINE} strokeWidth="1.5" />
      <rect x={x + w / 2 - 33} y={y + h + 14} width="66" height="8" rx="4" fill={BODY} stroke={LINE} strokeWidth="1.5" />
    </>
  )
}

const ART = {
  desktop: (on) => (
    <>
      <Shadow cx={130} rx={112} />
      <Monitor x={18} y={12} w={150} h={100} on={on} />
      <rect x="186" y="34" width="58" height="104" rx="9" fill={BODY} stroke={LINE} strokeWidth="1.5" />
      <rect x="197" y="48" width="36" height="5" rx="2.5" fill="#10151f" />
      <rect x="197" y="59" width="36" height="5" rx="2.5" fill="#10151f" />
      <circle cx="215" cy="108" r="7" fill="none" stroke={on ? '#34d399' : '#5b6578'} strokeWidth="2" />
      <Led cx={215} cy={126} on={on} />
    </>
  ),
  mini: (on) => (
    <>
      <Shadow cx={130} rx={112} />
      <Monitor x={14} y={10} w={170} h={104} on={on} />
      <rect x="166" y="102" width="82" height="46" rx="10" fill={BODY} stroke={LINE} strokeWidth="1.5" />
      <rect x="176" y="106" width="62" height="3" rx="1.5" fill={LINE} opacity=".7" />
      <Led cx={182} cy={128} on={on} r={3} />
      <rect x="196" y="124" width="40" height="5" rx="2.5" fill="#10151f" />
      <rect x="196" y="134" width="16" height="4" rx="2" fill="#10151f" />
      <rect x="216" y="134" width="20" height="4" rx="2" fill="#10151f" />
    </>
  ),
  laptop: (on) => (
    <>
      <Shadow cx={130} rx={112} />
      <rect x="54" y="14" width="152" height="108" rx="10" fill={BEZEL} stroke={LINE} strokeWidth="1.5" />
      <Screen x={62} y={22} w={136} h={92} on={on} />
      <path d="M28 126 H232 L224 144 Q222 148 217 148 H43 Q38 148 36 144 Z" fill="#2b3547" stroke={LINE} strokeWidth="1.5" />
      <rect x="108" y="126" width="44" height="6" rx="3" fill={BEZEL} />
    </>
  ),
}

export function MachineArt({ kind = 'desktop', online = false, className = '' }) {
  return (
    <svg viewBox="0 0 260 160" role="img" aria-label={KINDS[kind] || KINDS.desktop} className={className}>
      <defs>
        <linearGradient id="rm-on" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stopColor="#38bdf8" /><stop offset="1" stopColor="#4f46e5" /></linearGradient>
        <linearGradient id="rm-off" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stopColor="#1f2937" /><stop offset="1" stopColor="#111827" /></linearGradient>
      </defs>
      {(ART[kind] || ART.desktop)(!!online)}
    </svg>
  )
}
