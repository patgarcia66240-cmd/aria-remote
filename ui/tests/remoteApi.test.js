import { describe, expect, it, vi } from 'vitest'
import { FrameAssembler, keyMessage, openSession, pointerMessage, pointerPosition, wheelMessage } from '../src/components/remoteApi'

function chunk(id, index, count, bytes) {
  const buffer = new ArrayBuffer(8 + bytes.length)
  const view = new DataView(buffer)
  view.setUint32(0, id)
  view.setUint16(4, index)
  view.setUint16(6, count)
  new Uint8Array(buffer, 8).set(bytes)
  return buffer
}

describe('FrameAssembler', () => {
  it('recompose une image à partir de ses morceaux, dans l\'ordre', () => {
    const assembler = new FrameAssembler()
    expect(assembler.push(chunk(7, 0, 3, [1, 2]))).toBeNull()
    expect(assembler.push(chunk(7, 1, 3, [3]))).toBeNull()
    expect([...assembler.push(chunk(7, 2, 3, [4, 5]))]).toEqual([1, 2, 3, 4, 5])
    expect([...assembler.push(chunk(8, 0, 1, [9]))]).toEqual([9])
  })

  it('accepte les morceaux dans le désordre et ignore une image plus ancienne que celle déjà affichée', () => {
    const assembler = new FrameAssembler()
    expect(assembler.push(chunk(5, 1, 2, [3, 4]))).toBeNull()
    expect([...assembler.push(chunk(5, 0, 2, [1, 2]))]).toEqual([1, 2, 3, 4])
    expect(assembler.push(chunk(4, 0, 1, [9]))).toBeNull()            // plus ancienne : inutile
    expect([...assembler.push(chunk(6, 0, 1, [7]))]).toEqual([7])
  })

  it('abandonne une image incomplète quand la suivante commence, et ignore les messages invalides', () => {
    const assembler = new FrameAssembler()
    assembler.push(chunk(1, 0, 2, [1]))
    expect([...assembler.push(chunk(2, 0, 1, [7]))]).toEqual([7])
    assembler.push(chunk(3, 0, 3, [1]))
    expect(assembler.push(chunk(3, 2, 3, [3]))).toBeNull()      // le morceau 1 manque : rien n'est recomposé
    expect(assembler.push(new ArrayBuffer(3))).toBeNull()
    expect(assembler.push(chunk(4, 5, 2, [1]))).toBeNull()
    expect(assembler.push(null)).toBeNull()
  })
})

const rect = { left: 100, top: 50, width: 400, height: 200 }
const element = { getBoundingClientRect: () => rect }

describe('entrées du contrôle à distance', () => {
  it('normalise la position du pointeur entre 0 et 1, même hors de l\'écran', () => {
    expect(pointerPosition({ clientX: 300, clientY: 150 }, element)).toEqual({ x: 0.5, y: 0.5 })
    expect(pointerPosition({ clientX: 0, clientY: 999 }, element)).toEqual({ x: 0, y: 1 })
    expect(pointerPosition({ clientX: 1, clientY: 1 }, { getBoundingClientRect: () => ({ left: 0, top: 0, width: 0, height: 0 }) })).toEqual({ x: 0, y: 0 })
  })

  it('n\'envoie le bouton que pour un appui ou un relâchement, et borne la molette', () => {
    expect(pointerMessage('move', { clientX: 100, clientY: 50, button: 0 }, element)).toEqual({ t: 'move', x: 0, y: 0 })
    expect(pointerMessage('down', { clientX: 500, clientY: 250, button: 2 }, element)).toEqual({ t: 'down', x: 1, y: 1, button: 2 })
    expect(wheelMessage({ deltaX: 0, deltaY: 5000 })).toEqual({ t: 'wheel', dx: 0, dy: 400 })
    expect(wheelMessage({ deltaX: -3, deltaY: -9000 })).toEqual({ t: 'wheel', dx: -3, dy: -400 })
    expect(keyMessage({ code: 'KeyA', key: 'a' }, true)).toEqual({ t: 'key', down: true, code: 'KeyA', key: 'a' })
  })
})

function fakePeer(log) {
  const peer = {
    connectionState: 'new',
    localDescription: null,
    channels: [],
    createDataChannel: vi.fn((label) => { const channel = { label, readyState: 'open', send: vi.fn() }; peer.channels.push(channel); return channel }),
    createOffer: vi.fn(async () => ({ type: 'offer', sdp: 'v=0 offer' })),
    setLocalDescription: vi.fn(async (description) => {
      peer.localDescription = description
      peer.onicecandidate?.({ candidate: { candidate: 'candidate:local' } })     // un candidat sort avant que l'offre soit partie
    }),
    setRemoteDescription: vi.fn(async (description) => { log.push(['remote', description.sdp]); peer.connectionState = 'connected' }),
    addIceCandidate: vi.fn(async () => {}),
    close: vi.fn(),
  }
  return peer
}

function backend({ answer = 'v=0 answer', state = 'CONNECTING' } = {}) {
  const log = []
  const request = {
    post: vi.fn(async (path, body) => { log.push([path, body]); return { session_id: 'sess_test', state: 'CONNECTING' } }),
    api: vi.fn(async (path, options) => {
      log.push([path, options?.method || 'GET'])
      if (path.startsWith('/ice-servers')) return { ice_servers: [{ urls: ['stun:s.example'] }] }
      if (path.includes('/signaling')) return { state, answer, ice: ['{"candidate":"candidate:agent"}'], next: 1 }
      return {}
    }),
  }
  return { request, log }
}

describe('openSession', () => {
  it('attend l\'acceptation, envoie l\'offre AVANT les candidats, applique la réponse puis se connecte', async () => {
    const { request, log } = backend()
    const peers = []
    const states = []
    const link = await openSession({
      deviceId: 'dev_bureau_0001', permissions: ['view_screen'], request, sleep: async () => {}, onState: (s) => states.push(s),
      makePeer: (config) => { const peer = fakePeer(log); peer.config = config; peers.push(peer); return peer },
    })
    expect(states).toEqual(['asking', 'connecting', 'connected'])
    expect(link.sessionId).toBe('sess_test')
    expect(peers[0].config).toEqual({ iceServers: [{ urls: ['stun:s.example'] }] })
    const order = log.map(([path]) => path)
    expect(order.indexOf('/signaling/offer')).toBeLessThan(order.indexOf('/signaling/ice'))
    expect(log.find(([path]) => path === '/signaling/ice')[1]).toEqual({ session_id: 'sess_test', candidate: JSON.stringify({ candidate: 'candidate:local' }) })
    expect(peers[0].setRemoteDescription).toHaveBeenCalledWith({ type: 'answer', sdp: 'v=0 answer' })
    expect(peers[0].addIceCandidate).toHaveBeenCalledWith({ candidate: 'candidate:agent' })

    link.sendInput({ t: 'move', x: 0.5, y: 0.5 })
    expect(peers[0].channels.find((c) => c.label === 'control').send).toHaveBeenCalledWith('{"t":"move","x":0.5,"y":0.5}')
    await link.close()
    expect(log.at(-1)).toEqual(['/sessions/sess_test', 'DELETE'])
    expect(peers[0].close).toHaveBeenCalled()
    expect(states.at(-1)).toBe('closed')
  })

  it('transmet les images binaires et les infos de l\'agent aux écouteurs', async () => {
    const { request, log } = backend()
    const frames = []
    const infos = []
    let peer
    await openSession({
      deviceId: 'dev_bureau_0001', permissions: ['view_screen'], request, sleep: async () => {}, onFrame: (data) => frames.push(data), onInfo: (info) => infos.push(info),
      makePeer: () => { peer = fakePeer(log); return peer },
    })
    peer.channels.find((c) => c.label === 'frames').onmessage({ data: chunk(1, 0, 1, [255, 216]) })
    peer.channels.find((c) => c.label === 'control').onmessage({ data: '{"t":"info","width":1920,"height":1080}' })
    peer.channels.find((c) => c.label === 'control').onmessage({ data: 'pas du json' })
    expect(frames).toHaveLength(1)
    expect([...frames[0]]).toEqual([255, 216])
    expect(infos).toEqual([{ t: 'info', width: 1920, height: 1080 }])
  })

  it('envoie la position du curseur distant à son propre écouteur, sans écraser les infos de l\'écran', async () => {
    const { request, log } = backend()
    const infos = []
    const cursors = []
    let peer
    await openSession({
      deviceId: 'dev_bureau_0001', permissions: ['view_screen'], request, sleep: async () => {}, onInfo: (info) => infos.push(info), onCursor: (c) => cursors.push(c),
      makePeer: () => { peer = fakePeer(log); return peer },
    })
    const control = peer.channels.find((c) => c.label === 'control')
    control.onmessage({ data: '{"t":"info","width":1920,"height":1080}' })
    control.onmessage({ data: '{"t":"cursor","x":0.25,"y":0.75}' })
    expect(infos).toEqual([{ t: 'info', width: 1920, height: 1080 }])
    expect(cursors).toEqual([{ t: 'cursor', x: 0.25, y: 0.75 }])
  })

  it('abandonne et ferme la session si le serveur l\'a terminée pendant la négociation', async () => {
    const { request, log } = backend({ answer: '', state: 'DISCONNECTED' })
    await expect(openSession({ deviceId: 'dev_bureau_0001', permissions: ['view_screen'], request, sleep: async () => {}, makePeer: () => fakePeer(log) }))
      .rejects.toThrow('La session a été fermée.')
    expect(log.at(-1)).toEqual(['/sessions/sess_test', 'DELETE'])
  })

  it('n\'ouvre rien si l\'appareil refuse (la création échoue)', async () => {
    const request = { post: vi.fn(async () => { throw new Error('L\'utilisateur de l\'appareil a refusé la connexion.') }), api: vi.fn() }
    await expect(openSession({ deviceId: 'dev_bureau_0001', permissions: ['view_screen'], request, makePeer: () => { throw new Error('pas de pair') } }))
      .rejects.toThrow('refusé')
    expect(request.api).not.toHaveBeenCalled()
  })

  it('renégocie après une coupure, puis renonce après trois tentatives', async () => {
    const { request, log } = backend()
    const peers = []
    const states = []
    await openSession({
      deviceId: 'dev_bureau_0001', permissions: ['view_screen'], request, sleep: async () => {}, onState: (s) => states.push(s),
      makePeer: () => { const peer = fakePeer(log); peers.push(peer); return peer },
    })
    const drop = async () => {
      const peer = peers.at(-1)
      peer.connectionState = 'failed'
      peer.onconnectionstatechange()
      await vi.waitFor(() => expect(['connected', 'lost']).toContain(states.at(-1)))
    }
    await drop()
    expect(peers).toHaveLength(2)
    expect(peers[0].close).toHaveBeenCalled()
    expect(log.filter(([path]) => path === '/sessions/sess_test/reconnect')).toHaveLength(1)
    expect(states.slice(-3)).toEqual(['reconnecting', 'connecting', 'connected'])
    await drop()
    await drop()
    expect(peers).toHaveLength(4)
    await drop()
    expect(states.at(-1)).toBe('lost')
    expect(peers).toHaveLength(4)
  })
})
