import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { activeTab, canControl, canHost, clockText, formatCode, hasTabs, hostScreen, permissionText, secondsLeft } from '../web/host.js'

describe('modes', () => {
  it('contrôler seulement, être contrôlé seulement, ou les deux', () => {
    assert.deepEqual([canControl('control'), canHost('control'), hasTabs('control')], [true, false, false])
    assert.deepEqual([canControl('host'), canHost('host'), hasTabs('host')], [false, true, false])
    assert.deepEqual([canControl('both'), canHost('both'), hasTabs('both')], [true, true, true])
  })

  it('l\'onglet montré existe toujours dans le mode choisi', () => {
    assert.equal(activeTab('control', 'host'), 'control')
    assert.equal(activeTab('host', 'control'), 'host')
    assert.equal(activeTab('both', 'host'), 'host')
    assert.equal(activeTab('both', undefined), 'control')
  })
})

describe('code d\'appairage', () => {
  it('se lit en deux groupes et le décompte ne devient jamais négatif', () => {
    assert.equal(formatCode('483921'), '483 921')
    assert.equal(formatCode(null), '')
    assert.equal(secondsLeft(10_000, 4_000), 6)
    assert.equal(secondsLeft(10_000, 20_000), 0)
    assert.equal(secondsLeft(undefined, 0), 0)
    assert.equal(clockText(125), '2:05')
  })
})

describe('écran de « Cet appareil »', () => {
  const now = 1_000_000
  it('suit le lien de l\'agent avec le serveur', () => {
    assert.equal(hostScreen(null, now), 'off')
    assert.equal(hostScreen({ link: 'setup' }, now), 'setup')
    assert.equal(hostScreen({ link: 'connecting' }, now), 'connecting')
    assert.equal(hostScreen({ link: 'online' }, now), 'ready')
    assert.equal(hostScreen({ link: 'error' }, now), 'error')
  })

  it('montre le code tant qu\'il est valable, puis « prêt » une fois appairé', () => {
    assert.equal(hostScreen({ link: 'unpaired', pairing_code: '123456', pairing_expires_ms: now + 5000 }, now), 'pairing')
    assert.equal(hostScreen({ link: 'online', pairing_code: '123456', pairing_expires_ms: now + 5000 }, now), 'pairing')
    assert.equal(hostScreen({ link: 'online', pairing_code: '123456', pairing_expires_ms: now - 1 }, now), 'ready')
  })

  it('décrit en clair ce que le contrôleur demande', () => {
    assert.equal(permissionText(['view_screen', 'control_mouse']), 'voir l\'écran, utiliser la souris')
    assert.equal(permissionText(undefined), '')
  })
})
