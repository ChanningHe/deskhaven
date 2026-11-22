import { blake2b } from '@noble/hashes/blake2.js'
import nacl from 'tweetnacl'

import { concatBytes, u32le, utf8 } from '../bytes'
import { BOX_KEY_BYTES, SECRET_KEY_BYTES } from './nacl'

/** Newest key exchange version this client speaks (hbb_common `tcp::KX_VERSION_LATEST`). */
export const KX_VERSION_LATEST = 1

export function kxVersionFor(advertised: number): number {
  return Math.min(advertised, KX_VERSION_LATEST)
}

/** What both sides saw during the key exchange; mixed into the version 1 subkeys. */
export interface KxTranscript {
  /** Ephemeral box key of the side that sent the sealed key (this client). */
  initiatorPk: Uint8Array
  /** Ephemeral box key the sealed key was sealed to (the peer's, from its SignedId). */
  responderPk: Uint8Array
  advertised: number
  picked: number
}

export type KxRole = 'initiator' | 'responder'

const KX_SPLIT_CONTEXT = utf8('rdkx-spl')
const KX_SPLIT_INITIATOR = 1
const KX_SPLIT_RESPONDER = 2

function deriveSubkey(key: Uint8Array, direction: number, t: KxTranscript): Uint8Array {
  if (t.initiatorPk.length !== BOX_KEY_BYTES || t.responderPk.length !== BOX_KEY_BYTES) {
    throw new Error('key exchange transcript has a public key of the wrong length')
  }
  const input = concatBytes(
    KX_SPLIT_CONTEXT,
    Uint8Array.of(direction),
    u32le(t.advertised),
    u32le(t.picked),
    t.initiatorPk,
    t.responderPk,
  )
  return blake2b(input, { key, dkLen: SECRET_KEY_BYTES })
}

/** Version 1 per-direction keys: the initiator sends under `initiator`, the responder under `responder`. */
export function splitKeys(
  key: Uint8Array,
  t: KxTranscript,
): { initiator: Uint8Array; responder: Uint8Array } {
  return {
    initiator: deriveSubkey(key, KX_SPLIT_INITIATOR, t),
    responder: deriveSubkey(key, KX_SPLIT_RESPONDER, t),
  }
}

/** secretbox nonce for frame `seq`: little-endian u64 in the first 8 bytes, the rest zero. */
export function frameNonce(seq: bigint): Uint8Array {
  const nonce = new Uint8Array(nacl.secretbox.nonceLength)
  new DataView(nonce.buffer).setBigUint64(0, seq, true)
  return nonce
}

export class FrameDecryptError extends Error {
  constructor() {
    super('frame decryption failed')
    this.name = 'FrameDecryptError'
  }
}

/**
 * Stream encryption after the key exchange (hbb_common `tcp::Encrypt`): every frame is one
 * secretbox, and each direction counts its own frames from 1.
 */
export class FrameCipher {
  readonly #sendKey: Uint8Array
  readonly #recvKey: Uint8Array
  #sent = 0n
  #received = 0n

  private constructor(sendKey: Uint8Array, recvKey: Uint8Array) {
    if (sendKey.length !== SECRET_KEY_BYTES || recvKey.length !== SECRET_KEY_BYTES) {
      throw new Error('frame key has the wrong length')
    }
    this.#sendKey = sendKey
    this.#recvKey = recvKey
  }

  /** Version 0: one key for both directions. */
  static v0(key: Uint8Array): FrameCipher {
    return new FrameCipher(key, key)
  }

  /** Version 1: per-direction subkeys bound to the key exchange transcript. */
  static v1(key: Uint8Array, t: KxTranscript, role: KxRole): FrameCipher {
    if (key.length !== SECRET_KEY_BYTES) throw new Error('frame key has the wrong length')
    const { initiator, responder } = splitKeys(key, t)
    return role === 'initiator'
      ? new FrameCipher(initiator, responder)
      : new FrameCipher(responder, initiator)
  }

  seal(plain: Uint8Array): Uint8Array<ArrayBuffer> {
    this.#sent += 1n
    // tweetnacl returns a fresh, ArrayBuffer-backed array; its typings just predate the generic.
    return nacl.secretbox(plain, frameNonce(this.#sent), this.#sendKey) as Uint8Array<ArrayBuffer>
  }

  /** Throws FrameDecryptError when the frame does not authenticate under the next nonce. */
  open(frame: Uint8Array): Uint8Array {
    // The peer's decoder passes frames of at most one byte through unencrypted and uncounted
    // (hbb_common `Encrypt::dec`); the counters must stay in step with it.
    if (frame.length <= 1) return frame
    this.#received += 1n
    const plain = nacl.secretbox.open(frame, frameNonce(this.#received), this.#recvKey)
    if (!plain) throw new FrameDecryptError()
    return plain
  }
}
