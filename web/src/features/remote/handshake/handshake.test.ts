import { create, toBinary } from '@bufbuild/protobuf'
import nacl from 'tweetnacl'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { fixtureBytes, fixtureSession } from '../__fixtures__/fixtures'
import {
  ManualTimers,
  MemoryTransport,
  msg,
  replayToKeyed,
  SESSION_ID,
  sealed,
  startFixture,
  type Replay,
} from '../__fixtures__/harness'
import { proof, salted } from '../crypto/password'
import {
  HashSchema,
  LoginResponseSchema,
  MiscSchema,
  OptionMessageSchema,
  PeerInfoSchema,
  SignedIdSchema,
  TestDelaySchema,
  type Message,
} from '../proto/gen/message_pb.js'
import { IdPkSchema } from '../proto/gen/rendezvous_pb.js'
import {
  IDENTITY_TIMEOUT_MS,
  READ_TIMEOUT_MS,
  SILENCE_MS,
  SILENCE_REASON,
  startHandshake,
  type Credential,
  type CredentialSlot,
  type Handshake,
  type HandshakeDeps,
  type HandshakeEvent,
  type Link,
} from './handshake'

const s = fixtureSession('1.4.5')
const peerInfo = create(PeerInfoSchema, { platform: 'Windows', version: '1.4.5' })

/** Keyed against the captured 1.4.5 peer, its captured Hash answered, waiting for the verdict. */
async function awaitingVerdict(password?: string): Promise<Replay> {
  const r = await replayToKeyed('1.4.5', password === undefined ? {} : { password })
  r.socket.receive(fixtureBytes('1.4.5', 'hbbr-enc-01-Hash'))
  r.peer.seal(new Uint8Array(2)) // frame 1 was the captured Hash
  await r.nextEvent()
  await r.socket.waitForSent(3)
  return r
}

function fromPeer(r: Replay, union: Message['union']): void {
  r.socket.receive(sealed(r.peer, msg(union)))
}

function loginError(r: Replay, error: string): void {
  fromPeer(r, {
    case: 'loginResponse',
    value: create(LoginResponseSchema, { union: { case: 'error', value: error } }),
  })
}

function loginOk(r: Replay): void {
  fromPeer(r, {
    case: 'loginResponse',
    value: create(LoginResponseSchema, { union: { case: 'peerInfo', value: peerInfo } }),
  })
}

function quiet(): HandshakeDeps {
  return { startTimer: new ManualTimers().start }
}

function lastSent(r: Replay): Message {
  const last = r.peerReceived().at(-1)
  if (!last) throw new Error('nothing sent')
  return last
}

describe('login verdicts (rustdesk 1.5.0 src/client.rs:114-118, handle_login_error at 4500)', () => {
  it('"Wrong Password" (server/connection.rs:3034) asks again; login() resends the proof', async () => {
    const r = await awaitingVerdict('not-it')
    loginError(r, 'Wrong Password')
    expect(await r.nextEvent()).toEqual({ kind: 'state', state: { kind: 'wrong_password' } })
    await r.handshake.login(s.password)
    expect(await r.nextEvent()).toEqual({ kind: 'state', state: { kind: 'connecting' } })
    const retry = lastSent(r)
    expect(retry.union.case === 'loginRequest' && retry.union.value.password).toEqual(
      await proof(await salted(s.password, s.salt), s.challenge),
    )
  })

  it('"Empty Password" (client.rs:4505) asks for a password', async () => {
    const r = await awaitingVerdict('x')
    loginError(r, 'Empty Password')
    expect(await r.nextEvent()).toEqual({ kind: 'state', state: { kind: 'need_password' } })
  })

  it('"No Password Access" (connection.rs:3008, client.rs:4487) waits for the user to accept', async () => {
    const r = await awaitingVerdict()
    loginError(r, 'No Password Access')
    expect(await r.nextEvent()).toEqual({ kind: 'state', state: { kind: 'awaiting_approval' } })
    loginOk(r)
    expect(await r.nextEvent()).toEqual({ kind: 'state', state: { kind: 'active', peerInfo } })
  })

  it('"2FA Required" (connection.rs:1876, client.rs:4513) takes a code; "Wrong 2FA Code" (connection.rs:3081) asks again', async () => {
    const r = await awaitingVerdict(s.password)
    loginError(r, '2FA Required')
    expect(await r.nextEvent()).toEqual({
      kind: 'state',
      state: { kind: 'need_2fa', wrongCode: false },
    })
    r.handshake.submit2fa('123456')
    expect(await r.nextEvent()).toEqual({ kind: 'state', state: { kind: 'connecting' } })
    expect(lastSent(r).union).toMatchObject({ case: 'auth2fa', value: { code: '123456' } })
    loginError(r, 'Wrong 2FA Code')
    expect(await r.nextEvent()).toEqual({
      kind: 'state',
      state: { kind: 'need_2fa', wrongCode: true },
    })
  })

  it.each(['Offline', 'Too many wrong attempts', 'constructor'])(
    'any other error ("%s", e.g. connection.rs:2947) fails and closes',
    async (error) => {
      const r = await awaitingVerdict(s.password)
      loginError(r, error)
      expect(await r.nextEvent()).toEqual({
        kind: 'state',
        state: { kind: 'failed', reason: error },
      })
      expect(await r.nextEvent()).toEqual({ kind: 'closed', cause: { by: 'peer', reason: error } })
      expect(r.socket.closedWith).toBe(1000)
      expect(() => {
        r.handshake.send(msg({ case: 'misc', value: create(MiscSchema) }))
      }).toThrow()
    },
  )

  it('without a password, logs in empty so the user can accept (client.rs:4745), then uses login()', async () => {
    const r = await replayToKeyed('1.4.5')
    r.socket.receive(fixtureBytes('1.4.5', 'hbbr-enc-01-Hash'))
    r.peer.seal(new Uint8Array(2))
    expect(await r.nextEvent()).toEqual({ kind: 'state', state: { kind: 'need_password' } })
    const empty = lastSent(r)
    expect(empty.union.case === 'loginRequest' && empty.union.value.password).toEqual(
      new Uint8Array(),
    )
    await r.handshake.login(s.password)
    const login = lastSent(r)
    expect(login.union.case === 'loginRequest' && login.union.value.password).toEqual(
      await proof(await salted(s.password, s.salt), s.challenge),
    )
  })
})

describe('step timeouts (hbb_common CONNECT_TIMEOUT/READ_TIMEOUT, client.rs:1690)', () => {
  async function timesOut(r: { nextEvent: () => Promise<HandshakeEvent> }) {
    expect(await r.nextEvent()).toEqual({
      kind: 'state',
      state: { kind: 'failed', reason: 'timeout' },
    })
    expect(await r.nextEvent()).toEqual({
      kind: 'closed',
      cause: { by: 'link', reason: 'timeout' },
    })
  }

  it('bounds relay connect, pairing and the SignedId', async () => {
    const started = startFixture('1.4.5')
    expect(started.timers.running).toBe(IDENTITY_TIMEOUT_MS)
    started.timers.expire()
    await timesOut(started)
    expect(started.socket.closedWith).toBe(1000)
    expect(started.socket.sent).toHaveLength(1)
  })

  it('bounds the wait for the Hash once our key is sent', async () => {
    const r = await replayToKeyed('1.4.5', { password: s.password })
    expect(r.timers.running).toBe(READ_TIMEOUT_MS)
    r.timers.expire()
    await timesOut(r)
    expect(r.socket.sent).toHaveLength(2)
  })

  it('bounds the wait for the login verdict', async () => {
    const r = await awaitingVerdict(s.password)
    expect(r.timers.running).toBe(READ_TIMEOUT_MS)
    r.timers.expire()
    await timesOut(r)
  })

  it('runs no timer while the peer waits on a person', async () => {
    const r = await awaitingVerdict()
    expect(r.timers.running).toBeNull()
    loginError(r, 'No Password Access')
    await r.nextEvent()
    expect(r.timers.running).toBeNull()
    loginOk(r)
    await r.nextEvent()
    expect(r.timers.running).toBe(SILENCE_MS)
  })
})

describe('password lifetime', () => {
  it('forgets a rejected password: a later Hash gets an empty login, not the old one', async () => {
    const r = await awaitingVerdict('not-it')
    loginError(r, 'Wrong Password')
    expect(await r.nextEvent()).toEqual({ kind: 'state', state: { kind: 'wrong_password' } })
    fromPeer(r, { case: 'hash', value: create(HashSchema, { salt: 'salt2', challenge: 'chal2' }) })
    expect(await r.nextEvent()).toEqual({ kind: 'state', state: { kind: 'need_password' } })
    const again = lastSent(r)
    expect(again.union.case === 'loginRequest' && again.union.value.password).toEqual(
      new Uint8Array(),
    )
  })

  it('keeps a password given before the Hash for that Hash', async () => {
    const r = await replayToKeyed('1.4.5')
    await r.handshake.login(s.password)
    r.socket.receive(fixtureBytes('1.4.5', 'hbbr-enc-01-Hash'))
    r.peer.seal(new Uint8Array(2))
    expect(await r.nextEvent()).toEqual({ kind: 'state', state: { kind: 'connecting' } })
    await r.socket.waitForSent(3)
    const login = lastSent(r)
    expect(login.union.case === 'loginRequest' && login.union.value.password).toEqual(
      await proof(await salted(s.password, s.salt), s.challenge),
    )
  })
})

function slot(held: Credential | null = null) {
  const kept: Credential[] = []
  let taken = 0
  const credentials: CredentialSlot = {
    take: () => {
      taken++
      const credential = held
      held = null
      return credential
    },
    keep: (credential) => {
      kept.push(credential)
    },
  }
  return { credentials, kept, taken: () => taken }
}

async function saved(salt = s.salt): Promise<Credential> {
  return { salt, salted: await salted(s.password, salt) }
}

function sentPassword(r: Replay): Uint8Array | undefined {
  const last = lastSent(r)
  return last.union.case === 'loginRequest' ? last.union.value.password : undefined
}

describe('logging in again without the password (client.rs:2961-2974)', () => {
  async function hashed(credentials: CredentialSlot, password?: string): Promise<Replay> {
    const r = await replayToKeyed('1.4.5', {
      credentials,
      ...(password === undefined ? {} : { password }),
    })
    r.socket.receive(fixtureBytes('1.4.5', 'hbbr-enc-01-Hash'))
    r.peer.seal(new Uint8Array(2))
    return r
  }

  it('answers the Hash with the kept credential, not with need_password, in the session it is given', async () => {
    const k = slot(await saved())
    const r = await hashed(k.credentials)
    expect(await r.nextEvent()).toEqual({ kind: 'state', state: { kind: 'connecting' } })
    await r.socket.waitForSent(3)
    expect(sentPassword(r)).toEqual(await proof(await salted(s.password, s.salt), s.challenge))
    const login = lastSent(r)
    expect(login.union.case === 'loginRequest' && login.union.value.sessionId).toBe(SESSION_ID)
    loginOk(r)
    expect(await r.nextEvent()).toMatchObject({ state: { kind: 'active' } })
    expect(k.kept).toEqual([await saved()])
    expect(k.taken()).toBe(1)
  })

  it('drops a credential for another salt and asks for the password', async () => {
    const k = slot(await saved('another salt'))
    const r = await hashed(k.credentials)
    expect(await r.nextEvent()).toEqual({ kind: 'state', state: { kind: 'need_password' } })
    expect(sentPassword(r)).toEqual(new Uint8Array())
    loginOk(r)
    await r.nextEvent()
    expect(k.kept).toEqual([])
  })

  it('submits the kept credential once: after Wrong Password a later Hash gets an empty login', async () => {
    const k = slot(await saved())
    const r = await hashed(k.credentials)
    await r.nextEvent()
    await r.socket.waitForSent(3)
    loginError(r, 'Wrong Password')
    expect(await r.nextEvent()).toEqual({ kind: 'state', state: { kind: 'wrong_password' } })
    fromPeer(r, { case: 'hash', value: create(HashSchema, { salt: s.salt, challenge: 'chal2' }) })
    expect(await r.nextEvent()).toEqual({ kind: 'state', state: { kind: 'need_password' } })
    expect(sentPassword(r)).toEqual(new Uint8Array())
    expect(k.kept).toEqual([])
  })

  it('prefers a password typed before the Hash and keeps what it logged in with', async () => {
    const k = slot()
    const r = await hashed(k.credentials, s.password)
    await r.nextEvent()
    await r.socket.waitForSent(3)
    expect(k.taken()).toBe(0)
    loginOk(r)
    await r.nextEvent()
    expect(k.kept).toEqual([await saved()])
  })

  it('keeps nothing from a rejected password, then keeps the accepted one', async () => {
    const k = slot()
    const r = await hashed(k.credentials, 'not-it')
    await r.nextEvent()
    await r.socket.waitForSent(3)
    loginError(r, 'Wrong Password')
    await r.nextEvent()
    expect(k.kept).toEqual([])
    await r.handshake.login(s.password)
    await r.nextEvent()
    loginOk(r)
    await r.nextEvent()
    expect(k.kept).toEqual([await saved()])
  })

  it('keeps the password through 2FA, for a peer that skips 2FA in a recent session (connection.rs:1860)', async () => {
    const k = slot()
    const r = await hashed(k.credentials, s.password)
    await r.nextEvent()
    await r.socket.waitForSent(3)
    loginError(r, '2FA Required')
    await r.nextEvent()
    r.handshake.submit2fa('123456')
    await r.nextEvent()
    loginOk(r)
    await r.nextEvent()
    expect(k.kept).toEqual([await saved()])
  })

  it('keeps nothing when the peer user approved instead of the password', async () => {
    const k = slot()
    const r = await hashed(k.credentials, 'whatever')
    await r.nextEvent()
    await r.socket.waitForSent(3)
    loginError(r, 'No Password Access')
    await r.nextEvent()
    loginOk(r)
    await r.nextEvent()
    expect(k.kept).toEqual([])
  })
})

describe('why the handshake closed', () => {
  async function closedEvent(r: { nextEvent: () => Promise<HandshakeEvent> }) {
    for (;;) {
      const event = await r.nextEvent()
      if (event.kind === 'closed') return event
    }
  }

  it('the link: the relay socket drops', async () => {
    const r = await awaitingVerdict(s.password)
    r.socket.drop(1006)
    expect(await closedEvent(r)).toEqual({
      kind: 'closed',
      cause: { by: 'link', reason: 'relay connection closed (code 1006)' },
    })
  })

  it('local: the peer identity does not verify', async () => {
    const transport = new MemoryTransport()
    const peer = syntheticPeer('', 1)
    const handshake = startHandshake({ kind: 'relay', transport }, peer.params, quiet())
    transport.inbox.push(msg({ case: 'misc', value: create(MiscSchema) }))
    const events = handshake.events[Symbol.asyncIterator]()
    await events.next()
    expect((await events.next()).value).toMatchObject({ kind: 'closed', cause: { by: 'local' } })
  })

  it('whoever asks: close() carries its cause', async () => {
    const r = await awaitingVerdict(s.password)
    r.handshake.close({ by: 'user', reason: '' })
    expect(await closedEvent(r)).toEqual({ kind: 'closed', cause: { by: 'user', reason: '' } })
  })
})

describe('message order', () => {
  it('passes PeerInfo, CursorData and Misc that arrive before the verdict to the caller', async () => {
    const r = await awaitingVerdict(s.password)
    fromPeer(r, { case: 'peerInfo', value: peerInfo })
    fromPeer(r, { case: 'misc', value: create(MiscSchema) })
    loginOk(r)
    expect(await r.nextEvent()).toMatchObject({ message: { union: { case: 'peerInfo' } } })
    expect(await r.nextEvent()).toMatchObject({ message: { union: { case: 'misc' } } })
    expect(await r.nextEvent()).toMatchObject({ state: { kind: 'active' } })
  })

  it('echoes the peer TestDelay in any phase but not one marked from_client', async () => {
    const r = await awaitingVerdict(s.password)
    fromPeer(r, {
      case: 'testDelay',
      value: create(TestDelaySchema, { time: 1n, fromClient: true }),
    })
    loginOk(r)
    fromPeer(r, { case: 'testDelay', value: create(TestDelaySchema, { time: 2n }) })
    await r.nextEvent()
    await r.nextEvent()
    await r.nextEvent()
    expect(r.socket.sent).toHaveLength(4)
    expect(lastSent(r).union).toMatchObject({ case: 'testDelay', value: { time: 2n } })
  })

  it('refuses to send before the stream is keyed', () => {
    const transport = new MemoryTransport()
    const peer = syntheticPeer('', 1)
    const handshake = startHandshake({ kind: 'relay', transport }, peer.params, quiet())
    expect(() => {
      handshake.send(msg({ case: 'misc', value: create(MiscSchema) }))
    }).toThrow()
    expect(transport.sent).toHaveLength(0)
  })
})

/** An identity we can sign for, to exercise fingerprint and version cases the captures lack. */
function syntheticPeer(dtlsFingerprint: string, kxVersion: number) {
  const id = '42'
  const server = nacl.sign.keyPair()
  const device = nacl.sign.keyPair()
  const box = nacl.box.keyPair()
  const idPk = (pk: Uint8Array, extra: object = {}) =>
    toBinary(IdPkSchema, create(IdPkSchema, { id, pk, ...extra }))
  return {
    params: {
      targetId: id,
      serverPk: server.publicKey,
      signedPeerPk: nacl.sign(idPk(device.publicKey), server.secretKey),
      myId: 'admin',
      myName: 'Admin',
      sessionId: 7n,
      decoders: { vp9: false, av1: false, vp8: true },
      loginOption: create(OptionMessageSchema),
    },
    signedId: msg({
      case: 'signedId',
      value: create(SignedIdSchema, {
        id: nacl.sign(idPk(box.publicKey, { dtlsFingerprint, kxVersion }), device.secretKey),
      }),
    }),
  }
}

async function firstEvent(handshake: Handshake): Promise<HandshakeEvent> {
  const next = await handshake.events[Symbol.asyncIterator]().next()
  if (next.done === true) throw new Error('no event')
  return next.value
}

describe('identity binding', () => {
  const FP = 'sha-256 5E:8E:00:11'

  function start(link: Link, transport: MemoryTransport, peer: ReturnType<typeof syntheticPeer>) {
    const handshake = startHandshake(link, peer.params, quiet())
    transport.inbox.push(peer.signedId)
    return handshake
  }

  it('WebRTC: keys with version 0 and no stream cipher when the signed fingerprint matches', async () => {
    const transport = new MemoryTransport()
    const handshake = start(
      { kind: 'webrtc', transport, remoteFingerprint: FP },
      transport,
      syntheticPeer(FP, 1),
    )
    expect(handshake.link).toBe('webrtc')
    await vi.waitFor(() => {
      expect(transport.sent).toHaveLength(1)
    })
    expect(transport.sent[0]?.union).toMatchObject({ case: 'publicKey', value: { kxVersion: 0 } })
    expect(transport.cipher).toBeNull()
  })

  it('relay: keys with version 1 and a stream cipher when the peer advertises 1', async () => {
    const transport = new MemoryTransport()
    const handshake = start({ kind: 'relay', transport }, transport, syntheticPeer('', 1))
    expect(handshake.link).toBe('relay')
    await vi.waitFor(() => {
      expect(transport.sent).toHaveLength(1)
    })
    expect(transport.sent[0]?.union).toMatchObject({ case: 'publicKey', value: { kxVersion: 1 } })
    expect(transport.cipher).not.toBeNull()
  })

  it.each([
    ['another fingerprint', 'sha-256 AA:BB'],
    ['no fingerprint', ''],
  ])('WebRTC: fails closed when the peer signed %s', async (_, signed) => {
    const transport = new MemoryTransport()
    const link: Link = { kind: 'webrtc', transport, remoteFingerprint: FP }
    expect(await firstEvent(start(link, transport, syntheticPeer(signed, 0)))).toMatchObject({
      state: { kind: 'failed' },
    })
    expect(transport.sent).toHaveLength(0)
    expect(transport.closed).toBe(true)
  })

  it('relay: fails closed when the peer signed a DTLS fingerprint', async () => {
    const transport = new MemoryTransport()
    expect(
      await firstEvent(start({ kind: 'relay', transport }, transport, syntheticPeer(FP, 1))),
    ).toMatchObject({ state: { kind: 'failed' } })
    expect(transport.sent).toHaveLength(0)
  })

  it('relay: fails closed when the first message is not a SignedId', async () => {
    const transport = new MemoryTransport()
    const peer = syntheticPeer('', 1)
    const handshake = startHandshake({ kind: 'relay', transport }, peer.params, quiet())
    transport.inbox.push(msg({ case: 'misc', value: create(MiscSchema) }))
    expect(await firstEvent(handshake)).toMatchObject({ state: { kind: 'failed' } })
    expect(transport.sent).toHaveLength(0)
  })

  it('fails when the server-signed identity names another device', async () => {
    const transport = new MemoryTransport()
    const peer = syntheticPeer('', 1)
    const handshake = startHandshake(
      { kind: 'relay', transport },
      { ...peer.params, targetId: '43' },
    )
    expect(await firstEvent(handshake)).toMatchObject({ state: { kind: 'failed' } })
  })

  it('fails when the browser can decode none of the codecs', async () => {
    const transport = new MemoryTransport()
    const peer = syntheticPeer('', 1)
    const handshake = startHandshake(
      { kind: 'relay', transport },
      { ...peer.params, decoders: { vp9: false, av1: false, vp8: false } },
    )
    expect(await firstEvent(handshake)).toEqual({
      kind: 'state',
      state: { kind: 'failed', reason: 'no supported video decoder' },
    })
  })
})

describe('a silent peer (it sends TestDelay every second: connection.rs:504,703)', () => {
  afterEach(() => {
    vi.useRealTimers()
  })

  async function active() {
    vi.useFakeTimers()
    const transport = new MemoryTransport()
    const peer = syntheticPeer('', 1)
    const heard: number[] = []
    const handshake = startHandshake(
      { kind: 'relay', transport },
      {
        ...peer.params,
        onHeard: (at) => {
          heard.push(at)
        },
      },
    )
    void handshake.login('pw')
    const events: HandshakeEvent[] = []
    void (async () => {
      for await (const event of handshake.events) events.push(event)
    })()
    transport.inbox.push(peer.signedId)
    transport.inbox.push(
      msg({ case: 'hash', value: create(HashSchema, { salt: 's', challenge: 'c' }) }),
    )
    await vi.waitFor(() => {
      expect(transport.sent.at(-1)?.union.case).toBe('loginRequest')
    })
    transport.inbox.push(
      msg({
        case: 'loginResponse',
        value: create(LoginResponseSchema, { union: { case: 'peerInfo', value: peerInfo } }),
      }),
    )
    await vi.waitFor(() => {
      expect(events.at(-1)).toMatchObject({ state: { kind: 'active' } })
    })
    const testDelay = async () => {
      transport.inbox.push(msg({ case: 'testDelay', value: create(TestDelaySchema, { time: 1n }) }))
      await vi.advanceTimersByTimeAsync(0)
    }
    return { transport, events, heard, testDelay }
  }

  it('tells when it last heard the peer, in wall-clock time', async () => {
    const a = await active()
    expect(a.heard).toHaveLength(3)
    await vi.advanceTimersByTimeAsync(5000)
    await a.testDelay()
    expect(a.heard.at(-1)).toBe(Date.now())
  })

  it('stays up while the peer keeps talking', async () => {
    const a = await active()
    for (let i = 0; i < 20; i++) {
      await vi.advanceTimersByTimeAsync(SILENCE_MS - 1000)
      await a.testDelay()
    }
    expect(a.events.some((e) => e.kind === 'closed')).toBe(false)
    expect(a.transport.closed).toBe(false)
  })

  it(`closes as a lost link after ${String(SILENCE_MS)} ms without a message`, async () => {
    const a = await active()
    await a.testDelay()
    await vi.advanceTimersByTimeAsync(SILENCE_MS - 1)
    expect(a.events.some((e) => e.kind === 'closed')).toBe(false)
    await vi.advanceTimersByTimeAsync(1)
    expect(a.events.at(-1)).toEqual({
      kind: 'closed',
      cause: { by: 'link', reason: SILENCE_REASON },
    })
    expect(a.transport.closed).toBe(true)
  })
})
