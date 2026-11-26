import { create, fromBinary } from '@bufbuild/protobuf'

import { AsyncQueue } from '../async-queue'
import { FrameCipher, kxVersionFor } from '../crypto/frame-cipher'
import {
  BOX_KEY_BYTES,
  SECRET_KEY_BYTES,
  boxKeyPairFromSecret,
  openSigned,
  randomBytes,
  sealKey,
} from '../crypto/nacl'
import { proof, salted } from '../crypto/password'
import {
  Auth2FASchema,
  MessageSchema,
  PublicKeySchema,
  type Hash,
  type LoginResponse,
  type Message,
  type OptionMessage,
} from '../proto/gen/message_pb.js'
import { IdPkSchema, type IdPk } from '../proto/gen/rendezvous_pb.js'
import { TransportError, type KeyedTransport, type Transport } from '../transport/transport'
import {
  LOGIN_CODECS,
  loginRequest,
  loginStateForError,
  supportedDecoding,
  type DecoderSupport,
  type LoginFields,
  type LoginState,
} from './login'

export interface Credential {
  readonly salt: string
  readonly salted: Uint8Array
}

export interface CredentialSlot {
  take(): Credential | null
  keep(credential: Credential): void
}

export type CloseBy = 'user' | 'peer' | 'link' | 'local' | 'session_switch' | 'restart'

export interface CloseCause {
  by: CloseBy
  reason: string
}

export interface HandshakeParams {
  targetId: string
  serverPk: Uint8Array
  signedPeerPk: Uint8Array
  /** Who we are to the peer's user; issued by the console server, never typed by the user. */
  myId: string
  myName: string
  /** Reused across reconnects: the peer's recent-session key. */
  sessionId: bigint
  decoders: DecoderSupport
  loginOption: OptionMessage
  credentials?: CredentialSlot
  onHeard?: (at: number) => void
}

export type StartTimer = (ms: number, onExpire: () => void) => () => void

export interface HandshakeDeps {
  random?: (length: number) => Uint8Array
  startTimer?: StartTimer
}

// The client bounds each handshake step at 18 s (CONNECT_TIMEOUT, READ_TIMEOUT).
export const CONNECT_TIMEOUT_MS = 18_000
export const READ_TIMEOUT_MS = 18_000
export const IDENTITY_TIMEOUT_MS = CONNECT_TIMEOUT_MS + READ_TIMEOUT_MS
export const TIMEOUT_REASON = 'timeout'
// The peer sends TestDelay every second: silence this long is a dead link.
export const SILENCE_MS = 8000
export const SILENCE_REASON = 'no message from the device for 8 s'

const realTimer: StartTimer = (ms, onExpire) => {
  const id = setTimeout(onExpire, ms)
  return () => {
    clearTimeout(id)
  }
}

export type Link =
  | { kind: 'relay'; transport: KeyedTransport }
  | { kind: 'webrtc'; transport: Transport; remoteFingerprint: string }

export type LinkKind = Link['kind']

export type HandshakeEvent =
  | { kind: 'state'; state: LoginState }
  | { kind: 'message'; message: Message }
  | { kind: 'closed'; cause: CloseCause }

export interface Handshake {
  readonly link: LinkKind
  readonly events: AsyncIterable<HandshakeEvent>
  /** Before the peer's Hash arrives, the password is kept for it; afterwards it is sent. */
  login(password: string): Promise<void>
  submit2fa(code: string): void
  /** Only once the stream is keyed, never in plaintext. */
  send(message: Message): void
  close(cause: CloseCause): void
}

type Phase = 'identity' | 'auth' | 'active' | 'closed'

/** Every check fails closed: there is no unauthenticated fallback. */
export function startHandshake(
  link: Link,
  params: HandshakeParams,
  deps: HandshakeDeps = {},
): Handshake {
  const handshake = new PeerHandshake(
    link,
    params,
    deps.random ?? randomBytes,
    deps.startTimer ?? realTimer,
  )
  void handshake.run()
  return handshake
}

class PeerHandshake implements Handshake {
  readonly #events = new AsyncQueue<HandshakeEvent>()
  readonly #link: Link
  readonly #params: HandshakeParams
  readonly #random: (length: number) => Uint8Array
  readonly #startTimer: StartTimer
  #phase: Phase = 'identity'
  #state: LoginState = { kind: 'connecting' }
  #password: string | null = null
  #proven: Credential | null = null
  #hash: Hash | null = null
  #fields: LoginFields | null = null
  #devicePk: Uint8Array | null = null
  #cancelTimer: (() => void) | null = null

  constructor(
    link: Link,
    params: HandshakeParams,
    random: (length: number) => Uint8Array,
    startTimer: StartTimer,
  ) {
    this.#link = link
    this.#params = params
    this.#random = random
    this.#startTimer = startTimer
  }

  get link(): LinkKind {
    return this.#link.kind
  }

  get events(): AsyncIterable<HandshakeEvent> {
    return this.#events
  }

  async run(): Promise<void> {
    try {
      this.#start()
      for await (const message of this.#link.transport.messages) {
        if (this.#phase === 'closed') return
        // Wall-clock time, so that a sleeping machine counts against the peer's window too.
        this.#params.onHeard?.(Date.now())
        if (this.#phase === 'active') this.#arm(SILENCE_MS, SILENCE_REASON)
        await this.#onMessage(message)
      }
      this.#close({ by: 'link', reason: 'connection closed' })
    } catch (error) {
      this.#close({
        by: error instanceof TransportError ? 'link' : 'local',
        reason: error instanceof Error ? error.message : String(error),
      })
    }
  }

  async login(password: string): Promise<void> {
    if (this.#phase !== 'identity' && this.#phase !== 'auth') {
      throw new Error('the session is not waiting for a password')
    }
    this.#password = password
    if (this.#hash) await this.#sendPassword(password, this.#hash)
  }

  submit2fa(code: string): void {
    if (this.#phase !== 'auth') throw new Error('the session is not waiting for a 2FA code')
    this.#link.transport.send(
      create(MessageSchema, { union: { case: 'auth2fa', value: create(Auth2FASchema, { code }) } }),
    )
    this.#setState({ kind: 'connecting' })
    this.#arm(READ_TIMEOUT_MS)
  }

  send(message: Message): void {
    if (this.#phase !== 'auth' && this.#phase !== 'active') {
      throw new Error('the session is not keyed')
    }
    this.#link.transport.send(message)
  }

  close(cause: CloseCause): void {
    this.#close(cause)
  }

  #start(): void {
    const params = this.#params
    const decoding = supportedDecoding(params.decoders, LOGIN_CODECS)
    if (!decoding) throw new Error('no supported video decoder')
    const peer = verifyIdPk(params.signedPeerPk, params.serverPk, params.targetId, 'server')
    this.#devicePk = peer.pk
    this.#fields = {
      targetId: params.targetId,
      myId: params.myId,
      myName: params.myName,
      sessionId: params.sessionId,
      decoding,
      option: params.loginOption,
    }
    this.#arm(IDENTITY_TIMEOUT_MS)
  }

  async #onMessage(message: Message): Promise<void> {
    if (this.#phase === 'identity') {
      this.#keyStream(message)
      return
    }
    const { union } = message
    if (union.case === 'testDelay' && !union.value.fromClient) this.#link.transport.send(message)
    if (this.#phase === 'auth') {
      if (union.case === 'hash') {
        await this.#onHash(union.value)
        return
      }
      if (union.case === 'loginResponse' && this.#onLoginResponse(union.value)) return
    }
    this.#events.push({ kind: 'message', message })
  }

  #keyStream(message: Message): void {
    if (message.union.case !== 'signedId') {
      throw new Error(`expected the peer's signed identity, got ${message.union.case ?? 'nothing'}`)
    }
    const devicePk = this.#devicePk ?? new Uint8Array()
    const peer = verifyIdPk(message.union.value.id, devicePk, this.#params.targetId, 'peer')
    const link = this.#link
    if (link.kind === 'relay' && peer.dtlsFingerprint !== '') {
      throw new Error('peer signed a DTLS fingerprint on a relay link')
    }
    if (
      link.kind === 'webrtc' &&
      (peer.dtlsFingerprint === '' || peer.dtlsFingerprint !== link.remoteFingerprint)
    ) {
      throw new Error('peer DTLS fingerprint is not the one of this channel')
    }
    // WebRTC is keyed by DTLS and takes no stream key of its own, so it always runs version 0.
    const picked = link.kind === 'relay' ? kxVersionFor(peer.kxVersion) : 0
    const ours = boxKeyPairFromSecret(this.#random(BOX_KEY_BYTES))
    const key = this.#random(SECRET_KEY_BYTES)
    link.transport.send(
      create(MessageSchema, {
        union: {
          case: 'publicKey',
          value: create(PublicKeySchema, {
            asymmetricValue: ours.publicKey,
            symmetricValue: sealKey(key, peer.pk, ours.secretKey),
            kxVersion: picked,
          }),
        },
      }),
    )
    if (link.kind === 'relay') {
      link.transport.startEncryption(
        picked === 0
          ? FrameCipher.v0(key)
          : FrameCipher.v1(
              key,
              {
                initiatorPk: ours.publicKey,
                responderPk: peer.pk,
                advertised: peer.kxVersion,
                picked,
              },
              'initiator',
            ),
      )
    }
    this.#phase = 'auth'
    this.#arm(READ_TIMEOUT_MS)
  }

  async #onHash(hash: Hash): Promise<void> {
    this.#hash = hash
    if (this.#password !== null) {
      await this.#sendPassword(this.#password, hash)
      return
    }
    const kept = this.#params.credentials?.take() ?? null
    if (kept?.salt === hash.salt) {
      await this.#sendProof(kept, hash)
      return
    }
    this.#link.transport.send(loginRequest(this.#loginFields(), new Uint8Array()))
    this.#awaitUser({ kind: 'need_password' })
  }

  async #sendPassword(password: string, hash: Hash): Promise<void> {
    await this.#sendProof({ salt: hash.salt, salted: await salted(password, hash.salt) }, hash)
  }

  async #sendProof(credential: Credential, hash: Hash): Promise<void> {
    const login = await proof(credential.salted, hash.challenge)
    if (this.#phase !== 'auth') return
    this.#proven = credential
    this.#link.transport.send(loginRequest(this.#loginFields(), login))
    this.#setState({ kind: 'connecting' })
    this.#arm(READ_TIMEOUT_MS)
  }

  #onLoginResponse(response: LoginResponse): boolean {
    const { union } = response
    if (union.case === 'peerInfo') {
      const proven = this.#proven
      this.#phase = 'active'
      this.#arm(SILENCE_MS, SILENCE_REASON)
      this.#password = null
      this.#proven = null
      if (proven) this.#params.credentials?.keep(proven)
      this.#setState({ kind: 'active', peerInfo: union.value })
      return true
    }
    if (union.case === 'error') {
      const state = loginStateForError(union.value)
      if (state.kind === 'wrong_password') this.#password = null
      // The peer asks for a 2FA code only past the password.
      if (state.kind !== 'need_2fa') this.#proven = null
      if (state.kind === 'failed') this.#close({ by: 'peer', reason: state.reason })
      else this.#awaitUser(state)
      return true
    }
    return false
  }

  #loginFields(): LoginFields {
    if (!this.#fields) throw new Error('handshake not started')
    return this.#fields
  }

  /** The peer now waits on a person (password, approval, 2FA code), so no step timer runs. */
  #awaitUser(state: LoginState): void {
    this.#disarm()
    this.#setState(state)
  }

  #arm(ms: number, reason = TIMEOUT_REASON): void {
    this.#disarm()
    this.#cancelTimer = this.#startTimer(ms, () => {
      this.#close({ by: 'link', reason })
    })
  }

  #disarm(): void {
    this.#cancelTimer?.()
    this.#cancelTimer = null
  }

  #setState(state: LoginState): void {
    this.#state = state
    this.#events.push({ kind: 'state', state })
  }

  #close(cause: CloseCause): void {
    if (this.#phase === 'closed') return
    const wasActive = this.#phase === 'active'
    this.#phase = 'closed'
    this.#disarm()
    this.#password = null
    this.#proven = null
    this.#link.transport.close()
    if (!wasActive && this.#state.kind !== 'failed') {
      this.#setState({ kind: 'failed', reason: cause.reason })
    }
    this.#events.push({ kind: 'closed', cause })
    this.#events.end()
  }
}

function verifyIdPk(
  blob: Uint8Array,
  signerPk: Uint8Array,
  expectedId: string,
  signer: 'server' | 'peer',
): IdPk {
  const opened = openSigned(blob, signerPk)
  if (!opened) throw new Error(`${signer} signature on the peer identity does not verify`)
  let idPk: IdPk
  try {
    idPk = fromBinary(IdPkSchema, opened)
  } catch {
    throw new Error(`${signer}-signed peer identity is malformed`)
  }
  if (idPk.id !== expectedId) throw new Error(`${signer}-signed identity names another device`)
  if (idPk.pk.length !== BOX_KEY_BYTES) throw new Error(`${signer}-signed key has the wrong length`)
  return idPk
}
