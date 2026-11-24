import { create } from '@bufbuild/protobuf'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { WebClientSession } from '@/api/generated/model'

import { msg, type MemoryTransport } from '../__fixtures__/harness'
import { TestDelaySchema } from '../proto/gen/message_pb.js'
import { PEER_FINGERPRINT, answerEndpoint, type FakePeerConnection } from '../transport/fake-rtc'
import { RemoteController } from './controller'
import { fakeDeps, fakeWebRtc } from './fake-deps'
import { OPENED } from './fake-session'
import { RELAY_FALLBACK_DELAY_MS, WEBRTC_CONNECT_TIMEOUT_MS } from './race'

const DEVICE = '123456789'

const ANSWERED: WebClientSession = {
  ...OPENED,
  answer: answerEndpoint(),
  ice_servers: [
    { urls: ['stun:stun.example:3478'], username: null, credential: null },
    { urls: ['turn:turn.example:3478'], username: 'u', credential: 'c' },
  ],
}

function race(opened: WebClientSession = ANSWERED) {
  const f = fakeDeps(() => Promise.resolve(opened))
  const w = fakeWebRtc()
  f.deps.webrtc = w.webrtc
  const controller = new RemoteController(DEVICE, f.deps)
  controller.connect()
  const pc = (): FakePeerConnection => {
    const first = w.pcs[0]
    if (!first) throw new Error('no peer connection')
    return first
  }
  return { ...f, w, controller, pc }
}

/** The peer's first frame on the relay: it has paired. */
function pair(relay: MemoryTransport | undefined) {
  relay?.inbox.push(msg({ case: 'testDelay', value: create(TestDelaySchema, { time: 1n }) }))
}

beforeEach(() => {
  vi.useFakeTimers()
})

afterEach(() => {
  vi.useRealTimers()
})

describe('RemoteController: WebRTC first, the relay as fallback', () => {
  it('offers WebRTC with the session and applies the answer with its ICE servers', async () => {
    const r = race()
    await vi.advanceTimersByTimeAsync(0)
    expect(r.openSession).toHaveBeenCalledWith(DEVICE, expect.stringMatching(/^webrtc:\/\//))
    expect(r.pc().configuration).toEqual({
      iceServers: [
        { urls: ['stun:stun.example:3478'] },
        { urls: ['turn:turn.example:3478'], username: 'u', credential: 'c' },
      ],
    })
    expect(r.createRelay).toHaveBeenCalledTimes(1)
    expect(r.startSession).not.toHaveBeenCalled()
    expect(r.controller.phase.get()).toEqual({ kind: 'opening' })
    expect(r.controller.transport.get()).toBe('connecting')
  })

  it('takes the data channel the moment it opens and closes the relay', async () => {
    const r = race()
    await vi.advanceTimersByTimeAsync(0)
    r.pc().channel?.open()
    await vi.advanceTimersByTimeAsync(0)
    expect(r.startSession).toHaveBeenCalledTimes(1)
    const [link, params] = r.startSession.mock.calls[0] ?? []
    expect(link?.kind === 'webrtc' && link.remoteFingerprint).toBe(PEER_FINGERPRINT)
    expect(link?.transport).not.toBe(r.relays[0])
    expect(params?.targetId).toBe(DEVICE)
    expect(r.relays[0]?.closed).toBe(true)
    expect(r.pc().closed).toBe(false)
    expect(r.controller.transport.get()).toBe('webrtc')
    expect(r.controller.phase.get()).toEqual({ kind: 'session', session: r.sessions[0] })
    expect(r.w.polls.length).toBeGreaterThan(0)
    expect(r.w.polls.every((signal) => signal.aborted)).toBe(true)
  })

  it('holds a paired relay and still takes WebRTC opening within the grace period', async () => {
    const r = race()
    await vi.advanceTimersByTimeAsync(0)
    pair(r.relays[0])
    await vi.advanceTimersByTimeAsync(RELAY_FALLBACK_DELAY_MS - 1)
    expect(r.startSession).not.toHaveBeenCalled()
    r.pc().channel?.open()
    await vi.advanceTimersByTimeAsync(0)
    expect(r.startSession).toHaveBeenCalledTimes(1)
    expect(r.startSession.mock.calls[0]?.[0].kind).toBe('webrtc')
    expect(r.relays[0]?.closed).toBe(true)
  })

  it('takes the paired relay when the grace period runs out, its first frame intact', async () => {
    const r = race()
    await vi.advanceTimersByTimeAsync(0)
    pair(r.relays[0])
    await vi.advanceTimersByTimeAsync(RELAY_FALLBACK_DELAY_MS)
    expect(r.startSession).toHaveBeenCalledTimes(1)
    const link = r.startSession.mock.calls[0]?.[0]
    expect(link?.kind).toBe('relay')
    const first = await link?.transport.messages[Symbol.asyncIterator]().next()
    expect(first?.done === false && first.value.union.case).toBe('testDelay')
    expect(r.pc().closed).toBe(true)
    expect(r.relays[0]?.closed).toBe(false)
    expect(r.controller.transport.get()).toBe('relay')
  })

  it('goes straight to the relay when the device gave no answer', async () => {
    const r = race(OPENED)
    await vi.advanceTimersByTimeAsync(0)
    expect(r.startSession).toHaveBeenCalledWith(
      { kind: 'relay', transport: r.relays[0] },
      expect.anything(),
      expect.anything(),
      expect.anything(),
    )
    expect(r.pc().closed).toBe(true)
    expect(r.controller.transport.get()).toBe('relay')
  })

  it('offers nothing where the browser has no WebRTC', async () => {
    const f = fakeDeps()
    const w = fakeWebRtc()
    w.createOfferer.mockRejectedValue(new ReferenceError('RTCPeerConnection is not defined'))
    f.deps.webrtc = w.webrtc
    new RemoteController(DEVICE, f.deps).connect()
    await vi.advanceTimersByTimeAsync(0)
    expect(f.openSession).toHaveBeenCalledWith(DEVICE)
    expect(f.openSession.mock.calls[0]).toHaveLength(1)
    expect(f.startSession.mock.calls[0]?.[0].kind).toBe('relay')
  })

  it('takes the relay as soon as it pairs once WebRTC has failed', async () => {
    const r = race()
    await vi.advanceTimersByTimeAsync(0)
    r.pc().setState('failed')
    await vi.advanceTimersByTimeAsync(0)
    expect(r.startSession).not.toHaveBeenCalled()
    pair(r.relays[0])
    await vi.advanceTimersByTimeAsync(0)
    expect(r.startSession.mock.calls[0]?.[0].kind).toBe('relay')
  })

  it('falls back to the relay when the data channel never opens', async () => {
    const r = race()
    await vi.advanceTimersByTimeAsync(WEBRTC_CONNECT_TIMEOUT_MS)
    expect(r.startSession).not.toHaveBeenCalled()
    pair(r.relays[0])
    await vi.advanceTimersByTimeAsync(0)
    expect(r.startSession.mock.calls[0]?.[0].kind).toBe('relay')
    expect(r.pc().closed).toBe(true)
  })

  it('fails, closing everything, only when both links fail', async () => {
    const r = race()
    await vi.advanceTimersByTimeAsync(0)
    r.pc().setState('failed')
    r.relays[0]?.close()
    await vi.advanceTimersByTimeAsync(0)
    const phase = r.controller.phase.get()
    expect(
      phase.kind === 'failed' && phase.failure.kind === 'error' && phase.failure.reason,
    ).toMatch(/^WebRTC failed: .*relay failed/)
    expect(r.startSession).not.toHaveBeenCalled()
    expect(r.closeSession).toHaveBeenCalledWith(OPENED.session_id)
    expect(r.pc().closed).toBe(true)
  })

  it('closes both links and the server session on disconnect mid-race', async () => {
    const r = race()
    await vi.advanceTimersByTimeAsync(0)
    r.controller.disconnect()
    await vi.advanceTimersByTimeAsync(RELAY_FALLBACK_DELAY_MS)
    expect(r.pc().closed).toBe(true)
    expect(r.relays[0]?.closed).toBe(true)
    expect(r.closeSession).toHaveBeenCalledTimes(1)
    expect(r.startSession).not.toHaveBeenCalled()
    expect(r.controller.phase.get()).toEqual({ kind: 'idle' })
  })

  it('trickles candidates once the answer is applied', async () => {
    const r = race()
    await vi.advanceTimersByTimeAsync(0)
    r.pc().gather('candidate:1 1 udp 2113937151 a.local 57595 typ host')
    await vi.advanceTimersByTimeAsync(0)
    expect(r.w.postCandidate).toHaveBeenCalledWith(
      OPENED.session_id,
      expect.stringContaining('a.local 57595'),
    )
    expect(r.w.pollCandidates).toHaveBeenCalledWith(OPENED.session_id, 0, expect.anything())
  })
  it('reconnects by the relay alone after a handshake over WebRTC failed, until one succeeds', async () => {
    const r = race()
    await vi.advanceTimersByTimeAsync(0)
    r.pc().channel?.open()
    await vi.advanceTimersByTimeAsync(0)
    r.sessions[0]?.end({
      by: 'local',
      reason: 'peer DTLS fingerprint is not the one of this channel',
    })

    r.controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    expect(r.openSession.mock.calls[1]).toEqual([DEVICE])
    expect(r.w.createOfferer).toHaveBeenCalledTimes(1)
    expect(r.startSession.mock.calls[1]?.[0].kind).toBe('relay')
    expect(r.controller.transport.get()).toBe('relay')

    r.sessions[1]?.end({ by: 'link', reason: 'relay connection closed' })
    r.controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    expect(r.openSession.mock.calls[2]).toEqual([DEVICE])

    r.sessions[2]?.state.set({ kind: 'need_password' })
    r.controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    expect(r.openSession.mock.calls[3]?.[1]).toMatch(/^webrtc:\/\//)
  })

  it('keeps offering WebRTC after a session that got past the handshake, or was hung up', async () => {
    const r = race()
    await vi.advanceTimersByTimeAsync(0)
    r.pc().channel?.open()
    await vi.advanceTimersByTimeAsync(0)
    r.sessions[0]?.state.set({ kind: 'need_password' })
    r.sessions[0]?.end({ by: 'link', reason: 'connection closed' })
    r.controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    expect(r.openSession.mock.calls[1]?.[1]).toMatch(/^webrtc:\/\//)

    r.w.pcs[1]?.channel?.open()
    await vi.advanceTimersByTimeAsync(0)
    expect(r.startSession.mock.calls[1]?.[0].kind).toBe('webrtc')
    r.controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    expect(r.openSession.mock.calls[2]?.[1]).toMatch(/^webrtc:\/\//)
  })
})
