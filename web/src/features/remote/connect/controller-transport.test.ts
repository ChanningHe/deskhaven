import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { WebClientSession } from '@/api/generated/model'

import type { Credential } from '../handshake/handshake'
import { SETTINGS_KEY, type RemoteSettings } from '../settings/settings'
import { answerEndpoint } from '../transport/fake-rtc'
import { RemoteController } from './controller'
import { fakeDeps, fakeWebRtc } from './fake-deps'
import { OPENED } from './fake-session'

const DEVICE = '123456789'
const ANSWERED: WebClientSession = { ...OPENED, answer: answerEndpoint() }
const CREDENTIAL: Credential = { salt: 'salt', salted: new Uint8Array(32) }

beforeEach(() => {
  vi.useFakeTimers()
})

afterEach(() => {
  vi.useRealTimers()
})

function setup(stored?: Partial<RemoteSettings>) {
  const f = fakeDeps(() => Promise.resolve(ANSWERED))
  const w = fakeWebRtc()
  f.deps.webrtc = w.webrtc
  if (stored) f.storage.setItem(SETTINGS_KEY, JSON.stringify({ v: 1, settings: stored }))
  const controller = new RemoteController(DEVICE, f.deps)
  return { ...f, w, controller }
}

type Setup = ReturnType<typeof setup>

function session(s: Setup, index: number) {
  const found = s.sessions[index]
  if (!found) throw new Error(`no session ${String(index)}`)
  return found
}

/** Over WebRTC, logged in with a typed password. */
async function live(s: Setup) {
  s.controller.connect()
  await vi.advanceTimersByTimeAsync(0)
  s.w.pcs[0]?.channel?.open()
  await vi.advanceTimersByTimeAsync(0)
  s.params(0).onHeard?.(Date.now())
  s.params(0).credentials?.keep(CREDENTIAL)
  session(s, 0).activate()
  expect(s.controller.transport.get()).toBe('webrtc')
}

describe('RemoteController: the transport setting', () => {
  it('offers no WebRTC with the relay setting and goes by the relay alone', async () => {
    const s = setup({ transport: 'relay' })
    s.controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    expect(s.w.createOfferer).not.toHaveBeenCalled()
    expect(s.openSession.mock.calls[0]).toEqual([DEVICE])
    expect(s.startSession.mock.calls[0]?.[0].kind).toBe('relay')
    expect(s.controller.transport.get()).toBe('relay')
  })

  it('offers WebRTC with the auto setting', async () => {
    const s = setup({ transport: 'auto' })
    s.controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    expect(s.w.createOfferer).toHaveBeenCalledTimes(1)
    expect(s.openSession.mock.calls[0]?.[1]).toMatch(/^webrtc:\/\//)
  })

  it('reconnects a live session once, by the relay, in the same peer session and credential', async () => {
    const s = setup()
    await live(s)
    s.controller.updateSettings({ transport: 'relay' })
    await vi.advanceTimersByTimeAsync(0)
    expect(session(s, 0).closed).toBe(1)
    expect(s.closeSession).toHaveBeenCalledWith(OPENED.session_id)
    expect(s.openSession).toHaveBeenCalledTimes(2)
    expect(s.openSession.mock.calls[1]).toEqual([DEVICE])
    expect(s.w.createOfferer).toHaveBeenCalledTimes(1)
    expect(s.startSession.mock.calls[1]?.[0].kind).toBe('relay')
    expect(s.controller.transport.get()).toBe('relay')
    expect(s.controller.phase.get()).toEqual({ kind: 'session', session: s.sessions[1] })
    expect(s.params(1).sessionId).toBe(s.params(0).sessionId)
    expect(s.params(1).credentials?.take()).toBe(CREDENTIAL)

    s.controller.updateSettings({ transport: 'relay' })
    s.controller.updateSettings({ fps: 45 })
    await vi.advanceTimersByTimeAsync(60_000)
    expect(s.openSession).toHaveBeenCalledTimes(2)
  })

  it('offers WebRTC again when switched back to auto', async () => {
    const s = setup({ transport: 'relay' })
    s.controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    session(s, 0).activate()
    s.controller.updateSettings({ transport: 'auto' })
    await vi.advanceTimersByTimeAsync(0)
    expect(s.openSession).toHaveBeenCalledTimes(2)
    expect(s.openSession.mock.calls[1]?.[1]).toMatch(/^webrtc:\/\//)
  })

  it('tells the peer nothing about it', async () => {
    const s = setup()
    await live(s)
    session(s, 0).patches.length = 0
    s.controller.updateSettings({ transport: 'relay' })
    expect(session(s, 0).patches).toEqual([])
  })

  it('starts a connect still opening over again, by the relay', async () => {
    const s = setup()
    s.controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    s.controller.updateSettings({ transport: 'relay' })
    await vi.advanceTimersByTimeAsync(0)
    expect(s.w.pcs[0]?.closed).toBe(true)
    expect(s.openSession).toHaveBeenCalledTimes(2)
    expect(s.openSession.mock.calls[1]).toEqual([DEVICE])
    expect(s.startSession).toHaveBeenCalledTimes(1)
    expect(s.startSession.mock.calls[0]?.[0].kind).toBe('relay')
  })

  it('reconnects at once while a reconnect waits for its turn', async () => {
    const s = setup()
    await live(s)
    session(s, 0).end({ by: 'link', reason: 'connection closed' })
    expect(s.controller.phase.get()).toMatchObject({ kind: 'reconnecting', attempt: 1 })
    s.controller.updateSettings({ transport: 'relay' })
    await vi.advanceTimersByTimeAsync(0)
    expect(s.openSession).toHaveBeenCalledTimes(2)
    expect(s.openSession.mock.calls[1]).toEqual([DEVICE])
    expect(s.params(1).credentials?.take()).toBe(CREDENTIAL)
    await vi.advanceTimersByTimeAsync(60_000)
    expect(s.openSession).toHaveBeenCalledTimes(2)
  })

  it('only saves the choice without a session to reconnect', async () => {
    const idle = setup()
    idle.controller.updateSettings({ transport: 'relay' })
    await vi.advanceTimersByTimeAsync(60_000)
    expect(idle.openSession).not.toHaveBeenCalled()
    expect(new RemoteController(DEVICE, idle.deps).settings.get().transport).toBe('relay')

    const ended = setup()
    await live(ended)
    session(ended, 0).end({ by: 'peer', reason: 'closed' })
    ended.controller.updateSettings({ transport: 'relay' })
    await vi.advanceTimersByTimeAsync(60_000)
    expect(ended.openSession).toHaveBeenCalledTimes(1)
  })
})
