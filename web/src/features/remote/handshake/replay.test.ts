import { create, fromBinary } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { fixtureBytes, fixtureSession } from '../__fixtures__/fixtures'
import type { FakeSocket } from '../__fixtures__/harness'
import {
  PAIRING_UUID,
  SESSION_ID,
  msg,
  replayToKeyed,
  sealed,
  signedPeerPk,
  startFixture,
} from '../__fixtures__/harness'
import { passwordHash } from '../crypto/password'
import {
  CursorDataSchema,
  DisplayInfoSchema,
  HashSchema,
  LoginResponseSchema,
  MessageSchema,
  OptionMessage_BoolOption,
  PeerInfoSchema,
  SupportedDecoding_PreferCodec,
  TestDelaySchema,
  type Message,
} from '../proto/gen/message_pb.js'
import { RendezvousMessageSchema } from '../proto/gen/rendezvous_pb.js'
import { CLIENT_VERSION } from './login'

const peerInfo = create(PeerInfoSchema, {
  platform: 'Linux',
  version: '1.4.5',
  displays: [create(DisplayInfoSchema, { width: 1280, height: 800, online: true })],
})

function capturedPublicKey(version: '1.5.0' | '1.4.5'): Message {
  return fromBinary(MessageSchema, fixtureBytes(version, 'hbbr-public-key'))
}

function sentAt(socket: FakeSocket, index: number): Uint8Array {
  const frame = socket.sent[index]
  if (!frame) throw new Error(`no frame ${index} sent`)
  return frame
}

describe('relay replay against the captured 1.4.5 peer (key exchange v0)', () => {
  const s = fixtureSession('1.4.5')

  it('asks the relay to pair, then answers the SignedId with the sealed key', async () => {
    const r = await replayToKeyed('1.4.5', { password: s.password })
    const request = fromBinary(RendezvousMessageSchema, sentAt(r.socket, 0))
    expect(request.union.case).toBe('requestRelay')
    expect(request.union.value).toMatchObject({
      licenceKey: s.serverPub,
      uuid: PAIRING_UUID,
      id: s.peerId,
    })
    const ours = fromBinary(MessageSchema, sentAt(r.socket, 1))
    const captured = capturedPublicKey('1.4.5')
    if (ours.union.case !== 'publicKey' || captured.union.case !== 'publicKey') {
      throw new Error('not a PublicKey')
    }
    expect(ours.union.value.asymmetricValue).toEqual(s.controllerBoxPk)
    expect(ours.union.value.symmetricValue).toEqual(captured.union.value.symmetricValue)
    expect(ours.union.value.kxVersion).toBe(0)
  })

  it('decrypts the captured Hash, logs in, and decrypts the captured VP9 frame 6', async () => {
    const r = await replayToKeyed('1.4.5', { password: s.password })
    r.socket.receive(fixtureBytes('1.4.5', 'hbbr-enc-01-Hash'))
    r.peer.seal(new Uint8Array(2)) // frame 1 was the captured Hash; keep the peer's counter in step
    expect(await r.nextEvent()).toEqual({ kind: 'state', state: { kind: 'connecting' } })
    await r.socket.waitForSent(3)

    const login = r.peerReceived()[0]
    if (login?.union.case !== 'loginRequest') throw new Error('not a LoginRequest')
    expect(login.union.value).toMatchObject({
      username: s.peerId,
      password: await passwordHash(s.password, s.salt, s.challenge),
      myId: 'console-admin',
      myName: 'Admin',
      myPlatform: 'Web',
      version: CLIENT_VERSION,
      sessionId: SESSION_ID,
      videoAckRequired: false,
      option: {
        disableAudio: OptionMessage_BoolOption.Yes,
        supportedDecoding: {
          abilityVp9: 1,
          abilityAv1: 1,
          abilityVp8: 1,
          prefer: SupportedDecoding_PreferCodec.VP9,
        },
      },
    })

    // Frames 2-5 were not kept (they name the capture host); 1.4.5 sent TestDelay first.
    const delay = msg({ case: 'testDelay', value: create(TestDelaySchema, { time: 42n }) })
    r.socket.receive(sealed(r.peer, delay))
    r.socket.receive(
      sealed(
        r.peer,
        msg({
          case: 'loginResponse',
          value: create(LoginResponseSchema, { union: { case: 'peerInfo', value: peerInfo } }),
        }),
      ),
    )
    r.socket.receive(sealed(r.peer, msg({ case: 'peerInfo', value: peerInfo })))
    r.socket.receive(sealed(r.peer, msg({ case: 'cursorData', value: create(CursorDataSchema) })))
    r.socket.receive(fixtureBytes('1.4.5', 'hbbr-enc-06-VideoFrame'))

    expect(await r.nextEvent()).toMatchObject({
      kind: 'message',
      message: { union: { case: 'testDelay' } },
    })
    expect(await r.nextEvent()).toEqual({ kind: 'state', state: { kind: 'active', peerInfo } })
    expect(await r.nextEvent()).toMatchObject({ message: { union: { case: 'peerInfo' } } })
    expect(await r.nextEvent()).toMatchObject({ message: { union: { case: 'cursorData' } } })
    const video = await r.nextEvent()
    if (video.kind !== 'message' || video.message.union.case !== 'videoFrame') {
      throw new Error('expected the VideoFrame')
    }
    const frames = video.message.union.value.union
    expect(frames.case).toBe('vp9s')
    expect(frames.case === 'vp9s' && frames.value.frames[0]?.key).toBe(true)

    const echo = r.peerReceived()[1]
    expect(echo?.union).toMatchObject({
      case: 'testDelay',
      value: { time: 42n, fromClient: false },
    })
    expect(r.socket.sent).toHaveLength(4)
  })

  it('fails closed on the captured Hash with any byte flipped, before logging in', async () => {
    const hash = fixtureBytes('1.4.5', 'hbbr-enc-01-Hash')
    for (let i = 0; i < hash.length; i++) {
      const r = await replayToKeyed('1.4.5', { password: s.password })
      const tampered = hash.slice()
      tampered[i] = (tampered[i] ?? 0) ^ 0x01
      r.socket.receive(tampered)
      expect(await r.nextEvent(), `byte ${i}`).toMatchObject({ state: { kind: 'failed' } })
      expect(r.socket.sent, `byte ${i}`).toHaveLength(2)
      expect(r.socket.closedWith).toBe(1000)
    }
  })
})

describe('relay replay against the captured 1.5.0 peer (key exchange v1)', () => {
  const s = fixtureSession('1.5.0')

  it('picks version 1 and seals the same key the capture did', async () => {
    const r = await replayToKeyed('1.5.0')
    const ours = fromBinary(MessageSchema, sentAt(r.socket, 1))
    const captured = capturedPublicKey('1.5.0')
    if (ours.union.case !== 'publicKey' || captured.union.case !== 'publicKey') {
      throw new Error('not a PublicKey')
    }
    expect(ours.union.value.asymmetricValue).toEqual(s.controllerBoxPk)
    expect(ours.union.value.symmetricValue).toEqual(captured.union.value.symmetricValue)
    expect(ours.union.value.kxVersion).toBe(1)
  })

  it('refuses the captured Hash, sealed under v0 because the capture picked v0', async () => {
    const r = await replayToKeyed('1.5.0', { password: s.password })
    r.socket.receive(fixtureBytes('1.5.0', 'hbbr-enc-01-Hash'))
    const failed = await r.nextEvent()
    expect(failed).toMatchObject({ kind: 'state', state: { kind: 'failed' } })
    expect(r.socket.sent).toHaveLength(2)
  })

  it('logs in over the v1 subkeys with the captured identity', async () => {
    const r = await replayToKeyed('1.5.0', { password: s.password })
    r.socket.receive(
      sealed(
        r.peer,
        msg({ case: 'hash', value: create(HashSchema, { salt: s.salt, challenge: s.challenge }) }),
      ),
    )
    expect(await r.nextEvent()).toEqual({ kind: 'state', state: { kind: 'connecting' } })
    await r.socket.waitForSent(3)
    const login = r.peerReceived()[0]
    expect(login?.union.case === 'loginRequest' && login.union.value.password).toEqual(
      await passwordHash(s.password, s.salt, s.challenge),
    )
  })
})

describe('fail-closed identity checks on the fixtures', () => {
  it('fails before sending anything when the server-signed key has any byte flipped', async () => {
    const blob = signedPeerPk('1.5.0')
    for (let i = 0; i < blob.length; i++) {
      const tampered = blob.slice()
      tampered[i] = (tampered[i] ?? 0) ^ 0x01
      const started = startFixture('1.5.0', { signedPeerPk: tampered })
      expect(await started.nextEvent(), `byte ${i}`).toMatchObject({ state: { kind: 'failed' } })
      expect(started.socket.sent, `byte ${i}`).toHaveLength(0)
    }
  })

  it('never sends its key when any byte of the signed SignedId blob is flipped', async () => {
    const frame = fixtureBytes('1.5.0', 'hbbr-signed-id')
    const signed = fromBinary(MessageSchema, frame)
    if (signed.union.case !== 'signedId') throw new Error('not a SignedId')
    // The blob is the frame's tail; the protobuf shell in front is covered by tamper.test.ts.
    for (let i = frame.length - signed.union.value.id.length; i < frame.length; i++) {
      const tampered = frame.slice()
      tampered[i] = (tampered[i] ?? 0) ^ 0x01
      const started = startFixture('1.5.0')
      started.socket.receive(tampered)
      expect(await started.nextEvent(), `byte ${i}`).toMatchObject({ state: { kind: 'failed' } })
      expect(started.socket.sent, `byte ${i}`).toHaveLength(1)
    }
  })
})
