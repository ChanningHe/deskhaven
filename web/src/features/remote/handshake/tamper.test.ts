import { fromBinary } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  FIXTURE_VERSIONS,
  fixtureBytes,
  fixtureSession,
  type FixtureFile,
} from '../__fixtures__/fixtures'
import { decodeBase64 } from '../bytes'
import { FrameCipher, FrameDecryptError } from '../crypto/frame-cipher'
import { openSigned } from '../crypto/nacl'
import { MessageSchema } from '../proto/gen/message_pb.js'
import { IdPkSchema, RendezvousMessageSchema } from '../proto/gen/rendezvous_pb.js'

// Byte-flip sweeps over whole wire frames, decoded with @bufbuild/protobuf. Bytes covered by a
// signature or a MAC must never survive; the protobuf shell around them (tags, lengths, the
// unsigned RelayResponse uuid/relay_server/version) can, because the decoder is lenient and
// those bytes carry nothing the identity check relies on. The survivors are pinned so a decoder
// change shows up here.

function blobStart(frame: Uint8Array, blob: Uint8Array): number {
  return frame.length - blob.length
}

function verifiesAs(
  bytes: Uint8Array,
  read: (bytes: Uint8Array) => Uint8Array | null,
  signerPk: Uint8Array,
  id: string,
): boolean {
  try {
    const blob = read(bytes)
    const opened = blob && openSigned(blob, signerPk)
    return opened !== null && fromBinary(IdPkSchema, opened).id === id
  } catch {
    return false
  }
}

function survivors(frame: Uint8Array, accepted: (bytes: Uint8Array) => boolean): number[] {
  const out: number[] = []
  for (let i = 0; i < frame.length; i++) {
    const tampered = frame.slice()
    tampered[i] = (tampered[i] ?? 0) ^ 0x01
    if (accepted(tampered)) out.push(i)
  }
  return out
}

function range(from: number, to: number): number[] {
  return Array.from({ length: to - from + 1 }, (_, i) => from + i)
}

const relayPk = (bytes: Uint8Array): Uint8Array | null => {
  const m = fromBinary(RendezvousMessageSchema, bytes)
  return m.union.case === 'relayResponse' && m.union.value.union.case === 'pk'
    ? m.union.value.union.value
    : null
}

const signedId = (bytes: Uint8Array): Uint8Array | null => {
  const m = fromBinary(MessageSchema, bytes)
  return m.union.case === 'signedId' ? m.union.value.id : null
}

describe.each(FIXTURE_VERSIONS)('byte flips in the %s fixture', (version) => {
  const s = fixtureSession(version)

  it('RelayResponse: only shell and unsigned fields survive, never the signed pk', () => {
    const frame = fixtureBytes(version, 'hbbs-relay-response')
    const start = blobStart(frame, relayPk(frame) ?? new Uint8Array())
    const ok = survivors(frame, (b) => verifiesAs(b, relayPk, decodeBase64(s.serverPub), s.peerId))
    expect(start).toBe(69)
    expect(ok.every((i) => i < start)).toBe(true)
    expect(ok).toEqual([0, 4, ...range(6, 42), ...range(44, 60), ...range(62, 67)])
  })

  it('SignedId: only the outer tag and lengths survive, never the signed blob', () => {
    const frame = fixtureBytes(version, 'hbbr-signed-id')
    const start = blobStart(frame, signedId(frame) ?? new Uint8Array())
    const ok = survivors(frame, (b) => verifiesAs(b, signedId, s.devicePk, s.peerId))
    expect(start).toBe(4)
    expect(ok).toEqual([0, 1, 2])
  })

  it.each<FixtureFile>(['hbbr-enc-01-Hash', 'hbbr-enc-06-VideoFrame'])(
    '%s: every flipped byte fails authentication',
    (file) => {
      const frame = fixtureBytes(version, file)
      const seq = file === 'hbbr-enc-01-Hash' ? 1 : 6
      const opens = (bytes: Uint8Array): boolean => {
        const cipher = FrameCipher.v0(s.symmetricKey)
        const earlier = FrameCipher.v0(s.symmetricKey)
        for (let n = 1; n < seq; n++) cipher.open(earlier.seal(new Uint8Array(2)))
        try {
          cipher.open(bytes)
          return true
        } catch (error) {
          if (!(error instanceof FrameDecryptError)) throw error
          return false
        }
      }
      expect(opens(frame)).toBe(true)
      expect(survivors(frame, opens)).toEqual([])
    },
  )
})
