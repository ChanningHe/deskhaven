import { create, toBinary } from '@bufbuild/protobuf'
import nacl from 'tweetnacl'
import { describe, expect, it, vi } from 'vitest'

import { fixtureBytes, fixtureSession } from '../__fixtures__/fixtures'
import {
  ManualTimers,
  MemoryTransport,
  msg,
  replayToKeyed,
  sealed,
  startFixture,
  type Replay,
} from '../__fixtures__/harness'
import { passwordHash } from '../crypto/password'
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
  startHandshake,
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
      await passwordHash(s.password, s.salt, s.challenge),
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
      expect(await r.nextEvent()).toEqual({ kind: 'closed', reason: error })
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
    const proof = lastSent(r)
    expect(proof.union.case === 'loginRequest' && proof.union.value.password).toEqual(
      await passwordHash(s.password, s.salt, s.challenge),
    )
  })
})

describe('step timeouts (hbb_common CONNECT_TIMEOUT/READ_TIMEOUT, client.rs:1690)', () => {
  async function timesOut(r: { nextEvent: () => Promise<HandshakeEvent> }) {
    expect(await r.nextEvent()).toEqual({
      kind: 'state',
      state: { kind: 'failed', reason: 'timeout' },
    })
    expect(await r.nextEvent()).toEqual({ kind: 'closed', reason: 'timeout' })
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
    expect(r.timers.running).toBeNull()
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
      await passwordHash(s.password, s.salt, s.challenge),
    )
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
