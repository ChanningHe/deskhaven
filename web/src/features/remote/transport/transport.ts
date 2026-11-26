import type { FrameCipher } from '../crypto/frame-cipher'
import type { Message } from '../proto/gen/message_pb.js'

/** `messages` is single-use; it ends after a local `close` and throws TransportError when the link fails. */
export interface Transport {
  send(message: Message): void
  readonly messages: AsyncIterable<Message>
  close(): void
}

/** A transport without encryption of its own (relay): the handshake keys it once. */
export interface KeyedTransport extends Transport {
  /** Frames after this call are sealed and opened with `cipher`. Callable once. */
  startEncryption(cipher: FrameCipher): void
}

export class TransportError extends Error {
  constructor(reason: string) {
    super(reason)
    this.name = 'TransportError'
  }
}
