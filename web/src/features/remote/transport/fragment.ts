import { concatBytes } from '../bytes'
import { MAX_FRAME_BYTES } from './relay'

// Framing of one logical message on the data channel, as hbb_common does it: each data-channel
// message is a 1-byte header and up to MAX_FRAGMENT_PAYLOAD bytes.

export const FRAG_MORE = 1
export const FRAG_END = 0
/** Kept well under the SCTP 65536-byte message size, header included. */
export const MAX_FRAGMENT_PAYLOAD = 60_000
/** One session's messages must fit either transport, so the data channel takes what the relay takes. */
export const MAX_MESSAGE_BYTES = MAX_FRAME_BYTES

/** Always at least one fragment: a zero-length data-channel message reads as end of stream. */
export function fragment(message: Uint8Array): Uint8Array<ArrayBuffer>[] {
  const parts: Uint8Array<ArrayBuffer>[] = []
  let offset = 0
  do {
    const end = Math.min(offset + MAX_FRAGMENT_PAYLOAD, message.length)
    const part = new Uint8Array(end - offset + 1)
    part[0] = end < message.length ? FRAG_MORE : FRAG_END
    part.set(message.subarray(offset, end), 1)
    parts.push(part)
    offset = end
  } while (offset < message.length)
  return parts
}

/** Rebuilds messages from fragments; any framing violation throws and the stream is dead. */
export class Reassembler {
  readonly #limit: number
  #parts: Uint8Array[] = []
  #size = 0

  constructor(limit: number = MAX_MESSAGE_BYTES) {
    this.#limit = limit
  }

  push(chunk: Uint8Array): Uint8Array | null {
    const header = chunk[0]
    if (header === undefined) throw new Error('empty data channel message')
    if (header !== FRAG_END && header !== FRAG_MORE) {
      throw new Error(`fragment header ${header} is neither FRAG_END nor FRAG_MORE`)
    }
    // A payload-less FRAG_MORE adds nothing, so the size cap would never stop a stream of them.
    if (header === FRAG_MORE && chunk.length === 1) {
      throw new Error('FRAG_MORE fragment carries no payload')
    }
    const payload = chunk.subarray(1)
    if (this.#size + payload.length > this.#limit) {
      throw new Error(`reassembled message exceeds ${this.#limit} bytes`)
    }
    this.#parts.push(payload)
    this.#size += payload.length
    if (header === FRAG_MORE) return null
    const message = this.#parts.length === 1 ? payload : concatBytes(...this.#parts)
    this.#parts = []
    this.#size = 0
    return message
  }
}
