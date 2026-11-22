import { fromBinary } from '@bufbuild/protobuf'
import nacl from 'tweetnacl'
import { describe, expect, it } from 'vitest'

import { FIXTURE_VERSIONS, fixtureBytes, fixtureSession } from '../__fixtures__/fixtures'
import { decodeBase64 } from '../bytes'
import { MessageSchema } from '../proto/gen/message_pb.js'
import { IdPkSchema, RendezvousMessageSchema } from '../proto/gen/rendezvous_pb.js'
import {
  FrameCipher,
  FrameDecryptError,
  frameNonce,
  kxVersionFor,
  splitKeys,
  type KxTranscript,
} from './frame-cipher'
import { boxKeyPairFromSecret, openSigned, sealKey } from './nacl'
import { passwordHash } from './password'

function hex(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('')
}

// TextEncoder output is a foreign-realm Uint8Array under jsdom, which tweetnacl rejects.
function ascii(text: string): Uint8Array<ArrayBuffer> {
  return Uint8Array.from(text, (c) => c.charCodeAt(0))
}

function unhex(text: string): Uint8Array {
  return Uint8Array.from(text.match(/../g) ?? [], (b) => parseInt(b, 16))
}

function signedPeerPk(version: (typeof FIXTURE_VERSIONS)[number]): Uint8Array {
  const rr = fromBinary(RendezvousMessageSchema, fixtureBytes(version, 'hbbs-relay-response'))
  if (rr.union.case !== 'relayResponse' || rr.union.value.union.case !== 'pk') {
    throw new Error('fixture is not a RelayResponse carrying pk')
  }
  return rr.union.value.union.value
}

describe.each(FIXTURE_VERSIONS)('openSigned with the %s fixture', (version) => {
  const s = fixtureSession(version)

  it('opens the server-signed peer key and yields the device key', () => {
    const opened = openSigned(signedPeerPk(version), decodeBase64(s.serverPub))
    expect(opened).not.toBeNull()
    const idPk = fromBinary(IdPkSchema, opened ?? new Uint8Array())
    expect(idPk.id).toBe(s.peerId)
    expect(idPk.pk).toEqual(s.devicePk)
  })

  it('opens the device-signed SignedId and yields the ephemeral box key', () => {
    const msg = fromBinary(MessageSchema, fixtureBytes(version, 'hbbr-signed-id'))
    if (msg.union.case !== 'signedId') throw new Error('not a SignedId')
    const opened = openSigned(msg.union.value.id, s.devicePk)
    const idPk = fromBinary(IdPkSchema, opened ?? new Uint8Array())
    expect(idPk.pk).toEqual(s.peerBoxPk)
    expect(idPk.kxVersion).toBe(s.peerKxVersion)
    expect(idPk.dtlsFingerprint).toBe('')
  })

  it('rejects a blob signed by another key', () => {
    expect(openSigned(signedPeerPk(version), s.devicePk)).toBeNull()
  })

  it('rejects a key of the wrong length', () => {
    expect(openSigned(signedPeerPk(version), new Uint8Array(31))).toBeNull()
  })
})

describe.each(FIXTURE_VERSIONS)('sealKey with the %s fixture', (version) => {
  it('reproduces the sealed key the controller sent on the wire', () => {
    const s = fixtureSession(version)
    const msg = fromBinary(MessageSchema, fixtureBytes(version, 'hbbr-public-key'))
    if (msg.union.case !== 'publicKey') throw new Error('not a PublicKey')
    const ours = boxKeyPairFromSecret(s.controllerBoxSk)
    expect(ours.publicKey).toEqual(s.controllerBoxPk)
    expect(sealKey(s.symmetricKey, s.peerBoxPk, ours.secretKey)).toEqual(
      msg.union.value.symmetricValue,
    )
  })
})

describe.each(FIXTURE_VERSIONS)('FrameCipher v0 with the %s fixture', (version) => {
  const s = fixtureSession(version)

  it('opens the first received frame as Hash with the fixture salt and challenge', () => {
    const plain = FrameCipher.v0(s.symmetricKey).open(fixtureBytes(version, 'hbbr-enc-01-Hash'))
    const msg = fromBinary(MessageSchema, plain)
    expect(msg.union.case).toBe('hash')
    expect(msg.union.value).toMatchObject({ salt: s.salt, challenge: s.challenge })
  })

  it('needs the frame counter: the Hash frame does not open as frame 2', () => {
    const cipher = FrameCipher.v0(s.symmetricKey)
    cipher.open(fixtureBytes(version, 'hbbr-enc-01-Hash'))
    expect(() => cipher.open(fixtureBytes(version, 'hbbr-enc-01-Hash'))).toThrow(FrameDecryptError)
  })

  it('opens frame 6 as a VP9 key frame under nonce 6', () => {
    const frame = fixtureBytes(version, 'hbbr-enc-06-VideoFrame')
    const plain = nacl.secretbox.open(frame, frameNonce(6n), s.symmetricKey)
    const msg = fromBinary(MessageSchema, plain ?? new Uint8Array())
    if (msg.union.case !== 'videoFrame') throw new Error('not a VideoFrame')
    const vp9 = msg.union.value.union
    expect(vp9.case).toBe('vp9s')
    expect(vp9.case === 'vp9s' && vp9.value.frames[0]?.key).toBe(true)
  })
})

describe('FrameCipher', () => {
  const key = nacl.randomBytes(32)

  it('numbers frames from 1 per direction with a little-endian u64 nonce', () => {
    const cipher = FrameCipher.v0(key)
    const first = cipher.seal(ascii('a'))
    const second = cipher.seal(ascii('a'))
    expect(nacl.secretbox.open(first, frameNonce(1n), key)).toEqual(ascii('a'))
    expect(nacl.secretbox.open(second, frameNonce(2n), key)).toEqual(ascii('a'))
    expect(hex(frameNonce(0x0102n))).toBe('0201' + '00'.repeat(22))
  })

  it('keeps the receive counter independent of the send counter', () => {
    const a = FrameCipher.v0(key)
    const b = FrameCipher.v0(key)
    a.seal(ascii('x'))
    a.seal(ascii('y'))
    expect(a.open(b.seal(ascii('hello')))).toEqual(ascii('hello'))
  })

  it('rejects a frame with any byte flipped', () => {
    const frame = FrameCipher.v0(key).seal(ascii('payload'))
    for (let i = 0; i < frame.length; i++) {
      const tampered = frame.slice()
      tampered[i] = (tampered[i] ?? 0) ^ 0x01
      expect(() => FrameCipher.v0(key).open(tampered), `byte ${i}`).toThrow(FrameDecryptError)
    }
  })

  it('passes frames of at most one byte through without counting them, as the peer does', () => {
    const cipher = FrameCipher.v0(key)
    expect(cipher.open(new Uint8Array())).toEqual(new Uint8Array())
    expect(cipher.open(Uint8Array.of(7))).toEqual(Uint8Array.of(7))
    expect(cipher.open(FrameCipher.v0(key).seal(ascii('one')))).toEqual(ascii('one'))
  })

  it('refuses keys of the wrong length', () => {
    expect(() => FrameCipher.v0(new Uint8Array(16))).toThrow()
  })
})

describe('key exchange version 1', () => {
  // Expected values from hbb_common fae47e0 `tcp::Encrypt::new_split` (throwaway Rust program,
  // see the slice report), over the 1.5.0 fixture's one-shot keys.
  const s = fixtureSession('1.5.0')
  const transcript = (advertised: number): KxTranscript => ({
    initiatorPk: s.controllerBoxPk,
    responderPk: s.peerBoxPk,
    advertised,
    picked: kxVersionFor(advertised),
  })

  it('derives the same subkeys as hbb_common for advertised 1', () => {
    const keys = splitKeys(s.symmetricKey, transcript(1))
    expect(hex(keys.initiator)).toBe(
      'd565acb87e0372b36e0adba3bdaeb9cdb99f07f5d802a04edc2741e429c7a631',
    )
    expect(hex(keys.responder)).toBe(
      'ccc5b1a5699d7e968939766293d9eaa3d3da77eed672cc8cffd85c3bafcbe735',
    )
  })

  it('binds the advertised version: advertised 7 picks 1 but derives other subkeys', () => {
    expect(kxVersionFor(7)).toBe(1)
    const keys = splitKeys(s.symmetricKey, transcript(7))
    expect(hex(keys.initiator)).toBe(
      '9b90ae49291c90473a287013980d851b50e95666bf2e47e6f3f01fcf8e30c66b',
    )
    expect(hex(keys.responder)).toBe(
      'e433a6e0fc1e5f9c70e96a902f485238e80d3a96e8dad40d9cef31b4b41f39b0',
    )
  })

  it('seals initiator frames byte-for-byte like hbb_common', () => {
    const cipher = FrameCipher.v1(s.symmetricKey, transcript(1), 'initiator')
    expect(hex(cipher.seal(ascii('web client')))).toBe(
      'afee237a1b369e05d83d1798d62307aa95fd19e126d10548a88b',
    )
    expect(hex(cipher.seal(ascii('web client')))).toBe(
      'a1f04e87ec02374ed50cf27916dcf860b140d5dcea57e9020d3e',
    )
  })

  it('opens what the responder seals and rejects it under version 0', () => {
    const peerFrame = unhex('77a426c3483b4695a8d50d13428add63114395638a8880cc630f')
    const ours = FrameCipher.v1(s.symmetricKey, transcript(1), 'initiator')
    expect(ours.open(peerFrame)).toEqual(ascii('peer frame'))
    expect(() => FrameCipher.v0(s.symmetricKey).open(peerFrame)).toThrow(FrameDecryptError)
  })

  it('refuses a transcript key of the wrong length', () => {
    expect(() =>
      splitKeys(s.symmetricKey, { ...transcript(1), responderPk: new Uint8Array(31) }),
    ).toThrow()
  })

  it('never picks a version above the latest', () => {
    expect([0, 1, 2].map(kxVersionFor)).toEqual([0, 1, 1])
  })
})

describe('passwordHash', () => {
  it('is SHA256(SHA256(password ‖ salt) ‖ challenge)', async () => {
    const s = fixtureSession('1.5.0')
    const salted = new Uint8Array(await crypto.subtle.digest('SHA-256', ascii(s.password + s.salt)))
    const expected = new Uint8Array(
      await crypto.subtle.digest('SHA-256', new Uint8Array([...salted, ...ascii(s.challenge)])),
    )
    const got = await passwordHash(s.password, s.salt, s.challenge)
    expect(got).toHaveLength(32)
    expect(got).toEqual(expected)
  })

  it('matches a fixed vector', async () => {
    // Python: sha256(sha256(b'pwsalt').digest() + b'chal').hexdigest()
    expect(hex(await passwordHash('pw', 'salt', 'chal'))).toBe(
      '6b4858ead261a75c0d1cb54a609d649c19fcfee7e4f7e26cf339099c3f0657a4',
    )
  })
})
