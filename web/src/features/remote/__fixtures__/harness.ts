import { create, fromBinary, toBinary } from '@bufbuild/protobuf'
import { vi } from 'vitest'

import { AsyncQueue } from '../async-queue'
import { decodeBase64 } from '../bytes'
import { FrameCipher, kxVersionFor } from '../crypto/frame-cipher'
import {
  startHandshake,
  type CredentialSlot,
  type Handshake,
  type HandshakeEvent,
  type StartTimer,
} from '../handshake/handshake'
import { MessageSchema, type Message } from '../proto/gen/message_pb.js'
import { RendezvousMessageSchema } from '../proto/gen/rendezvous_pb.js'
import { loginOption } from '../settings/protocol'
import { DEFAULT_SETTINGS } from '../settings/settings'
import { RelayTransport } from '../transport/relay'
import type { KeyedTransport } from '../transport/transport'
import { fixtureBytes, fixtureSession, type FixtureVersion } from './fixtures'

export class FakeSocket {
  binaryType: BinaryType = 'blob'
  onopen: (() => void) | null = null
  onmessage: ((event: { data: unknown }) => void) | null = null
  onclose: ((event: { code: number }) => void) | null = null
  onerror: (() => void) | null = null
  readonly sent: Uint8Array[] = []
  closedWith: number | null = null
  readonly url: string

  constructor(url: string) {
    this.url = url
  }

  send(data: Uint8Array): void {
    this.sent.push(data.slice())
  }

  close(code?: number): void {
    this.closedWith = code ?? null
  }

  open(): void {
    this.onopen?.()
  }

  receive(bytes: Uint8Array): void {
    this.onmessage?.({ data: bytes.slice().buffer })
  }

  receiveData(data: unknown): void {
    this.onmessage?.({ data })
  }

  drop(code: number): void {
    this.onclose?.({ code })
  }

  async waitForSent(count: number): Promise<void> {
    await vi.waitFor(() => {
      if (this.sent.length < count) throw new Error(`${this.sent.length} of ${count} frames sent`)
    })
  }
}

export const PAIRING_UUID = 'pairing-uuid'

export function relayWith(
  socket: FakeSocket,
  licenceKey = 'server-key',
  targetId = '123456789',
): RelayTransport {
  return new RelayTransport({
    url: socket.url,
    licenceKey,
    uuid: PAIRING_UUID,
    targetId,
    createSocket: () => socket as unknown as WebSocket,
  })
}

export class MemoryTransport implements KeyedTransport {
  readonly sent: Message[] = []
  readonly inbox = new AsyncQueue<Message>()
  cipher: FrameCipher | null = null
  closed = false

  get messages(): AsyncIterable<Message> {
    return this.inbox
  }

  send(message: Message): void {
    this.sent.push(message)
  }

  close(): void {
    this.closed = true
    this.inbox.end()
  }

  startEncryption(cipher: FrameCipher): void {
    this.cipher = cipher
  }
}

export function signedPeerPk(version: FixtureVersion): Uint8Array {
  const rr = fromBinary(RendezvousMessageSchema, fixtureBytes(version, 'hbbs-relay-response'))
  if (rr.union.case !== 'relayResponse' || rr.union.value.union.case !== 'pk') {
    throw new Error('fixture is not a RelayResponse carrying pk')
  }
  return rr.union.value.union.value
}

export const SESSION_ID = 0x0102030405060708n

/** Replays the fixture's random draws: our box secret, then the stream key. */
export function fixtureRandom(version: FixtureVersion): (length: number) => Uint8Array {
  const s = fixtureSession(version)
  const draws = [s.controllerBoxSk, s.symmetricKey]
  return (length) => {
    const next = draws.shift()
    if (next?.length !== length) throw new Error(`unexpected random draw of ${length} bytes`)
    return next
  }
}

/** Timers the test fires by hand; the handshake runs at most one at a time. */
export class ManualTimers {
  readonly #running = new Set<{ ms: number; onExpire: () => void }>()

  readonly start: StartTimer = (ms, onExpire) => {
    const timer = { ms, onExpire }
    this.#running.add(timer)
    return () => this.#running.delete(timer)
  }

  get running(): number | null {
    const [timer] = this.#running
    return timer?.ms ?? null
  }

  expire(): void {
    const timers = [...this.#running]
    this.#running.clear()
    for (const timer of timers) timer.onExpire()
  }
}

export interface Started {
  socket: FakeSocket
  timers: ManualTimers
  handshake: Handshake
  nextEvent: () => Promise<HandshakeEvent>
}

export interface Replay extends Started {
  peer: FrameCipher
  peerReceived: () => Message[]
}

export interface ReplayOptions {
  password?: string
  signedPeerPk?: Uint8Array
  credentials?: CredentialSlot
}

export function startFixture(version: FixtureVersion, options: ReplayOptions = {}): Started {
  const s = fixtureSession(version)
  const socket = new FakeSocket('wss://console.test/ws/relay')
  const timers = new ManualTimers()
  const handshake = startHandshake(
    { kind: 'relay', transport: relayWith(socket, s.serverPub, s.peerId) },
    {
      targetId: s.peerId,
      serverPk: decodeBase64(s.serverPub),
      signedPeerPk: options.signedPeerPk ?? signedPeerPk(version),
      myId: 'console-admin',
      myName: 'Admin',
      sessionId: SESSION_ID,
      decoders: { vp9: true, av1: true, vp8: true, h264: true, h265: true },
      loginOption: loginOption(DEFAULT_SETTINGS),
      requireKxV1: false,
      ...(options.credentials ? { credentials: options.credentials } : {}),
    },
    { random: fixtureRandom(version), startTimer: timers.start },
  )
  if (options.password !== undefined) void handshake.login(options.password)
  const events = handshake.events[Symbol.asyncIterator]()
  socket.open()
  return {
    socket,
    timers,
    handshake,
    nextEvent: async () => {
      const r = await events.next()
      if (r.done) throw new Error('handshake events ended')
      return r.value
    },
  }
}

/** Connects to the fixture peer up to (not including) its first encrypted frame. */
export async function replayToKeyed(
  version: FixtureVersion,
  options: ReplayOptions = {},
): Promise<Replay> {
  const s = fixtureSession(version)
  const started = startFixture(version, options)
  started.socket.receive(fixtureBytes(version, 'hbbr-signed-id'))
  await started.socket.waitForSent(2)
  const picked = kxVersionFor(s.peerKxVersion)
  const transcript = {
    initiatorPk: s.controllerBoxPk,
    responderPk: s.peerBoxPk,
    advertised: s.peerKxVersion,
    picked,
  }
  const peer =
    picked === 0
      ? FrameCipher.v0(s.symmetricKey)
      : FrameCipher.v1(s.symmetricKey, transcript, 'responder')
  const KEYED_FROM = 2 // the relay request and our PublicKey go out in plaintext
  const received: Message[] = []
  return {
    ...started,
    peer,
    peerReceived: () => {
      for (const frame of started.socket.sent.slice(KEYED_FROM + received.length)) {
        received.push(fromBinary(MessageSchema, peer.open(frame)))
      }
      return received
    },
  }
}

export function sealed(peer: FrameCipher, message: Message): Uint8Array {
  return peer.seal(toBinary(MessageSchema, message))
}

export function msg(union: Message['union']): Message {
  return create(MessageSchema, { union })
}
