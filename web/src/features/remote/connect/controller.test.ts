import { describe, expect, it } from 'vitest'

import { ApiError } from '@/api/api-error'

import type { WebClientSession } from '@/api/generated/model'

import { mockApi } from '@/test/mock-api'

import { RemoteController, browserDeps, resolveRelayUrl } from './controller'
import { fakeDeps } from './fake-deps'
import { OPENED } from './fake-session'

const DEVICE = '123456789'

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0))
}

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((r) => {
    resolve = r
  })
  return { promise, resolve }
}

describe('resolveRelayUrl', () => {
  it('puts a bare path on the console origin with the matching WebSocket scheme', () => {
    expect(resolveRelayUrl('/ws/relay', { protocol: 'https:', host: 'a.example:8443' })).toBe(
      'wss://a.example:8443/ws/relay',
    )
    expect(resolveRelayUrl('/ws/relay', { protocol: 'http:', host: '127.0.0.1' })).toBe(
      'ws://127.0.0.1/ws/relay',
    )
  })

  it('takes an absolute ws(s) URL and refuses anything else', () => {
    const here = { protocol: 'https:', host: 'a.example' }
    expect(resolveRelayUrl('wss://relay.example/ws/relay', here)).toBe(
      'wss://relay.example/ws/relay',
    )
    expect(() => resolveRelayUrl('https://relay.example/ws/relay', here)).toThrow()
    expect(() => resolveRelayUrl('relay.example', here)).toThrow()
  })
})

describe('RemoteController', () => {
  it('opens a server session, then starts the peer session over the relay', async () => {
    const deps = fakeDeps()
    const controller = new RemoteController(DEVICE, deps.deps)
    controller.connect()
    expect(controller.phase.get()).toEqual({ kind: 'opening' })
    await settle()

    expect(deps.openSession).toHaveBeenCalledWith(DEVICE)
    expect(deps.createRelay).toHaveBeenCalledWith({
      url: 'wss://console.example/ws/relay',
      licenceKey: OPENED.server_pk,
      uuid: OPENED.uuid,
      targetId: DEVICE,
    })
    expect(deps.startSession).toHaveBeenCalledWith(
      expect.objectContaining({ kind: 'relay' }),
      expect.objectContaining({
        targetId: DEVICE,
        myId: 'alice',
        myName: 'Alice',
        serverPk: Uint8Array.from('k'.repeat(32), (c) => c.charCodeAt(0)),
        decoders: { vp9: true, av1: false, vp8: true },
      }),
      { viewOnly: false, lockAfterEnd: false },
    )
    expect(controller.phase.get()).toEqual({ kind: 'session', session: deps.sessions[0] })
  })

  it('targets the device it was asked for, whatever id the server echoes', async () => {
    const deps = fakeDeps(() => Promise.resolve({ ...OPENED, peer_id: '999' }))
    new RemoteController(DEVICE, deps.deps).connect()
    await settle()
    expect(deps.startSession).toHaveBeenCalledWith(
      expect.anything(),
      expect.objectContaining({ targetId: DEVICE }),
      expect.anything(),
    )
  })

  it('fails without opening a session when the browser decodes nothing', async () => {
    const deps = fakeDeps()
    deps.probeDecoders.mockResolvedValue({ vp9: false, av1: false, vp8: false })
    const controller = new RemoteController(DEVICE, deps.deps)
    controller.connect()
    await settle()
    expect(controller.phase.get()).toEqual({ kind: 'failed', failure: { kind: 'no_decoder' } })
    expect(deps.openSession).not.toHaveBeenCalled()
  })

  it('fails with the API error when the server refuses', async () => {
    const error = new ApiError(409, { code: 'device_offline' })
    const deps = fakeDeps(() => Promise.reject(error))
    const controller = new RemoteController(DEVICE, deps.deps)
    controller.connect()
    await settle()
    expect(controller.phase.get()).toEqual({ kind: 'failed', failure: { kind: 'api', error } })
    expect(deps.closeSession).not.toHaveBeenCalled()
  })

  it('closes the server session it cannot start, without touching the relay', async () => {
    const deps = fakeDeps(() => Promise.resolve({ ...OPENED, signed_peer_pk: '%%' }))
    const controller = new RemoteController(DEVICE, deps.deps)
    controller.connect()
    await settle()
    expect(controller.phase.get()).toMatchObject({ kind: 'failed', failure: { kind: 'error' } })
    expect(deps.createRelay).not.toHaveBeenCalled()
    expect(deps.closeSession).toHaveBeenCalledWith(OPENED.session_id)
  })

  it('closes the server session once when the peer session ends', async () => {
    const deps = fakeDeps()
    const controller = new RemoteController(DEVICE, deps.deps)
    controller.connect()
    await settle()
    deps.sessions[0]?.end({ by: 'link', reason: 'timeout' })
    controller.disconnect()
    expect(deps.closeSession).toHaveBeenCalledTimes(1)
    expect(deps.closeSession).toHaveBeenCalledWith(OPENED.session_id)
  })

  it('hangs up and closes the server session on disconnect', async () => {
    const deps = fakeDeps()
    const controller = new RemoteController(DEVICE, deps.deps)
    controller.connect()
    await settle()
    controller.disconnect()
    expect(deps.sessions[0]?.closed).toBe(1)
    expect(deps.closeSession).toHaveBeenCalledTimes(1)
  })

  it('closes a session that opens after the page left, and starts nothing', async () => {
    const opening = deferred<WebClientSession>()
    const deps = fakeDeps(() => opening.promise)
    const controller = new RemoteController(DEVICE, deps.deps)
    controller.connect()
    await settle()
    controller.disconnect()
    expect(controller.phase.get()).toEqual({ kind: 'idle' })
    opening.resolve(OPENED)
    await settle()
    expect(deps.startSession).not.toHaveBeenCalled()
    expect(deps.closeSession).toHaveBeenCalledWith(OPENED.session_id)
    expect(controller.phase.get()).toEqual({ kind: 'idle' })
  })

  it('reconnects with a fresh server session after the last one ended', async () => {
    const deps = fakeDeps()
    const controller = new RemoteController(DEVICE, deps.deps)
    controller.connect()
    await settle()
    deps.sessions[0]?.end({ by: 'link', reason: 'relay connection closed (code 1006)' })
    controller.connect()
    await settle()
    expect(deps.openSession).toHaveBeenCalledTimes(2)
    expect(controller.phase.get()).toEqual({ kind: 'session', session: deps.sessions[1] })
    expect(deps.closeSession).toHaveBeenCalledTimes(1)
  })
})

describe('browserDeps', () => {
  it('closes the server session with a DELETE that outlives the page', async () => {
    const fetchSpy = mockApi({ 'DELETE /admin/api/v1/web-client/sessions/sess-1': { status: 204 } })
    await browserDeps.closeSession('sess-1')
    expect(fetchSpy).toHaveBeenCalledWith(
      '/admin/api/v1/web-client/sessions/sess-1',
      expect.objectContaining({ method: 'DELETE', keepalive: true }),
    )
  })
})
