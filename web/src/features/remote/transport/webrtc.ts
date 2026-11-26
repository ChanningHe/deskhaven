import { fromBinary, toBinary } from '@bufbuild/protobuf'

import { AsyncQueue } from '../async-queue'
import { MessageSchema, type Message } from '../proto/gen/message_pb.js'
import { MAX_MESSAGE_BYTES, Reassembler, fragment } from './fragment'
import { MAX_QUEUED_BYTES, MAX_QUEUED_FRAMES } from './relay'
import { TransportError, type Transport } from './transport'
import { decodeAnswer, encodeOffer, sdpFingerprint } from './webrtc-sdp'

/** The label the controlled side expects. */
export const DATA_CHANNEL_LABEL = 'bootstrap'

export interface DataChannelLike {
  binaryType: BinaryType
  send(data: Uint8Array<ArrayBuffer>): void
  close(): void
  onopen: ((event: Event) => unknown) | null
  onclose: ((event: Event) => unknown) | null
  onerror: ((event: RTCErrorEvent) => unknown) | null
  onmessage: ((event: MessageEvent) => unknown) | null
}

export interface PeerConnectionLike {
  createDataChannel(label: string, init: RTCDataChannelInit): DataChannelLike
  createOffer(): Promise<RTCSessionDescriptionInit>
  setConfiguration(configuration: RTCConfiguration): void
  setLocalDescription(description: RTCSessionDescriptionInit): Promise<void>
  setRemoteDescription(description: RTCSessionDescriptionInit): Promise<void>
  readonly remoteDescription: { readonly sdp: string } | null
  addIceCandidate(candidate: RTCIceCandidateInit): Promise<void>
  readonly connectionState: RTCPeerConnectionState
  onicecandidate: ((event: RTCPeerConnectionIceEvent) => unknown) | null
  onconnectionstatechange: ((event: Event) => unknown) | null
  close(): void
}

export interface WebRtcOfferer extends Transport {
  readonly offer: string
  readonly opened: Promise<void>
  readonly connectionState: RTCPeerConnectionState
  readonly localCandidates: AsyncIterable<string>
  /** Applies the peer's answer; resolves with the DTLS fingerprint it commits the peer to. */
  accept(answer: string, iceServers: RTCIceServer[]): Promise<string>
  /** False when the candidate is malformed or refused: one bad candidate only costs its path. */
  addRemoteCandidate(candidate: string): Promise<boolean>
}

export type CreatePeerConnection = () => PeerConnectionLike

/** DTLS keys it, so the handshake runs on it without a stream cipher. */
export class WebRtcTransport implements WebRtcOfferer {
  readonly offer: string
  readonly opened: Promise<void>
  readonly messages: AsyncIterable<Message>
  readonly #pc: PeerConnectionLike
  readonly #channel: DataChannelLike
  readonly #description: RTCSessionDescriptionInit
  readonly #candidates = new AsyncQueue<string>()
  readonly #frames = new AsyncQueue<Uint8Array>()
  readonly #reassembler = new Reassembler()
  #settleOpened: { resolve: () => void; reject: (error: Error) => void } | null = null
  #open = false
  #closed = false
  #queuedBytes = 0
  #queuedFrames = 0

  /**
   * ICE servers come with the answer, so gathering (setLocalDescription) waits for `accept`:
   * servers set later only apply to a later gathering.
   */
  static async create(
    createPeerConnection: CreatePeerConnection = () => new RTCPeerConnection(),
  ): Promise<WebRtcTransport> {
    const pc = createPeerConnection()
    try {
      const channel = pc.createDataChannel(DATA_CHANNEL_LABEL, { ordered: true })
      const description = await pc.createOffer()
      return new WebRtcTransport(pc, channel, description, encodeOffer(description.sdp ?? ''))
    } catch (error) {
      pc.close()
      throw error
    }
  }

  private constructor(
    pc: PeerConnectionLike,
    channel: DataChannelLike,
    description: RTCSessionDescriptionInit,
    offer: string,
  ) {
    this.#pc = pc
    this.#channel = channel
    this.#description = description
    this.offer = offer
    this.opened = new Promise<void>((resolve, reject) => {
      this.#settleOpened = { resolve, reject }
    })
    this.opened.catch(() => undefined)
    channel.binaryType = 'arraybuffer'
    channel.onopen = () => {
      this.#open = true
      this.#settleOpened?.resolve()
      this.#settleOpened = null
    }
    channel.onmessage = (event: MessageEvent) => {
      this.#onChunk(event.data)
    }
    channel.onclose = () => {
      this.#fail('WebRTC data channel closed')
    }
    channel.onerror = () => {
      this.#fail('WebRTC data channel failed')
    }
    pc.onconnectionstatechange = () => {
      const state = pc.connectionState
      if (state === 'failed' || state === 'closed') this.#fail(`WebRTC connection ${state}`)
    }
    pc.onicecandidate = (event: RTCPeerConnectionIceEvent) => {
      const { candidate } = event
      if (!candidate) {
        this.#candidates.end()
        return
      }
      if (candidate.candidate !== '') this.#candidates.push(JSON.stringify(candidate.toJSON()))
    }
    this.messages = this.#decode()
  }

  get connectionState(): RTCPeerConnectionState {
    return this.#pc.connectionState
  }

  get localCandidates(): AsyncIterable<string> {
    return this.#candidates
  }

  async accept(answer: string, iceServers: RTCIceServer[]): Promise<string> {
    try {
      const sdp = decodeAnswer(answer)
      const fingerprint = sdpFingerprint(sdp)
      if (fingerprint === null) throw new Error('WebRTC answer needs exactly one DTLS fingerprint')
      this.#pc.setConfiguration({ iceServers })
      await this.#pc.setLocalDescription(this.#description)
      await this.#pc.setRemoteDescription({ type: 'answer', sdp })
      if (sdpFingerprint(this.#pc.remoteDescription?.sdp ?? '') !== fingerprint) {
        throw new Error('WebRTC answer fingerprint changed when applied')
      }
      return fingerprint
    } catch (error) {
      const reason = errorText(error)
      this.#fail(`WebRTC answer refused: ${reason}`)
      throw new TransportError(`WebRTC answer refused: ${reason}`)
    }
  }

  async addRemoteCandidate(candidate: string): Promise<boolean> {
    const init = candidateInit(candidate)
    if (!init || this.#closed) return false
    try {
      await this.#pc.addIceCandidate(init)
      return true
    } catch {
      return false
    }
  }

  send(message: Message): void {
    if (this.#closed) throw new TransportError('WebRTC connection is closed')
    if (!this.#open) throw new TransportError('WebRTC data channel is not open yet')
    const bytes = toBinary(MessageSchema, message)
    if (bytes.length > MAX_MESSAGE_BYTES) {
      throw new TransportError(`message of ${bytes.length} bytes exceeds ${MAX_MESSAGE_BYTES}`)
    }
    try {
      for (const part of fragment(bytes)) this.#channel.send(part)
    } catch (error) {
      // Fragments already sent leave the peer a partial message it cannot frame past.
      const reason = `WebRTC send failed: ${errorText(error)}`
      this.#fail(reason)
      throw new TransportError(reason)
    }
  }

  close(): void {
    this.#shutDown(null)
  }

  #onChunk(data: unknown): void {
    if (!(data instanceof ArrayBuffer)) {
      this.#fail('WebRTC peer sent a non-binary message')
      return
    }
    let message: Uint8Array | null
    try {
      message = this.#reassembler.push(new Uint8Array(data))
    } catch (error) {
      this.#fail(`bad fragment from WebRTC peer: ${errorText(error)}`)
      return
    }
    if (!message) return
    this.#queuedFrames += 1
    this.#queuedBytes += message.byteLength
    if (this.#queuedFrames > MAX_QUEUED_FRAMES || this.#queuedBytes > MAX_QUEUED_BYTES) {
      this.#fail(
        `WebRTC backlog of ${this.#queuedFrames} messages / ${this.#queuedBytes} bytes exceeds ` +
          `${MAX_QUEUED_FRAMES} / ${MAX_QUEUED_BYTES}`,
      )
      return
    }
    this.#frames.push(message)
  }

  #fail(reason: string): void {
    this.#shutDown(new TransportError(reason))
  }

  /** `error` null is our own close: the message stream just ends. */
  #shutDown(error: TransportError | null): void {
    if (this.#closed) return
    this.#closed = true
    this.#channel.onopen = null
    this.#channel.onmessage = null
    this.#channel.onclose = null
    this.#channel.onerror = null
    this.#pc.onconnectionstatechange = null
    this.#pc.onicecandidate = null
    this.#channel.close()
    this.#pc.close()
    this.#settleOpened?.reject(error ?? new TransportError('WebRTC connection closed'))
    this.#settleOpened = null
    this.#candidates.end()
    this.#frames.end(error ?? undefined)
  }

  async *#decode(): AsyncGenerator<Message, void, undefined> {
    for await (const frame of this.#frames) {
      this.#queuedFrames -= 1
      this.#queuedBytes -= frame.byteLength
      let message: Message
      try {
        message = fromBinary(MessageSchema, frame)
      } catch (error) {
        const reason = `bad message from WebRTC peer: ${errorText(error)}`
        this.#fail(reason)
        throw new TransportError(reason)
      }
      yield message
    }
  }
}

/**
 * The peer's candidate text is `RTCIceCandidate` JSON from its ICE agent, whose `sdpMid` may be
 * "" and `usernameFragment` null; both are accepted as they are.
 */
function candidateInit(text: string): RTCIceCandidateInit | null {
  let value: unknown
  try {
    value = JSON.parse(text)
  } catch {
    return null
  }
  if (typeof value !== 'object' || value === null) return null
  const { candidate, sdpMid, sdpMLineIndex, usernameFragment } = value as Record<string, unknown>
  if (typeof candidate !== 'string') return null
  return {
    candidate,
    sdpMid: typeof sdpMid === 'string' ? sdpMid : null,
    sdpMLineIndex: typeof sdpMLineIndex === 'number' ? sdpMLineIndex : null,
    usernameFragment: typeof usernameFragment === 'string' ? usernameFragment : null,
  }
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
