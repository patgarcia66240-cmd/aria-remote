import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import RemoteControl from '../src/components/RemoteControl'

let host
let root

beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true
})

afterEach(async () => {
  await act(async () => root?.unmount())
  host?.remove()
  vi.unstubAllGlobals()
})

const DEVICES = [
  { device_id: 'dev_bureau_0001', name: 'Bureau', paired: true, online: true, fingerprint: 'abcd1234abcd1234', granted: ['view_screen', 'control_mouse'] },
  { device_id: 'dev_salon_00001', name: 'Salon', paired: true, online: false, fingerprint: '1111222233334444', granted: ['view_screen'] },
]

async function render({ devices = DEVICES, status = { agent_installed: true, agents_online: 1, turn_configured: false }, pairResult } = {}) {
  const fetchMock = vi.fn(async (url) => {
    if (url === '/api/remote/status') return new Response(JSON.stringify(status))
    if (url === '/api/remote/devices') return new Response(JSON.stringify({ devices }))
    if (url === '/api/remote/pair') return pairResult || new Response(JSON.stringify({ device: devices[0] }))
    return new Response('{}', { status: 404 })
  })
  vi.stubGlobal('fetch', fetchMock)
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  await act(async () => root.render(<RemoteControl isActive />))
  return fetchMock
}

describe('Contrôle à distance (page Ordinateur)', () => {
  it('liste les appareils et ne propose de se connecter qu\'à un appareil appairé et en ligne', async () => {
    await render()
    expect(host.textContent).toContain('Bureau')
    expect(host.textContent).toContain('Disponible')
    expect(host.textContent).toContain('Hors ligne')
    const [bureau, salon] = [...host.querySelectorAll('li button')]
    expect(bureau.disabled).toBe(false)
    expect(salon.disabled).toBe(true)
  })

  it('ne propose que les permissions que l\'appareil autorise', async () => {
    await render()
    const rows = host.querySelectorAll('li')
    expect(rows[0].textContent).toContain('Souris')
    expect(rows[0].textContent).not.toContain('Clavier')
    expect(rows[1].textContent).toContain('n\'autorise que la consultation')
  })

  it('appaire avec le code à 6 chiffres, en ignorant tout ce qui n\'est pas un chiffre', async () => {
    const fetchMock = await render()
    const input = host.querySelector('#remote-code')
    const submit = host.querySelector('form button')
    expect(submit.disabled).toBe(true)
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set
    await act(async () => { setValue.call(input, '48a392b1'); input.dispatchEvent(new Event('input', { bubbles: true })) })
    expect(input.value).toBe('483921')
    await act(async () => host.querySelector('form').dispatchEvent(new Event('submit', { bubbles: true, cancelable: true })))
    const call = fetchMock.mock.calls.find(([url]) => url === '/api/remote/pair')
    expect(JSON.parse(call[1].body)).toEqual({ code: '483921' })
    expect(host.textContent).toContain('Bureau est appairé.')
  })

  it('affiche l\'erreur du serveur quand le code est refusé', async () => {
    await render({ pairResult: new Response(JSON.stringify({ detail: 'Code invalide ou expiré.' }), { status: 400 }) })
    const input = host.querySelector('#remote-code')
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set
    await act(async () => { setValue.call(input, '000000'); input.dispatchEvent(new Event('input', { bubbles: true })) })
    await act(async () => host.querySelector('form').dispatchEvent(new Event('submit', { bubbles: true, cancelable: true })))
    expect(host.querySelector('[role="alert"]').textContent).toContain('Code invalide ou expiré.')
  })

  it('explique comment installer l\'agent quand aucun appareil n\'existe', async () => {
    await render({ devices: [], status: { agent_installed: false, agents_online: 0, turn_configured: false } })
    expect(host.textContent).toContain('remote-agent/')
    expect(host.textContent).toContain('REMOTE_RENDEZVOUS_URL')
  })

  it('en mode serveur de rendez-vous, explique que les appareils s\'y connectent seuls', async () => {
    await render({ devices: [], status: { mode: 'rendezvous', server: 'https://rv.exemple.fr', agent_installed: false, agents_online: 0, turn_configured: true } })
    expect(host.textContent).toContain('serveur de rendez-vous https://rv.exemple.fr')
    expect(host.textContent).toContain('aucun port à ouvrir')
    expect(host.textContent).not.toContain('REMOTE_STUN_URLS')
  })
})
