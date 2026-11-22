import nacl from 'tweetnacl'

export const BOX_KEY_BYTES = nacl.box.publicKeyLength
export const SECRET_KEY_BYTES = nacl.secretbox.keyLength

const ZERO_BOX_NONCE = new Uint8Array(nacl.box.nonceLength)

/**
 * The message inside an ed25519 signed blob (64-byte signature ‖ message, libsodium's combined
 * mode), or null when the signature does not verify under `signerPk`.
 */
export function openSigned(blob: Uint8Array, signerPk: Uint8Array): Uint8Array | null {
  if (signerPk.length !== nacl.sign.publicKeyLength) return null
  return nacl.sign.open(blob, signerPk)
}

/**
 * Seals the session key to the peer's ephemeral box key. The all-zero nonce is safe only
 * because both keypairs are fresh per connection; it is what the peer expects.
 */
export function sealKey(key: Uint8Array, peerBoxPk: Uint8Array, ourBoxSk: Uint8Array): Uint8Array {
  return nacl.box(key, ZERO_BOX_NONCE, peerBoxPk, ourBoxSk)
}

export function boxKeyPairFromSecret(secret: Uint8Array): nacl.BoxKeyPair {
  return nacl.box.keyPair.fromSecretKey(secret)
}

export function randomBytes(length: number): Uint8Array {
  return nacl.randomBytes(length)
}
