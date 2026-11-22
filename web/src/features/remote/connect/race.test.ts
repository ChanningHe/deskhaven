import { create } from '@bufbuild/protobuf'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { MemoryTransport, msg } from '../__fixtures__/harness'
import { TestDelaySchema } from '../proto/gen/message_pb.js'

import { RELAY_FALLBACK_DELAY_MS, paired, preferWebRtc, withTimeout } from './race'

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: Error) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

beforeEach(() => {
  vi.useFakeTimers()
})

afterEach(() => {
  vi.useRealTimers()
})

describe('preferWebRtc', () => {
  function race() {
    const webrtc = deferred<string>()
    const relay = deferred<string>()
    let result: unknown = null
    let error: Error | null = null
    preferWebRtc(webrtc.promise, relay.promise).then(
      (r) => {
        result = r
      },
      (e: unknown) => {
        error = e as Error
      },
    )
    return { webrtc, relay, result: () => result, error: () => error }
  }

  it('takes WebRTC the moment it is ready', async () => {
    const r = race()
    r.webrtc.resolve('dc')
    await vi.advanceTimersByTimeAsync(0)
    expect(r.result()).toEqual({ winner: 'webrtc', value: 'dc' })
  })

  it('holds a paired relay for the grace period and takes WebRTC arriving within it', async () => {
    const r = race()
    r.relay.resolve('ws')
    await vi.advanceTimersByTimeAsync(RELAY_FALLBACK_DELAY_MS - 1)
    expect(r.result()).toBeNull()
    r.webrtc.resolve('dc')
    await vi.advanceTimersByTimeAsync(0)
    expect(r.result()).toEqual({ winner: 'webrtc', value: 'dc' })
  })

  it('takes the relay when the grace period runs out', async () => {
    const r = race()
    r.relay.resolve('ws')
    await vi.advanceTimersByTimeAsync(RELAY_FALLBACK_DELAY_MS)
    expect(r.result()).toEqual({ winner: 'relay', value: 'ws' })
    r.webrtc.resolve('dc')
    await vi.advanceTimersByTimeAsync(0)
    expect(r.result()).toEqual({ winner: 'relay', value: 'ws' })
  })

  it('takes a held relay at once when WebRTC fails during the grace period', async () => {
    const r = race()
    r.relay.resolve('ws')
    await vi.advanceTimersByTimeAsync(10)
    r.webrtc.reject(new Error('ice failed'))
    await vi.advanceTimersByTimeAsync(0)
    expect(r.result()).toEqual({ winner: 'relay', value: 'ws' })
  })

  it('takes the relay without waiting when WebRTC already failed', async () => {
    const r = race()
    r.webrtc.reject(new Error('ice failed'))
    await vi.advanceTimersByTimeAsync(0)
    expect(r.result()).toBeNull()
    r.relay.resolve('ws')
    await vi.advanceTimersByTimeAsync(0)
    expect(r.result()).toEqual({ winner: 'relay', value: 'ws' })
  })

  it('waits for WebRTC alone when the relay fails', async () => {
    const r = race()
    r.relay.reject(new Error('relay closed'))
    await vi.advanceTimersByTimeAsync(60_000)
    expect(r.result()).toBeNull()
    r.webrtc.resolve('dc')
    await vi.advanceTimersByTimeAsync(0)
    expect(r.result()).toEqual({ winner: 'webrtc', value: 'dc' })
  })

  it('fails only when both fail, naming both', async () => {
    const r = race()
    r.relay.reject(new Error('relay closed'))
    r.webrtc.reject(new Error('ice failed'))
    await vi.advanceTimersByTimeAsync(0)
    expect(r.error()?.message).toBe('WebRTC failed: ice failed; relay failed: relay closed')
  })
})

describe('withTimeout', () => {
  it('rejects with the reason once the time is up', async () => {
    const p = withTimeout(new Promise(() => undefined), 100, 'too slow')
    const check = expect(p).rejects.toThrow('too slow')
    await vi.advanceTimersByTimeAsync(100)
    await check
  })

  it('passes a settled value through', async () => {
    await expect(withTimeout(Promise.resolve(3), 100, 'too slow')).resolves.toBe(3)
  })
})

describe('paired', () => {
  it('resolves on the first frame and replays it before the rest', async () => {
    const relay = new MemoryTransport()
    const first = msg({ case: 'testDelay', value: create(TestDelaySchema, { time: 1n }) })
    const second = msg({ case: 'testDelay', value: create(TestDelaySchema, { time: 2n }) })
    const ready = paired(relay)
    relay.inbox.push(first)
    const link = await ready
    relay.inbox.push(second)
    const seen = []
    for await (const m of link.messages) {
      seen.push(m)
      if (seen.length === 2) break
    }
    expect(seen).toEqual([first, second])
  })

  it('rejects when the relay ends before pairing', async () => {
    const relay = new MemoryTransport()
    const ready = paired(relay)
    relay.close()
    await expect(ready).rejects.toThrow(/closed before pairing/)
  })
})
