import { concatBytes, utf8 } from '../bytes'

async function sha256(data: Uint8Array<ArrayBuffer>): Promise<Uint8Array<ArrayBuffer>> {
  return new Uint8Array(await crypto.subtle.digest('SHA-256', data))
}

export function salted(password: string, salt: string): Promise<Uint8Array<ArrayBuffer>> {
  return sha256(concatBytes(utf8(password), utf8(salt)))
}

export function proof(salted: Uint8Array, challenge: string): Promise<Uint8Array> {
  return sha256(concatBytes(salted, utf8(challenge)))
}

/** Login proof the peer checks: SHA256(SHA256(password ‖ salt) ‖ challenge), all UTF-8. */
export async function passwordHash(
  password: string,
  salt: string,
  challenge: string,
): Promise<Uint8Array> {
  return proof(await salted(password, salt), challenge)
}
