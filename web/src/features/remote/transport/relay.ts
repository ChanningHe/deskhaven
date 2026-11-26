import { create, fromBinary, toBinary } from '@bufbuild/protobuf'

import { AsyncQueue } from '../async-queue'
import type { FrameCipher } from '../crypto/frame-cipher'
import { MessageSchema, type Message } from '../proto/gen/message_pb.js'
import { RendezvousMessageSchema, RequestRelaySchema } from '../proto/gen/rendezvous_pb.js'
import { TransportError, type KeyedTransport } from './transport'

export const RELAY_PATH = '/ws/relay'

const NORMAL_CLOSURE = 1000

/** What the peer's own WebSocket stream accepts (`DEFAULT_MAX_MESSAGE_SIZE`). */
export const MAX_FRAME_BYTES = 64 << 20

/**
 * A backlog means the consumer stalled. Frames: ten times the client's own video queue, since
 * cursor, misc and TestDelay frames share this queue; about 20 s of 60 fps video. Bytes: one
 * largest frame, so an empty backlog always has room for any frame the peer may legally send.
 */
export const MAX_QUEUED_FRAMES = 1200
export const MAX_QUEUED_BYTES = MAX_FRAME_BYTES

export function relayUrl(location: Pick<Location, 'protocol' | 'host'>): string {
  const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:'
  return `${scheme}//${location.host}${RELAY_PATH}`
}

export interface RelayOptions {
  url: string
  /** The rendezvous server's public key, base64, exactly as the relay server checks it. */
  licenceKey: string
  uuid: string
  targetId: string
  createSocket?: (url: string) => WebSocket
}

/** The first frame asks the relay server to pair us by `uuid`; then every binary message is one frame. */
export class RelayTransport implements KeyedTransport {
  readonly messages: AsyncIterable<Message>
  readonly #socket: WebSocket
  readonly #frames = new AsyncQueue<Uint8Array>()
  #open = false
  #closed = false
  #cipher: FrameCipher | null = null
  #queuedBytes = 0
  #queuedFrames = 0

  constructor(options: RelayOptions) {
    const createSocket = options.createSocket ?? ((url: string) => new WebSocket(url))
    this.#socket = createSocket(options.url)
    this.#socket.binaryType = 'arraybuffer'
    this.#socket.onopen = () => {
      this.#open = true
      this.#socket.send(requestRelay(options))
    }
    this.#socket.onmessage = (event: MessageEvent) => {
      this.#onFrame(event.data)
    }
    this.#socket.onclose = (event: CloseEvent) => {
      this.#fail(`relay connection closed (code ${event.code})`)
    }
    this.#socket.onerror = () => {
      this.#fail('relay connection failed')
    }
    this.messages = this.#decode()
  }

  send(message: Message): void {
    if (this.#closed) throw new TransportError('relay connection is closed')
    if (!this.#open) throw new TransportError('relay connection is not open yet')
    const plain = toBinary(MessageSchema, message)
    this.#socket.send(this.#cipher ? this.#cipher.seal(plain) : plain)
  }

  startEncryption(cipher: FrameCipher): void {
    if (this.#cipher) throw new Error('relay encryption already started')
    this.#cipher = cipher
  }

  close(): void {
    if (this.#closed) return
    this.#closed = true
    this.#detach()
    this.#socket.close(NORMAL_CLOSURE)
    this.#frames.end()
  }

  #onFrame(data: unknown): void {
    if (!(data instanceof ArrayBuffer)) {
      this.#fail('relay sent a non-binary message')
      return
    }
    if (data.byteLength > MAX_FRAME_BYTES) {
      this.#fail(`relay frame of ${data.byteLength} bytes exceeds ${MAX_FRAME_BYTES}`)
      return
    }
    this.#queuedFrames += 1
    this.#queuedBytes += data.byteLength
    if (this.#queuedFrames > MAX_QUEUED_FRAMES || this.#queuedBytes > MAX_QUEUED_BYTES) {
      this.#fail(
        `relay backlog of ${this.#queuedFrames} frames / ${this.#queuedBytes} bytes exceeds ` +
          `${MAX_QUEUED_FRAMES} / ${MAX_QUEUED_BYTES}`,
      )
      return
    }
    this.#frames.push(new Uint8Array(data))
  }

  #fail(reason: string): void {
    if (this.#closed) return
    this.#closed = true
    this.#detach()
    this.#socket.close(NORMAL_CLOSURE)
    this.#frames.end(new TransportError(reason))
  }

  #detach(): void {
    this.#socket.onopen = null
    this.#socket.onmessage = null
    this.#socket.onclose = null
    this.#socket.onerror = null
  }

  // Frames are opened when the consumer pulls them, not on arrival, so a cipher installed
  // between two pulls applies exactly from the next frame on.
  async *#decode(): AsyncGenerator<Message, void, undefined> {
    for await (const frame of this.#frames) {
      this.#queuedFrames -= 1
      this.#queuedBytes -= frame.byteLength
      let message: Message
      try {
        message = fromBinary(MessageSchema, this.#cipher ? this.#cipher.open(frame) : frame)
      } catch (error) {
        const reason = error instanceof Error ? error.message : String(error)
        this.#fail(`bad frame from relay: ${reason}`)
        throw new TransportError(`bad frame from relay: ${reason}`)
      }
      yield message
    }
  }
}

function requestRelay(options: RelayOptions): Uint8Array<ArrayBuffer> {
  return toBinary(
    RendezvousMessageSchema,
    create(RendezvousMessageSchema, {
      union: {
        case: 'requestRelay',
        value: create(RequestRelaySchema, {
          licenceKey: options.licenceKey,
          uuid: options.uuid,
          id: options.targetId,
        }),
      },
    }),
  )
}
