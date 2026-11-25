// Écran distant en H.264 (web/video.js) : `node --test tests` depuis desktop/.
import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { canShowVideo, h264Codecs, showVideo, videoStalled } from '../web/video.js'

const codec = (mimeType, sdpFmtpLine = '') => ({ mimeType, clockRate: 90000, sdpFmtpLine })
const MODE1 = 'level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f'
const MODE0 = 'level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=42e01f'
const receiver = (...codecs) => ({ getCapabilities: () => ({ codecs }) })

describe('codecs H.264 décodables', () => {
  it('ne garde que le H.264 en paquétisation 1 (celui que l\'agent encode)', () => {
    const found = h264Codecs(receiver(codec('video/VP8'), codec('video/H264', MODE0), codec('video/H264', MODE1), codec('video/h264', MODE1 + ';x=1'), codec('video/AV1')))
    assert.equal(found.length, 2)
    assert.ok(found.every((c) => /packetization-mode=1/.test(c.sdpFmtpLine)))
  })

  it('renvoie une liste vide quand le navigateur ne sait rien dire ou plante', () => {
    assert.deepEqual(h264Codecs(undefined), [])
    assert.deepEqual(h264Codecs({}), [])
    assert.deepEqual(h264Codecs({ getCapabilities: () => { throw new Error('refusé') } }), [])
    assert.deepEqual(h264Codecs(receiver(codec('video/VP8'))), [])
  })

  it('l\'affichage exige le décodage H.264 ET un signal par image (requestVideoFrameCallback)', () => {
    const withCallback = { prototype: { requestVideoFrameCallback() {} } }
    assert.equal(canShowVideo({ receiver: receiver(codec('video/H264', MODE1)), video: withCallback }), true)
    assert.equal(canShowVideo({ receiver: receiver(codec('video/H264', MODE1)), video: { prototype: {} } }), false)
    assert.equal(canShowVideo({ receiver: receiver(codec('video/VP8')), video: withCallback }), false)
    assert.equal(canShowVideo({ receiver: receiver(codec('video/H264', MODE1)), video: undefined }), false)
  })
})

function fakeVideoWorld() {
  const draws = []
  const video = {
    style: {}, removed: false, played: 0, videoWidth: 0, videoHeight: 0, srcObject: null, callbacks: [],
    setAttribute() {},
    play() { video.played += 1; return Promise.resolve() },
    remove() { video.removed = true },
    requestVideoFrameCallback(fn) { video.callbacks.push(fn) },
    tick() { const fn = video.callbacks.shift(); fn?.() },
  }
  const canvas = {
    width: 16, height: 9,
    parentNode: { children: [], append(node) { this.children.push(node) } },
    getContext: () => ({ drawImage: (source, x, y) => draws.push([source, x, y]) }),
  }
  const doc = { createElement: () => video }
  return { video, canvas, doc, draws }
}

describe("détection d'une vidéo bloquée", () => {
  const at = (now, lastFrameAt, since = 0) => videoStalled({ now, lastFrameAt, since, stallMs: 5000 })

  it("tant que des images arrivent, ce n'est pas bloqué", () => {
    assert.equal(at(10_000, 8_000), false)
    assert.equal(at(12_999, 8_000), false)
  })

  it('5 s sans image = bloqué', () => {
    assert.equal(at(13_000, 8_000), true)
  })

  it('avant la première image, on compte depuis le démarrage de la vidéo', () => {
    assert.equal(at(4_000, null, 0), false)
    assert.equal(at(5_000, null, 0), true)
    assert.equal(at(6_000, null, 2_000), false)
  })

  it('un démarrage plus récent que la dernière image repart de zéro (page redevenue visible)', () => {
    assert.equal(at(20_000, 8_000, 19_000), false)
  })
})

describe('showVideo', () => {
  it('décode dans une vidéo invisible et recopie chaque image dans le canvas, à sa taille', () => {
    const { video, canvas, doc, draws } = fakeVideoWorld()
    let clock = 1000
    let frames = 0
    const shown = showVideo({ id: 'flux' }, canvas, () => { frames += 1 }, { doc, now: () => clock })
    assert.equal(video.srcObject.id, 'flux')
    assert.equal(video.muted, true)
    assert.equal(video.played, 1)
    assert.equal(video.style.opacity, '0', 'la vidéo ne s\'affiche jamais elle-même')
    assert.ok(canvas.parentNode.children.includes(video), 'dans la page : sinon certains moteurs ne décodent pas')
    assert.equal(shown.lastFrameAt(), null, 'aucune image tant que la vidéo n\'a pas de taille')

    video.tick()
    assert.deepEqual(draws, [], 'une vidéo sans dimensions ne dessine rien')
    video.videoWidth = 1600
    video.videoHeight = 900
    clock = 2500
    video.tick()
    assert.deepEqual([canvas.width, canvas.height], [1600, 900])
    assert.equal(draws.length, 1)
    assert.equal(frames, 1)
    assert.equal(shown.lastFrameAt(), 2500)
    video.tick()
    assert.equal(frames, 2, 'le signal se réarme à chaque image')
  })

  it('stop() arrête tout : plus de dessin, flux détaché, vidéo retirée', () => {
    const { video, canvas, doc, draws } = fakeVideoWorld()
    video.videoWidth = 640
    video.videoHeight = 360
    const shown = showVideo({}, canvas, () => {}, { doc })
    video.tick()
    assert.equal(draws.length, 1)
    shown.stop()
    assert.equal(video.srcObject, null)
    assert.equal(video.removed, true)
    video.callbacks.push(() => {})
    video.tick()
    assert.equal(draws.length, 1)
  })
})
