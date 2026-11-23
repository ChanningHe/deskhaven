import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { ApiError } from '@/api/api-error'

import type { CloseCause, Credential, CredentialSlot } from '../handshake/handshake'
import type { SessionState } from '../session/session'
import { RemoteController } from './controller'
import { fakeDeps } from './fake-deps'
import { OPENED } from './fake-session'
import { RECENT_MS, RETRY_DELAYS_MS, STEADY_MS, retryDelay } from './reconnect'

const DEVICE = '123456789'
const SALTED_HEX = '5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8'
const CREDENTIAL: Credential = {
  salt: 'salt',
  salted: Uint8Array.from(SALTED_HEX.match(/../g) ?? [], (h) => parseInt(h, 16)),
}
const LINK: CloseCause = { by: 'link', reason: 'relay connection closed (code 1006)' }

beforeEach(() => {
  vi.useFakeTimers()
})

afterEach(() => {
  vi.useRealTimers()
})

type Fake = ReturnType<typeof fakeDeps>

function credentials(f: Fake, index: number): CredentialSlot {
  const slot = f.params(index).credentials
  if (!slot) throw new Error(`session ${String(index)} got no credential slot`)
  return slot
}

function session(f: Fake, index: number) {
  const found = f.sessions[index]
  if (!found) throw new Error(`no session ${String(index)}`)
  return found
}

function typedIn(f: Fake, index: number): void {
  heard(f, index)
  credentials(f, index).keep(CREDENTIAL)
  session(f, index).activate()
}

function cameBack(f: Fake, index: number): void {
  heard(f, index)
  const slot = credentials(f, index)
  const kept = slot.take()
  if (!kept) throw new Error('no credential kept')
  slot.keep(kept)
  session(f, index).activate()
}

function heard(f: Fake, index: number): void {
  f.params(index).onHeard?.(Date.now())
}

async function live(f = fakeDeps()) {
  const controller = new RemoteController(DEVICE, f.deps)
  controller.connect()
  await vi.advanceTimersByTimeAsync(0)
  typedIn(f, 0)
  return { f, controller }
}

describe('retryDelay', () => {
  it('is the base wait, give or take 20 %', () => {
    expect(retryDelay(1, () => 0.5)).toBe(1000)
    expect(retryDelay(1, () => 0)).toBe(800)
    expect(retryDelay(5, () => 0.999_999)).toBe(14_400)
    expect(retryDelay(6, () => 0.5)).toBeNull()
  })
})

describe('RemoteController: reconnecting after the link drops', () => {
  it('backs off 1, 2, 4, 8 and 12 s (27 s, inside the peer 30 s window), then gives up', async () => {
    const { f, controller } = await live()
    const started = Date.now()
    const opened: number[] = []
    f.openSession.mockImplementation(() => {
      opened.push(Date.now() - started)
      return Promise.resolve(OPENED)
    })
    session(f, 0).end(LINK)
    expect(controller.phase.get()).toEqual({ kind: 'reconnecting', attempt: 1, at: started + 1000 })

    for (let attempt = 1; attempt <= RETRY_DELAYS_MS.length; attempt++) {
      await vi.advanceTimersByTimeAsync((RETRY_DELAYS_MS[attempt - 1] ?? 0) - 1)
      expect(opened).toHaveLength(attempt - 1)
      await vi.advanceTimersByTimeAsync(1)
      expect(opened).toHaveLength(attempt)
      expect(controller.phase.get()).toMatchObject({ kind: 'reconnecting', attempt })
      session(f, attempt).end(LINK)
    }

    expect(opened).toEqual([1000, 3000, 7000, 15000, 27000])
    expect(RETRY_DELAYS_MS.reduce((a, b) => a + b)).toBe(27_000)
    expect(controller.phase.get()).toEqual({ kind: 'session', session: f.sessions[5] })
    expect(controller.hasCredential()).toBe(false)
    await vi.advanceTimersByTimeAsync(60_000)
    expect(f.openSession).toHaveBeenCalledTimes(6)
  })

  it.each<[CloseCause['by'], boolean]>([
    ['link', true],
    ['session_switch', true],
    ['peer', false],
    ['local', false],
    ['user', false],
  ])('a session closed by %s reconnects: %s', async (by, again) => {
    const { f, controller } = await live()
    session(f, 0).end({ by, reason: 'closed' })
    await vi.advanceTimersByTimeAsync(60_000)
    expect(f.openSession).toHaveBeenCalledTimes(again ? 2 : 1)
    if (!again) {
      expect(controller.phase.get()).toEqual({ kind: 'session', session: f.sessions[0] })
      expect(controller.hasCredential()).toBe(false)
    }
  })

  it('reconnects after a Windows session switch, which ends before the session is active', async () => {
    const f = fakeDeps()
    const controller = new RemoteController(DEVICE, f.deps)
    controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    credentials(f, 0).keep(CREDENTIAL)
    session(f, 0).state.set({
      kind: 'choose_session',
      sessions: [{ sid: 1, name: 'Console' }],
      current: 2,
    })
    session(f, 0).end({ by: 'session_switch', reason: 'connection closed' })
    await vi.advanceTimersByTimeAsync(1000)
    expect(f.openSession).toHaveBeenCalledTimes(2)
  })

  it('does not retry a first connect that never logged in', async () => {
    const f = fakeDeps()
    const controller = new RemoteController(DEVICE, f.deps)
    controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    session(f, 0).end({ by: 'link', reason: 'timeout' })
    await vi.advanceTimersByTimeAsync(60_000)
    expect(f.openSession).toHaveBeenCalledTimes(1)
    expect(controller.phase.get()).toEqual({ kind: 'session', session: f.sessions[0] })
  })

  it('waits while the page is hidden and starts once it is shown', async () => {
    const f = fakeDeps()
    await live(f)
    f.page.hidden = true
    session(f, 0).end(LINK)
    await vi.advanceTimersByTimeAsync(60_000)
    expect(f.openSession).toHaveBeenCalledTimes(1)
    f.page.show()
    await vi.advanceTimersByTimeAsync(0)
    expect(f.openSession).toHaveBeenCalledTimes(2)
  })

  it('keeps counting attempts when the link drops again within 30 s of coming back', async () => {
    const { f, controller } = await live()
    session(f, 0).end(LINK)
    await vi.advanceTimersByTimeAsync(1000)
    cameBack(f, 1)
    expect(controller.phase.get()).toEqual({ kind: 'session', session: f.sessions[1] })
    await vi.advanceTimersByTimeAsync(STEADY_MS - 1)
    session(f, 1).end(LINK)
    expect(controller.phase.get()).toMatchObject({ kind: 'reconnecting', attempt: 2 })
  })

  it('counts from 1 again once the session stayed active for 30 s', async () => {
    const { f, controller } = await live()
    session(f, 0).end(LINK)
    await vi.advanceTimersByTimeAsync(1000)
    cameBack(f, 1)
    await vi.advanceTimersByTimeAsync(STEADY_MS)
    session(f, 1).end(LINK)
    expect(controller.phase.get()).toMatchObject({ kind: 'reconnecting', attempt: 1 })
  })

  it('logs in again with the kept credential, once, in the same peer session', async () => {
    const { f, controller } = await live()
    expect(controller.hasCredential()).toBe(true)
    session(f, 0).end(LINK)
    await vi.advanceTimersByTimeAsync(1000)
    expect(f.params(1).sessionId).toBe(f.params(0).sessionId)
    const slot = credentials(f, 1)
    expect(slot.take()).toBe(CREDENTIAL)
    expect(controller.hasCredential()).toBe(false)
    expect(slot.take()).toBeNull()
  })

  it('hands the credential out only within 30 s of losing the link', async () => {
    const f = fakeDeps()
    const { controller } = await live(f)
    f.page.hidden = true
    session(f, 0).end(LINK)
    await vi.advanceTimersByTimeAsync(RECENT_MS)
    f.page.show()
    await vi.advanceTimersByTimeAsync(0)
    expect(credentials(f, 1).take()).toBeNull()
    expect(controller.hasCredential()).toBe(false)
  })

  it('measures the 30 s from the last message heard, not from noticing the drop', async () => {
    const { f, controller } = await live()
    await vi.advanceTimersByTimeAsync(RECENT_MS - 5000)
    session(f, 0).end(LINK)
    await vi.advanceTimersByTimeAsync(1000)
    expect(credentials(f, 1).take()).toBeNull()
    expect(controller.hasCredential()).toBe(false)
  })

  it('hands the credential out when the peer was heard just before the drop', async () => {
    const { f } = await live()
    await vi.advanceTimersByTimeAsync(20_000)
    heard(f, 0)
    session(f, 0).end(LINK)
    await vi.advanceTimersByTimeAsync(1000)
    expect(credentials(f, 1).take()).toBe(CREDENTIAL)
  })

  it('takes a new peer session id for a connect the user asks for, and keeps it for retries', async () => {
    const { f, controller } = await live()
    session(f, 0).end(LINK)
    await vi.advanceTimersByTimeAsync(1000)
    expect(f.params(1).sessionId).toBe(f.params(0).sessionId)
    controller.disconnect()
    controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    expect(f.params(2).sessionId).not.toBe(f.params(0).sessionId)
  })

  it('spreads each wait by up to 20 % either way', async () => {
    const f = fakeDeps()
    f.deps.random = () => 0
    await live(f)
    session(f, 0).end(LINK)
    await vi.advanceTimersByTimeAsync(799)
    expect(f.openSession).toHaveBeenCalledTimes(1)
    await vi.advanceTimersByTimeAsync(1)
    expect(f.openSession).toHaveBeenCalledTimes(2)
  })

  it('gives every controller its own peer session id', async () => {
    const a = fakeDeps()
    const b = fakeDeps()
    new RemoteController(DEVICE, a.deps).connect()
    new RemoteController(DEVICE, b.deps).connect()
    await vi.advanceTimersByTimeAsync(0)
    expect(a.params(0).sessionId).not.toBe(b.params(0).sessionId)
  })

  it.each<[string, SessionState]>([
    ['another salt (need_password)', { kind: 'need_password' }],
    ['Wrong Password', { kind: 'wrong_password' }],
    ['2FA Required', { kind: 'need_2fa', wrongCode: false }],
    ['No Password Access', { kind: 'awaiting_approval' }],
  ])('after %s stops reconnecting and asks the user, without the credential', async (_, state) => {
    const { f, controller } = await live()
    session(f, 0).end(LINK)
    await vi.advanceTimersByTimeAsync(1000)
    credentials(f, 1).take()
    session(f, 1).state.set(state)
    expect(controller.phase.get()).toEqual({ kind: 'session', session: f.sessions[1] })
    expect(controller.hasCredential()).toBe(false)
    expect(credentials(f, 1).take()).toBeNull()
    session(f, 1).end(LINK)
    await vi.advanceTimersByTimeAsync(60_000)
    expect(f.openSession).toHaveBeenCalledTimes(2)
  })

  it('stops after a login error that ends the session', async () => {
    const { f, controller } = await live()
    session(f, 0).end(LINK)
    await vi.advanceTimersByTimeAsync(1000)
    credentials(f, 1).take()
    session(f, 1).end({ by: 'peer', reason: 'Too many wrong attempts' })
    await vi.advanceTimersByTimeAsync(60_000)
    expect(f.openSession).toHaveBeenCalledTimes(2)
    expect(controller.phase.get()).toEqual({ kind: 'session', session: f.sessions[1] })
    expect(controller.hasCredential()).toBe(false)
  })

  it.each(['rate_limited', 'too_many_sessions'] as const)(
    'stops when POST sessions answers %s',
    async (code) => {
      const { f, controller } = await live()
      const error = new ApiError(429, { code })
      f.openSession.mockRejectedValue(error)
      session(f, 0).end(LINK)
      await vi.advanceTimersByTimeAsync(1000)
      expect(controller.phase.get()).toEqual({ kind: 'failed', failure: { kind: 'api', error } })
      expect(controller.hasCredential()).toBe(false)
      await vi.advanceTimersByTimeAsync(60_000)
      expect(f.openSession).toHaveBeenCalledTimes(2)
    },
  )

  it('tries again when POST sessions fails otherwise, and keeps the credential for it', async () => {
    const { f, controller } = await live()
    f.openSession.mockRejectedValueOnce(new ApiError(409, { code: 'device_offline' }))
    session(f, 0).end(LINK)
    await vi.advanceTimersByTimeAsync(1000)
    expect(controller.phase.get()).toMatchObject({ kind: 'reconnecting', attempt: 2 })
    expect(controller.hasCredential()).toBe(true)
    await vi.advanceTimersByTimeAsync(2000)
    expect(f.openSession).toHaveBeenCalledTimes(3)
    expect(credentials(f, 1).take()).toBe(CREDENTIAL)
  })

  it('drops the credential and the pending attempt when the user disconnects or cancels', async () => {
    const { f, controller } = await live()
    session(f, 0).end(LINK)
    expect(controller.hasCredential()).toBe(true)
    controller.disconnect()
    expect(controller.phase.get()).toEqual({ kind: 'idle' })
    expect(controller.hasCredential()).toBe(false)
    await vi.advanceTimersByTimeAsync(60_000)
    expect(f.openSession).toHaveBeenCalledTimes(1)
  })

  it('drops the credential when the user ends an active session', async () => {
    const { f, controller } = await live()
    controller.disconnect()
    expect(session(f, 0).closed).toBe(1)
    expect(controller.hasCredential()).toBe(false)
    await vi.advanceTimersByTimeAsync(60_000)
    expect(f.openSession).toHaveBeenCalledTimes(1)
  })

  it('ignores a credential kept by a connect the user already left', async () => {
    const f = fakeDeps()
    const controller = new RemoteController(DEVICE, f.deps)
    controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    controller.disconnect()
    credentials(f, 0).keep(CREDENTIAL)
    expect(controller.hasCredential()).toBe(false)
  })

  it('never puts the credential in a store', async () => {
    const { f, controller } = await live()
    const seen = [snapshot(controller)]
    session(f, 0).end(LINK)
    seen.push(snapshot(controller))
    await vi.advanceTimersByTimeAsync(1000)
    seen.push(snapshot(controller))
    expect(controller.hasCredential()).toBe(true)
    for (const text of seen) {
      expect(text).not.toContain(SALTED_HEX)
      expect(text).not.toContain(String(CREDENTIAL.salted[0]) + ',' + String(CREDENTIAL.salted[1]))
    }
  })
})

function snapshot(controller: RemoteController): string {
  const seen = new WeakSet()
  const walk = (value: unknown): unknown => {
    if (typeof value === 'bigint') return value.toString()
    if (value instanceof Uint8Array) {
      return [Array.from(value, (b) => b.toString(16).padStart(2, '0')).join(''), [...value]]
    }
    if (typeof value !== 'object' || value === null) return value
    if (seen.has(value)) return null
    seen.add(value)
    if ('get' in value && 'subscribe' in value && typeof value.get === 'function') {
      return walk((value.get as () => unknown)())
    }
    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, walk(v)]))
  }
  return JSON.stringify(
    walk({
      phase: controller.phase,
      transport: controller.transport,
      settings: controller.settings,
    }),
  )
}
