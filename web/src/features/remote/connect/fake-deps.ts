import { vi } from 'vitest'

import type { WebClientCandidates, WebClientSession } from '@/api/generated/model'

import { MemoryTransport } from '../__fixtures__/harness'
import type { Session } from '../session/session'
import { memoryStorage } from '../settings/testing'
import { FakePeerConnection } from '../transport/fake-rtc'
import { WebRtcTransport } from '../transport/webrtc'
import type { ConnectDeps, WebRtcDeps } from './controller'
import { FakeSession, OPENED } from './fake-session'

/** Deps that open OPENED (or `open`'s result) and start FakeSessions; the mocks are exposed. */
export function fakeDeps(open: () => Promise<WebClientSession> = () => Promise.resolve(OPENED)) {
  const sessions: FakeSession[] = []
  const relays: MemoryTransport[] = []
  const openSession = vi.fn<ConnectDeps['openSession']>(open)
  const closeSession = vi.fn<ConnectDeps['closeSession']>(() => Promise.resolve())
  const probeDecoders = vi.fn(() => Promise.resolve({ vp9: true, av1: false, vp8: true }))
  const createRelay = vi.fn<ConnectDeps['createRelay']>(() => {
    const relay = new MemoryTransport()
    relays.push(relay)
    return relay
  })
  const startSession = vi.fn<ConnectDeps['startSession']>((): Session => {
    const session = new FakeSession()
    sessions.push(session)
    return session
  })
  const storage = memoryStorage()
  const deps: ConnectDeps = {
    openSession,
    closeSession,
    probeDecoders,
    createRelay,
    startSession,
    location: { protocol: 'https:', host: 'console.example' },
    storage,
  }
  return {
    deps,
    storage,
    sessions,
    relays,
    openSession,
    closeSession,
    probeDecoders,
    createRelay,
    startSession,
  }
}

/** WebRTC deps over FakePeerConnections; candidate polls hang until the test stops them. */
export function fakeWebRtc() {
  const pcs: FakePeerConnection[] = []
  const polls: AbortSignal[] = []
  const createOfferer = vi.fn<WebRtcDeps['createOfferer']>(() =>
    WebRtcTransport.create(() => {
      const pc = new FakePeerConnection()
      pcs.push(pc)
      return pc
    }),
  )
  const postCandidate = vi.fn<WebRtcDeps['postCandidate']>(() => Promise.resolve())
  const pollCandidates = vi.fn<WebRtcDeps['pollCandidates']>(
    (_, __, signal) =>
      new Promise<WebClientCandidates>((_resolve, reject) => {
        polls.push(signal)
        signal.addEventListener('abort', () => {
          reject(new DOMException('aborted', 'AbortError'))
        })
      }),
  )
  const webrtc: WebRtcDeps = { createOfferer, postCandidate, pollCandidates }
  return { webrtc, pcs, polls, createOfferer, postCandidate, pollCandidates }
}
