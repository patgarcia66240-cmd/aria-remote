// Clé du contrôleur pour l'onglet ARIA (phase 5 de docs/SIGNATURE_CONTROLEUR.md).
// La clé privée Ed25519 est créée par WebCrypto en mode NON extractible et rangée dans IndexedDB : le code de la page ne peut pas la lire,
// il peut seulement demander une signature. La clé publique (32 octets, base64) sert à l'appairage ; l'empreinte se compare sur les deux écrans.
// Format identique à l'agent (agent/src/identity.rs) : base64 standard, SHA-256 en hexadécimal, message « pc-assistant-remote-v1|controller-offer|… ».

const DB_NAME = 'aria-remote-controller'
const STORE = 'keys'
const KEY_ID = 'controller'
const PREFIX = 'pc-assistant-remote-v1'
const ED25519_SPKI_HEADER = 12   // en-tête fixe de la clé publique Ed25519 au format SPKI ; les 32 octets suivants sont la clé brute

const toBase64 = (bytes) => btoa(String.fromCharCode(...new Uint8Array(bytes)))

function openDb() {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, 1)
    request.onupgradeneeded = () => request.result.createObjectStore(STORE)
    request.onsuccess = () => resolve(request.result)
    request.onerror = () => reject(request.error)
  })
}

async function loadPair(id = KEY_ID) {
  const db = await openDb()
  return new Promise((resolve, reject) => {
    const request = db.transaction(STORE).objectStore(STORE).get(id)
    request.onsuccess = () => { db.close(); resolve(request.result || null) }
    request.onerror = () => { db.close(); reject(request.error) }
  })
}

async function savePair(pair, id = KEY_ID) {
  const db = await openDb()
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, 'readwrite')
    tx.objectStore(STORE).put(pair, id)
    tx.oncomplete = () => { db.close(); resolve() }
    tx.onerror = () => { db.close(); reject(tx.error) }
  })
}

async function ensurePair() {
  if (!globalThis.crypto?.subtle || typeof indexedDB === 'undefined') {
    throw new Error('Ce navigateur ne permet pas de signer les sessions. Utilise l\'application de bureau pour contrôler cet appareil.')
  }
  const existing = await loadPair()
  if (existing) return existing
  let pair
  try {
    pair = await crypto.subtle.generateKey({ name: 'Ed25519' }, false, ['sign', 'verify'])   // privée non extractible
  } catch {
    throw new Error('Ce navigateur ne sait pas créer la clé de signature (Ed25519). Utilise l\'application de bureau pour contrôler cet appareil.')
  }
  await savePair(pair)
  return pair
}

async function rawPublicKey(pair) {
  const spki = new Uint8Array(await crypto.subtle.exportKey('spki', pair.publicKey))
  return spki.slice(ED25519_SPKI_HEADER)
}

async function sha256(bytes) {
  return new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))
}

const toHex = (bytes) => Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('')

/** { publicKey, fingerprint } : clé publique en base64 et empreinte lisible (même calcul que l'agent). */
export async function controllerIdentity() {
  const pair = await ensurePair()
  const raw = await rawPublicKey(pair)
  const hex = toHex(await sha256(raw)).slice(0, 16)
  return { publicKey: toBase64(raw), fingerprint: `${hex.slice(0, 4)} ${hex.slice(4, 8)} ${hex.slice(8, 12)} ${hex.slice(12, 16)}` }
}

/** Signature base64 de l'offre, liée à la session et au contenu exact du SDP. */
export async function signOffer(sessionId, sdp) {
  const pair = await ensurePair()
  const digest = toHex(await sha256(new TextEncoder().encode(sdp)))
  const message = new TextEncoder().encode(`${PREFIX}|controller-offer|${sessionId}|${digest}`)
  return toBase64(await crypto.subtle.sign('Ed25519', pair.privateKey, message))
}

// --- Clés épinglées des appareils (point 2 de l'audit) -----------------------------------------------------------------------
// La clé publique de chaque appareil est reçue à l'appairage et rangée ici ; une réponse n'est acceptée que si elle est signée par cette clé.

const PIN_PREFIX = 'pin:'

function fromBase64(text) {
  return Uint8Array.from(atob(text.trim()), (c) => c.charCodeAt(0))
}

/** Épingle la clé publique (32 octets, base64) d'un appareil ; renvoie son empreinte, à comparer avec la fenêtre de l'agent. */
export async function pinDevice(deviceId, publicKey) {
  const raw = fromBase64(publicKey)
  if (raw.length !== 32) throw new Error('Clé de l\'appareil invalide (32 octets attendus).')
  await savePair(publicKey.trim(), `${PIN_PREFIX}${deviceId}`)
  const hex = toHex(await sha256(raw)).slice(0, 16)
  return `${hex.slice(0, 4)} ${hex.slice(4, 8)} ${hex.slice(8, 12)} ${hex.slice(12, 16)}`
}

/** Vrai seulement si la réponse est signée par la clé épinglée de cet appareil (même message que l'agent). Pas de clé épinglée = faux. */
export async function verifyAnswer(deviceId, sessionId, offer, answer, signature) {
  const pinned = await loadPair(`${PIN_PREFIX}${deviceId}`)
  if (!pinned || !signature) return false
  try {
    const key = await crypto.subtle.importKey('raw', fromBase64(pinned), 'Ed25519', false, ['verify'])
    const message = `${PREFIX}|answer|${sessionId}|${toHex(await sha256(new TextEncoder().encode(offer)))}|${toHex(await sha256(new TextEncoder().encode(answer)))}`
    return await crypto.subtle.verify('Ed25519', key, fromBase64(signature), new TextEncoder().encode(message))
  } catch {
    return false
  }
}
