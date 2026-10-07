import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { KINDS, guessKind, loadKinds, machineSvg, saveKind } from '../web/machines.js'

const memory = () => { const data = {}; return { getItem: (k) => data[k] ?? null, setItem: (k, v) => { data[k] = v } } }

describe('illustrations des machines', () => {
  it('dessine les trois types, écran allumé ou éteint', () => {
    for (const { id, title } of KINDS) {
      const on = machineSvg(id, true)
      const off = machineSvg(id, false)
      assert.ok(on.includes(`aria-label="${title}"`) && on.startsWith('<svg') && on.endsWith('</svg>'))
      assert.ok(on.includes('scr-on') && !on.includes('scr-off'))
      assert.ok(off.includes('scr-off') && !off.includes('scr-on'))
    }
    assert.notEqual(machineSvg('desktop', true), machineSvg('laptop', true))
    assert.notEqual(machineSvg('mini', true), machineSvg('desktop', true))
    assert.ok(machineSvg('inconnu', true).includes('PC de bureau'), 'type inconnu : PC de bureau')
  })

  it('devine le type d\'après ce que l\'appareil annonce, sinon PC de bureau', () => {
    assert.equal(guessKind({ platform: 'windows-laptop' }), 'laptop')
    assert.equal(guessKind({ platform: 'windows', kind: 'Notebook' }), 'laptop')
    assert.equal(guessKind({ platform: 'windows-mini' }), 'mini')
    assert.equal(guessKind({ platform: 'windows' }), 'desktop')
    assert.equal(guessKind(null), 'desktop')
  })

  it('retient le choix par appareil, refuse un type inconnu et survit à un stockage indisponible', () => {
    const storage = memory()
    saveKind('dev_a', 'laptop', storage)
    saveKind('dev_b', 'mini', storage)
    saveKind('dev_c', 'tablette', storage)
    assert.deepEqual(loadKinds(storage), { dev_a: 'laptop', dev_b: 'mini' })
    assert.deepEqual(loadKinds({ getItem: () => '{pas du json' }), {})
    assert.deepEqual(loadKinds({ getItem: () => { throw new Error('bloqué') } }), {})
    assert.doesNotThrow(() => saveKind('dev_a', 'desktop', { getItem: () => null, setItem: () => { throw new Error('plein') } }))
  })
})
