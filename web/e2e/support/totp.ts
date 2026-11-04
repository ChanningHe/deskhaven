// RFC 6238 TOTP (HMAC-SHA1, 6 digits, 30 s steps: what deskhaven provisions), playing the
// authenticator app. The server accepts one step either side of its own and each step
// once per user, so a second code within the same 30 s comes from the next step.
import { createHmac } from 'node:crypto'

const STEP_MS = 30_000
const DIGITS = 6
const BASE32 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567'

/** RFC 4648 base32, as the console shows it (grouped with spaces, maybe padded). */
function base32Decode(text: string): Buffer {
  const clean = text.replace(/[\s=]/g, '').toUpperCase()
  let bits = 0
  let value = 0
  const bytes: number[] = []
  for (const char of clean) {
    const index = BASE32.indexOf(char)
    if (index < 0) throw new Error(`not base32: ${char}`)
    value = (value << 5) | index
    bits += 5
    if (bits >= 8) {
      bits -= 8
      bytes.push((value >>> bits) & 0xff)
    }
  }
  return Buffer.from(bytes)
}

/** RFC 4226 HOTP of `counter`. */
export function hotp(key: Buffer, counter: number): string {
  const message = Buffer.alloc(8)
  message.writeBigUInt64BE(BigInt(counter))
  const digest = createHmac('sha1', key).update(message).digest()
  const offset = (digest.at(-1) ?? 0) & 0x0f
  const code = digest.readUInt32BE(offset) & 0x7fffffff
  return String(code % 10 ** DIGITS).padStart(DIGITS, '0')
}

/** An authenticator for one seed that never hands out the same step twice. */
export class Authenticator {
  private readonly key: Buffer
  private lastStep = -1

  constructor(secret: string) {
    this.key = base32Decode(secret)
  }

  /** A code the server accepts now: the current step, or the next one if that was used. */
  nextCode(now = Date.now()): string {
    const step = Math.max(Math.floor(now / STEP_MS), this.lastStep + 1)
    this.lastStep = step
    return hotp(this.key, step)
  }
}
