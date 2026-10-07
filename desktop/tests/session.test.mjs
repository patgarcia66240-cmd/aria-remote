// Tests de la négociation et des entrées (web/session.js) : `node --test tests` depuis desktop/.
import assert from 'node:assert/strict'
import { describe, it, mock } from 'node:test'
import { FrameAssembler, createRequest, keyMessage, openSession, pointerMessage, pointerPosition, wheelMessage } from '../web/session.js'

function chunk(id, index, count, bytes) {
  const buffer = new ArrayBuffer(8 + bytes.length)
  const view = new DataView(buffer)
  view.setUint32(0, id)
  view.setUint16(4, index)
  view.setUint16(6, count)
  new Uint8Array(buffer, 8).set(bytes)
  return buffer
}

async function until(check, label = 'condition') {
  for (let i = 0; i < 200; i += 1) {
    if (check()) return
    await new Promise((resolve) => setTimeout(resolve, 5))
  }
  assert.fail(`délai dépassé : ${label}`)
}

describe('FrameAssembler', () => {
  it('recompose une image à partir de ses morceaux, dans l\'ordre', () => {
    const assembler = new FrameAssembler()
    assert.equal(assembler.push(chunk(7, 0, 3, [1, 2])), null)
    assert.equal(assembler.push(chunk(7, 1, 3, [3])), null)
    assert.deepEqual([...assembler.push(chunk(7, 2, 3, [4, 5]))], [1, 2, 3, 4, 5])
    assert.deepEqual([...assembler.push(chunk(8, 0, 1, [9]))], [9])
  })

  it('abandonne une image incomplète et ignore les messages invalides', () => {
    const assembler = new FrameAssembler()
    assembler.push(chunk(1, 0, 2, [1]))
    assert.deepEqual([...assembler.push(chunk(2, 0, 1, [7]))], [7])
    assembler.push(chunk(3, 0, 3, [1]))
    assert.equal(assembler.push(chunk(3, 2, 3, [3])), null)
    assert.equal(assembler.push(new ArrayBuffer(3)), null)
    assert.equal(assembler.push(chunk(4, 5, 2, [1])), null)
    assert.equal(assembler.push(null), null)
  })
})

const rect = { left: 100, top: 50, width: 400, height: 200 }
const element = { getBoundingClientRect: () => rect }

describe('entrées du contrôle à distance', () => {
  it('normalise la position du pointeur entre 0 et 1, même hors de l\'écran', () => {
    assert.deepEqual(pointerPosition({ clientX: 300, clientY: 150 }, element), { x: 0.5, y: 0.5 })
    assert.deepEqual(pointerPosition({ clientX: 0, clientY: 999 }, element), { x: 0, y: 1 })
    assert.deepEqual(pointerPosition({ clientX: 1, clientY: 1 }, { getBoundingClientRect: () => ({ left: 0, top: 0, width: 0, height: 0 }) }), { x: 0, y: 0 })
  })

  it('n\'envoie le bouton que pour un appui ou un relâchement, et borne la molette', () => {
    assert.deepEqual(pointerMessage('move', { clientX: 100, clientY: 50, button: 0 }, element), { t: 'move', x: 0, y: 0 })
    assert.deepEqual(pointerMessage('down', { clientX: 500, clientY: 250, button: 2 }, element), { t: 'down', x: 1, y: 1, button: 2 })
    assert.deepEqual(wheelMessage({ deltaX: 0, deltaY: 5000 }), { t: 'wheel', dx: 0, dy: 400 })
    assert.deepEqual(wheelMessage({ deltaX: -3, deltaY: -9000 }), { t: 'wheel', dx: -3, dy: -400 })
    assert.deepEqual(keyMessage({ code: 'KeyA', key: 'a' }, true), { t: 'key', down: true, code: 'KeyA', key: 'a' })
  })
})

describe('createRequest', () => {
  it('passe par la commande api_request, avec la méthode et le corps', async () => {
    const invoke = mock.fn(async () => ({ ok: true }))
    const request = createRequest(invoke)
    await request.api('/devices')
    await request.api('/sessions/s1', { method: 'DELETE' })
    await request.post('/pair', { code: '085201' })
    await request.post('/sessions/s1/reconnect')
    assert.deepEqual(invoke.mock.calls.map((c) => c.arguments), [
      ['api_request', { method: 'GET', path: '/devices', body: null }],
      ['api_request', { method: 'DELETE', path: '/sessions/s1', body: null }],
      ['api_request', { method: 'POST', path: '/pair', body: { code: '085201' } }],
      ['api_request', { method: 'POST', path: '/sessions/s1/reconnect', body: {} }],
    ])
  })
})

function fakePeer(log) {
  const peer = {
    connectionState: 'new',
    localDescription: null,
    channels: [],
    closed: 0,
    createDataChannel: (label) => { const channel = { label, readyState: 'open', sent: [], send: (data) => channel.sent.push(data) }; peer.channels.push(channel); return channel },
    createOffer: async () => ({ type: 'offer', sdp: 'v=0 offer' }),
    setLocalDescription: async (description) => {
      peer.localDescription = description
      peer.onicecandidate?.({ candidate: { candidate: 'candidate:local' } })     // un candidat sort avant que l'offre soit partie
    },
    remote: null,
    setRemoteDescription: async (description) => { peer.remote = description; log.push(['remote', description.sdp]); peer.connectionState = 'connected' },
    added: [],
    addIceCandidate: async (candidate) => { peer.added.push(candidate) },
    close: () => { peer.closed += 1 },
  }
  return peer
}

function backend({ answer = 'v=0 answer', state = 'CONNECTING' } = {}) {
  const log = []
  const request = {
    post: async (path, body) => { log.push([path, body]); return { session_id: 'sess_test', state: 'CONNECTING' } },
    api: async (path, options) => {
      log.push([path, options?.method || 'GET'])
      if (path.startsWith('/ice-servers')) return { ice_servers: [{ urls: ['stun:s.example'] }] }
      if (path.includes('/signaling')) return { state, answer, ice: ['{"candidate":"candidate:agent"}'], next: 1 }
      return {}
    },
  }
  return { request, log }
}

const base = { deviceId: 'dev_bureau_0001', permissions: ['view_screen'], sleep: async () => {} }

describe('openSession', () => {
  it('attend l\'acceptation, envoie l\'offre AVANT les candidats, applique la réponse puis se connecte', async () => {
    const { request, log } = backend()
    const peers = []
    const states = []
    const link = await openSession({ ...base, request, onState: (s) => states.push(s), makePeer: (config) => { const peer = fakePeer(log); peer.config = config; peers.push(peer); return peer } })
    assert.deepEqual(states, ['asking', 'connecting', 'connected'])
    assert.equal(link.sessionId, 'sess_test')
    assert.deepEqual(peers[0].config, { iceServers: [{ urls: ['stun:s.example'] }] })
    const order = log.map(([path]) => path)
    assert.ok(order.indexOf('/signaling/offer') < order.indexOf('/signaling/ice'))
    assert.deepEqual(log.find(([path]) => path === '/signaling/ice')[1], { session_id: 'sess_test', candidate: JSON.stringify({ candidate: 'candidate:local' }) })
    assert.deepEqual(peers[0].remote, { type: 'answer', sdp: 'v=0 answer' })
    assert.deepEqual(peers[0].added, [{ candidate: 'candidate:agent' }])

    link.sendInput({ t: 'move', x: 0.5, y: 0.5 })
    assert.deepEqual(peers[0].channels.find((c) => c.label === 'control').sent, ['{"t":"move","x":0.5,"y":0.5}'])
    await link.close()
    assert.deepEqual(log.at(-1), ['/sessions/sess_test', 'DELETE'])
    assert.ok(peers[0].closed > 0)
    assert.equal(states.at(-1), 'closed')
  })

  it('transmet images, infos de l\'écran et curseur distant à leurs écouteurs', async () => {
    const { request, log } = backend()
    const frames = []
    const infos = []
    const cursors = []
    let peer
    await openSession({ ...base, request, onFrame: (d) => frames.push(d), onInfo: (i) => infos.push(i), onCursor: (c) => cursors.push(c), makePeer: () => { peer = fakePeer(log); return peer } })
    peer.channels.find((c) => c.label === 'frames').onmessage({ data: chunk(1, 0, 1, [255, 216]) })
    const control = peer.channels.find((c) => c.label === 'control')
    control.onmessage({ data: '{"t":"info","width":1920,"height":1080}' })
    control.onmessage({ data: '{"t":"cursor","x":0.25,"y":0.75}' })
    control.onmessage({ data: 'pas du json' })
    assert.deepEqual([...frames[0]], [255, 216])
    assert.deepEqual(infos, [{ t: 'info', width: 1920, height: 1080 }])
    assert.deepEqual(cursors, [{ t: 'cursor', x: 0.25, y: 0.75 }])
  })

  it('abandonne et ferme la session si le serveur l\'a terminée pendant la négociation', async () => {
    const { request, log } = backend({ answer: '', state: 'DISCONNECTED' })
    await assert.rejects(openSession({ ...base, request, makePeer: () => fakePeer(log) }), /La session a été fermée\./)
    assert.deepEqual(log.at(-1), ['/sessions/sess_test', 'DELETE'])
  })

  it('n\'ouvre rien si l\'appareil refuse (la création échoue)', async () => {
    let apiCalls = 0
    const request = { post: async () => { throw new Error('L\'utilisateur de l\'appareil a refusé la connexion.') }, api: async () => { apiCalls += 1 } }
    await assert.rejects(openSession({ ...base, request, makePeer: () => { throw new Error('pas de pair') } }), /refusé/)
    assert.equal(apiCalls, 0)
  })

  it('renégocie après une coupure, puis renonce après trois tentatives', async () => {
    const { request, log } = backend()
    const peers = []
    const states = []
    await openSession({ ...base, request, onState: (s) => states.push(s), makePeer: () => { const peer = fakePeer(log); peers.push(peer); return peer } })
    const drop = async () => {
      const peer = peers.at(-1)
      peer.connectionState = 'failed'
      peer.onconnectionstatechange()
      await until(() => ['connected', 'lost'].includes(states.at(-1)), 'reconnexion')
    }
    await drop()
    assert.equal(peers.length, 2)
    assert.ok(peers[0].closed > 0)
    assert.equal(log.filter(([path]) => path === '/sessions/sess_test/reconnect').length, 1)
    assert.deepEqual(states.slice(-3), ['reconnecting', 'connecting', 'connected'])
    await drop()
    await drop()
    assert.equal(peers.length, 4)
    await drop()
    assert.equal(states.at(-1), 'lost')
    assert.equal(peers.length, 4)
  })
})
