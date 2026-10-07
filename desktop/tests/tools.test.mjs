import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { AutoQuality, CLIPBOARD_MAX_BYTES, ClipboardSync, FpsMeter, LatencyMeter, PRESETS, SHORTCUTS, latencyTone, screenLabel, shortcutMessages, streamMessage, stuckKeys } from '../web/tools.js'

describe('qualité de l\'image', () => {
  it('les préréglages sont croissants et dans les bornes de l\'agent', () => {
    const [a, b, c] = [PRESETS.economy, PRESETS.balanced, PRESETS.high]
    assert.ok(a.fps < b.fps && b.fps < c.fps && a.quality < b.quality && b.quality < c.quality && a.width < b.width && b.width < c.width)
    for (const p of [a, b, c]) assert.ok(p.fps >= 2 && p.fps <= 30 && p.quality >= 20 && p.quality <= 90 && p.width >= 640 && p.width <= 3840)
    assert.deepEqual(streamMessage(b), { t: 'stream', fps: 10, quality: 55, width: 1600 })
  })

  it('auto : baisse après trois mauvais échantillons de suite, pas avant', () => {
    const auto = new AutoQuality('balanced')
    assert.equal(auto.sample({ rtt: 400, fps: 9 }), null)
    assert.equal(auto.sample({ rtt: 400, fps: 9 }), null)
    assert.equal(auto.sample({ rtt: 400, fps: 9 }), PRESETS.economy)
    assert.equal(auto.sample({ rtt: 900, fps: 1 }), null, 'déjà au plus bas : rien de plus à baisser')
    for (let i = 0; i < 5; i += 1) assert.equal(auto.sample({ rtt: 900, fps: 1 }), null)
  })

  it('auto : un seul bon ou mauvais échantillon isolé ne change rien (pas d\'oscillation)', () => {
    const auto = new AutoQuality('balanced')
    for (let i = 0; i < 20; i += 1) assert.equal(auto.sample({ rtt: i % 2 ? 400 : 20, fps: i % 2 ? 2 : 10 }), null)
  })

  it('auto : remonte après six bons échantillons, jusqu\'au maximum seulement', () => {
    const auto = new AutoQuality('economy')
    const good = () => auto.sample({ rtt: 30, fps: auto.preset.fps })
    for (let i = 0; i < 5; i += 1) assert.equal(good(), null)
    assert.equal(good(), PRESETS.balanced)
    for (let i = 0; i < 5; i += 1) assert.equal(good(), null)
    assert.equal(good(), PRESETS.high)
    assert.equal(auto.preset, PRESETS.high)
    for (let i = 0; i < 12; i += 1) assert.equal(good(), null)
    assert.equal(auto.preset, PRESETS.high)
  })

  it('auto : sans mesure de latence, seules les images reçues comptent', () => {
    const auto = new AutoQuality('high')
    for (let i = 0; i < 2; i += 1) assert.equal(auto.sample({ rtt: null, fps: 3 }), null)
    assert.equal(auto.sample({ rtt: null, fps: 3 }), PRESETS.balanced)
  })
})

describe('latence et images par seconde', () => {
  it('chaque ping a son identifiant et la réponse donne le temps d\'aller-retour', () => {
    const meter = new LatencyMeter()
    const a = meter.ping(1000)
    const b = meter.ping(2000)
    assert.deepEqual([a, b], [{ t: 'ping', id: 1 }, { t: 'ping', id: 2 }])
    assert.equal(meter.pong(1, 1042), 42)
    assert.equal(meter.pong(1, 1100), null, 'une réponse déjà traitée est ignorée')
    assert.equal(meter.pong(99, 1100), null, 'un identifiant inconnu aussi')
    assert.equal(meter.pong(2, 2058), 58)
    assert.equal(meter.average, 50)
  })

  it('la moyenne ne garde que les cinq dernières mesures et les anciens ping sans réponse sont oubliés', () => {
    const meter = new LatencyMeter()
    for (let i = 0; i < 7; i += 1) { const { id } = meter.ping(0); meter.pong(id, 100 * (i + 1)) }
    assert.equal(meter.average, 500)        // moyenne de 300, 400, 500, 600, 700
    for (let i = 0; i < 30; i += 1) meter.ping(0)
    assert.ok(meter.sent.size <= 10)
    assert.equal(new LatencyMeter().average, null)
  })

  it('compte les images par seconde sur la durée écoulée', () => {
    const fps = new FpsMeter()
    assert.equal(fps.tick(0), null)
    for (let i = 0; i < 9; i += 1) fps.hit()
    assert.equal(fps.tick(1000), 9)
    for (let i = 0; i < 5; i += 1) fps.hit()
    assert.equal(fps.tick(3000), 2.5)
    assert.equal(fps.tick(3000), null, 'durée nulle : pas de division par zéro')
  })

  it('la couleur suit la latence', () => {
    assert.deepEqual([null, 12, 79, 80, 199, 200, 900].map(latencyTone), ['idle', 'ok', 'ok', 'warn', 'warn', 'bad', 'bad'])
  })
})

describe('raccourcis clavier', () => {
  const sequence = (id) => shortcutMessages(id).map((m) => `${m.down ? '+' : '-'}${m.code}`)

  it('chaque raccourci presse les modificateurs, tape la touche puis relâche dans l\'ordre inverse', () => {
    assert.deepEqual(sequence('win'), ['+MetaLeft', '-MetaLeft'])
    assert.deepEqual(sequence('alttab'), ['+AltLeft', '+Tab', '-Tab', '-AltLeft'])
    assert.deepEqual(sequence('taskmgr'), ['+ControlLeft', '+ShiftLeft', '+Escape', '-Escape', '-ShiftLeft', '-ControlLeft'])
    assert.deepEqual(sequence('altf4'), ['+AltLeft', '+F4', '-F4', '-AltLeft'])
    assert.deepEqual(shortcutMessages('inconnu'), [])
  })

  it('tous les messages sont des touches complètes, et chaque raccourci relâche tout ce qu\'il a pressé', () => {
    for (const { id } of SHORTCUTS) {
      const messages = shortcutMessages(id)
      assert.ok(messages.every((m) => m.t === 'key' && typeof m.down === 'boolean' && m.code && m.key), id)
      assert.deepEqual(stuckKeys(messages), [], id)
    }
  })

  it('si l\'envoi s\'arrête en route, les touches restées enfoncées sont relâchées (la dernière pressée d\'abord)', () => {
    const half = shortcutMessages('taskmgr').slice(0, 3)       // Ctrl, Maj, Échap enfoncés
    assert.deepEqual(stuckKeys(half).map((m) => `${m.down ? '+' : '-'}${m.code}`), ['-Escape', '-ShiftLeft', '-ControlLeft'])
  })
})

describe('presse-papiers', () => {
  it('ne partage que ce qui change après l\'ouverture, sans écho', () => {
    const sync = new ClipboardSync('déjà là')
    assert.equal(sync.changed('déjà là'), null)
    assert.equal(sync.changed('nouveau'), 'nouveau')
    assert.equal(sync.changed('nouveau'), null)
    assert.equal(sync.received('venu de l\'appareil'), 'venu de l\'appareil')
    assert.equal(sync.changed('venu de l\'appareil'), null, 'ce qu\'on vient de recevoir ne repart pas')
    assert.equal(sync.changed(null), null)
    assert.equal(sync.changed(undefined), null)
  })

  it('refuse le vide et l\'excès (en octets, pas en caractères)', () => {
    const sync = new ClipboardSync(null)
    assert.equal(sync.changed(''), null)
    assert.equal(sync.received(''), null)
    assert.equal(sync.changed('é'.repeat(CLIPBOARD_MAX_BYTES / 2 + 1)), null, 'deux octets par caractère')
    assert.equal(sync.received('é'.repeat(CLIPBOARD_MAX_BYTES / 2 + 1)), null)
    assert.equal(sync.changed('x'.repeat(CLIPBOARD_MAX_BYTES)), 'x'.repeat(CLIPBOARD_MAX_BYTES), 'la limite est incluse')
    assert.equal(sync.received(42), null)
  })
})

describe('écrans', () => {
  it('numérote les écrans et marque le principal', () => {
    assert.equal(screenLabel({ index: 0, primary: true, width: 1920, height: 1080 }, 2), 'Écran 1 (principal) · 1920×1080')
    assert.equal(screenLabel({ index: 1, primary: false, width: 800, height: 600 }, 2), 'Écran 2 · 800×600')
    assert.equal(screenLabel({ index: 0, primary: true, width: 1280, height: 720 }, 1), 'Écran (principal) · 1280×720')
  })
})
