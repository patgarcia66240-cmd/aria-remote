// Illustrations des machines : PC de bureau, mini PC, portable. Dessins fixes (aucune donnée de l'appareil n'y entre) ;
// l'écran est allumé (dégradé et logo) quand l'appareil est en ligne, éteint sinon.
export const KINDS = [
  { id: 'desktop', label: 'Bureau', title: 'PC de bureau' },
  { id: 'mini', label: 'Mini PC', title: 'Mini PC' },
  { id: 'laptop', label: 'Portable', title: 'PC portable' },
]

const BODY = '#232a37'
const LINE = '#3b4a66'
const BEZEL = '#1b2130'

const screen = (x, y, w, h, on) => {
  const scale = (Math.min(w, h) * 0.5) / 65
  const logo = on ? `<g transform="translate(${x + w / 2} ${y + h / 2}) scale(${scale.toFixed(4)}) translate(-38 -38.5)"><path d="M34.5 8Q38 4 42 8Q48 14 48.5 24L23.5 71H6.5Z" fill="#fff" fill-opacity=".92"/><path d="M52 31.5Q54 31 55 33.5L69.5 69Q69.8 71 67.5 71H57Q55 71 54 70L37 58.5Q36 57.5 36.8 56.5L50 33Q50.8 31.7 52 31.5Z" fill="#e0f2fe" fill-opacity=".92"/></g>` : ''
  return `<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="5" fill="url(#${on ? 'scr-on' : 'scr-off'})"/>${logo}`
}
const led = (cx, cy, on, r = 2.4) => `<circle cx="${cx}" cy="${cy}" r="${r}" fill="${on ? '#34d399' : '#5b6578'}"/>`
const shadow = (cx, rx) => `<ellipse cx="${cx}" cy="152" rx="${rx}" ry="6" fill="#000" opacity=".28"/>`
const monitor = (x, y, w, h, on) => `
  <rect x="${x}" y="${y}" width="${w}" height="${h}" rx="10" fill="${BEZEL}" stroke="${LINE}" stroke-width="1.5"/>
  ${screen(x + 8, y + 8, w - 16, h - 16, on)}
  <rect x="${x + w / 2 - 11}" y="${y + h}" width="22" height="16" fill="${BODY}" stroke="${LINE}" stroke-width="1.5"/>
  <rect x="${x + w / 2 - 33}" y="${y + h + 14}" width="66" height="8" rx="4" fill="${BODY}" stroke="${LINE}" stroke-width="1.5"/>`

const ART = {
  desktop: (on) => `${shadow(130, 112)}
    ${monitor(18, 12, 150, 100, on)}
    <rect x="186" y="34" width="58" height="104" rx="9" fill="${BODY}" stroke="${LINE}" stroke-width="1.5"/>
    <rect x="197" y="48" width="36" height="5" rx="2.5" fill="#10151f"/><rect x="197" y="59" width="36" height="5" rx="2.5" fill="#10151f"/>
    <circle cx="215" cy="108" r="7" fill="none" stroke="${on ? '#34d399' : '#5b6578'}" stroke-width="2"/>${led(215, 126, on)}`,
  mini: (on) => `${shadow(130, 112)}
    ${monitor(14, 10, 170, 104, on)}
    <rect x="166" y="102" width="82" height="46" rx="10" fill="${BODY}" stroke="${LINE}" stroke-width="1.5"/>
    <rect x="176" y="106" width="62" height="3" rx="1.5" fill="#3b4a66" opacity=".7"/>
    ${led(182, 128, on, 3)}<rect x="196" y="124" width="40" height="5" rx="2.5" fill="#10151f"/><rect x="196" y="134" width="16" height="4" rx="2" fill="#10151f"/><rect x="216" y="134" width="20" height="4" rx="2" fill="#10151f"/>`,
  laptop: (on) => `${shadow(130, 112)}
    <rect x="54" y="14" width="152" height="108" rx="10" fill="${BEZEL}" stroke="${LINE}" stroke-width="1.5"/>
    ${screen(62, 22, 136, 92, on)}
    <path d="M28 126 H232 L224 144 Q222 148 217 148 H43 Q38 148 36 144 Z" fill="#2b3547" stroke="${LINE}" stroke-width="1.5"/>
    <rect x="108" y="126" width="44" height="6" rx="3" fill="${BEZEL}"/>`,
}

export function machineSvg(kind, online) {
  const art = ART[kind] || ART.desktop
  return `<svg viewBox="0 0 260 160" role="img" aria-label="${(KINDS.find((k) => k.id === kind) || KINDS[0]).title}">${art(!!online)}</svg>`
}

/** Type par défaut : ce que l'appareil annonce (platform « …-laptop », « …-mini »), sinon PC de bureau. */
export function guessKind(device) {
  const text = `${device?.platform || ''} ${device?.kind || ''}`.toLowerCase()
  if (/laptop|notebook|portable/.test(text)) return 'laptop'
  if (/mini|nuc|stick/.test(text)) return 'mini'
  return 'desktop'
}

const STORE = 'aria-remote-kinds'
export function loadKinds(storage = globalThis.localStorage) {
  try { return JSON.parse(storage.getItem(STORE) || '{}') || {} } catch { return {} }
}
export function saveKind(deviceId, kind, storage = globalThis.localStorage) {
  if (!KINDS.some((k) => k.id === kind)) return
  try { storage.setItem(STORE, JSON.stringify({ ...loadKinds(storage), [deviceId]: kind })) } catch { /* stockage indisponible : le choix vaut pour cette session */ }
}
