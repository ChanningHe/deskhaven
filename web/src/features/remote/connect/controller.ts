import type { WebClientSession } from '@/api/generated/model'
import { closeWebClientSession, openWebClientSession } from '@/api/generated/web-client/web-client'

import { decodeBase64 } from '../bytes'
import { startHandshake, type HandshakeParams, type Link } from '../handshake/handshake'
import { supportedDecoding, type DecoderSupport } from '../handshake/login'
import { startSession, type Session } from '../session/session'
import { ValueStore, type Store } from '../session/store'
import { RelayTransport, type RelayOptions } from '../transport/relay'
import type { KeyedTransport } from '../transport/transport'
import { probeVideoDecoders } from '../video/codec'

export type ConnectFailure =
  /** POST sessions failed; the error carries the API error code. */
  | { kind: 'api'; error: unknown }
  /** This browser decodes none of the codecs a peer can send. */
  | { kind: 'no_decoder' }
  /** The opened session could not be started (malformed keys or relay URL). */
  | { kind: 'error'; reason: string }

export type RemotePhase =
  /** Not connecting: before the first connect, or after a disconnect while opening. */
  | { kind: 'idle' }
  | { kind: 'opening' }
  /** The session carries its own state from here on, closed included. */
  | { kind: 'session'; session: Session }
  | { kind: 'failed'; failure: ConnectFailure }

export interface ConnectDeps {
  openSession(deviceId: string): Promise<WebClientSession>
  closeSession(sessionId: string): Promise<void>
  probeDecoders(): Promise<DecoderSupport>
  createRelay(options: RelayOptions): KeyedTransport
  startSession(link: Link, params: HandshakeParams): Session
  location: Pick<Location, 'protocol' | 'host'>
}

export const browserDeps: ConnectDeps = {
  openSession: (deviceId) => openWebClientSession({ device_id: deviceId }),
  // keepalive: the close must outlive the page when it goes away (pagehide).
  closeSession: (sessionId) => closeWebClientSession(sessionId, { keepalive: true }),
  probeDecoders: probeVideoDecoders,
  createRelay: (options) => new RelayTransport(options),
  startSession: (link, params) => startSession(startHandshake(link, params)),
  get location() {
    return window.location
  },
}

/**
 * The relay's WebSocket URL from POST sessions: a bare path is on the console's own origin;
 * anything else must be an absolute ws(s) URL.
 */
export function resolveRelayUrl(relayUrl: string, location: Pick<Location, 'protocol' | 'host'>) {
  if (relayUrl.startsWith('/')) {
    const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:'
    return `${scheme}//${location.host}${relayUrl}`
  }
  const url = new URL(relayUrl)
  if (url.protocol !== 'wss:' && url.protocol !== 'ws:') {
    throw new Error(`relay URL is not a WebSocket URL: ${relayUrl}`)
  }
  return url.href
}

/** One connect: the server session it opened (to close) and the peer session it started. */
interface Attempt {
  serverSession: string | null
  released: boolean
  session: Session | null
}

/**
 * Connects the browser to one device over the relay: opens the server session, starts the
 * handshake and the peer session, and closes the server session again once the peer session
 * ends or the page leaves. Each connect replaces the previous attempt.
 */
export class RemoteController {
  readonly #phase = new ValueStore<RemotePhase>({ kind: 'idle' })
  readonly #deviceId: string
  readonly #deps: ConnectDeps
  #current: Attempt | null = null

  constructor(deviceId: string, deps: ConnectDeps = browserDeps) {
    this.#deviceId = deviceId
    this.#deps = deps
  }

  get phase(): Store<RemotePhase> {
    return this.#phase
  }

  connect(): void {
    this.disconnect()
    const attempt: Attempt = {
      serverSession: null,
      released: false,
      session: null,
    }
    this.#current = attempt
    this.#phase.set({ kind: 'opening' })
    void this.#run(attempt)
  }

  /** Ends the current attempt: hangs up on the peer and closes the server session. */
  disconnect(): void {
    const attempt = this.#current
    if (!attempt) return
    this.#current = null
    attempt.session?.close()
    this.#release(attempt)
    if (this.#phase.get().kind === 'opening') this.#phase.set({ kind: 'idle' })
  }

  async #run(attempt: Attempt): Promise<void> {
    const decoders = await this.#deps.probeDecoders()
    if (this.#stale(attempt)) return
    if (!supportedDecoding(decoders)) {
      this.#fail(attempt, { kind: 'no_decoder' })
      return
    }
    let opened: WebClientSession
    try {
      opened = await this.#deps.openSession(this.#deviceId)
    } catch (error) {
      this.#fail(attempt, { kind: 'api', error })
      return
    }
    attempt.serverSession = opened.session_id
    if (this.#stale(attempt)) {
      this.#release(attempt)
      return
    }
    try {
      attempt.session = this.#start(opened, decoders)
    } catch (error) {
      this.#release(attempt)
      this.#fail(attempt, { kind: 'error', reason: errorText(error) })
      return
    }
    const session = attempt.session
    const stop = session.state.subscribe(() => {
      if (session.state.get().kind !== 'closed') return
      stop()
      this.#release(attempt)
    })
    this.#phase.set({ kind: 'session', session })
  }

  #start(opened: WebClientSession, decoders: DecoderSupport): Session {
    // The target is the device the user picked, never the id the server echoes: the
    // handshake checks both signatures against it.
    const params: HandshakeParams = {
      targetId: this.#deviceId,
      serverPk: decodeBase64(opened.server_pk),
      signedPeerPk: decodeBase64(opened.signed_peer_pk),
      myId: opened.my_id,
      myName: opened.my_name,
      decoders,
    }
    const transport = this.#deps.createRelay({
      url: resolveRelayUrl(opened.relay_url, this.#deps.location),
      licenceKey: opened.server_pk,
      uuid: opened.uuid,
      targetId: this.#deviceId,
    })
    return this.#deps.startSession({ kind: 'relay', transport }, params)
  }

  /** A later connect or a disconnect replaced the attempt. */
  #stale(attempt: Attempt): boolean {
    return attempt !== this.#current
  }

  #fail(attempt: Attempt, failure: ConnectFailure): void {
    if (this.#stale(attempt)) return
    this.#phase.set({ kind: 'failed', failure })
  }

  #release(attempt: Attempt): void {
    const id = attempt.serverSession
    if (id === null || attempt.released) return
    attempt.released = true
    // Best effort: the server drops an unclosed session after its TTL anyway, and the page
    // may be gone by the time this settles.
    this.#deps.closeSession(id).catch(() => undefined)
  }
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
