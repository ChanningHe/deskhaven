import { vi } from 'vitest'

import type { WebClientSession } from '@/api/generated/model'

import type { Session } from '../session/session'
import type { KeyedTransport } from '../transport/transport'
import type { ConnectDeps } from './controller'
import { FakeSession, OPENED } from './fake-session'

/** Deps that open OPENED (or `open`'s result) and start FakeSessions; the mocks are exposed. */
export function fakeDeps(open: () => Promise<WebClientSession> = () => Promise.resolve(OPENED)) {
  const sessions: FakeSession[] = []
  const openSession = vi.fn(open)
  const closeSession = vi.fn<ConnectDeps['closeSession']>(() => Promise.resolve())
  const probeDecoders = vi.fn(() => Promise.resolve({ vp9: true, av1: false, vp8: true }))
  const createRelay = vi.fn<ConnectDeps['createRelay']>((): KeyedTransport => ({
    send: vi.fn(),
    messages: {
      [Symbol.asyncIterator]: () => ({
        next: () => new Promise<IteratorResult<never>>(() => undefined),
      }),
    },
    close: vi.fn(),
    startEncryption: vi.fn(),
  }))
  const startSession = vi.fn<ConnectDeps['startSession']>((): Session => {
    const session = new FakeSession()
    sessions.push(session)
    return session
  })
  const deps: ConnectDeps = {
    openSession,
    closeSession,
    probeDecoders,
    createRelay,
    startSession,
    location: { protocol: 'https:', host: 'console.example' },
  }
  return { deps, sessions, openSession, closeSession, probeDecoders, createRelay, startSession }
}
