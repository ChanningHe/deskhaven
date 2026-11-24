import type { IceServerView, WebClientSession } from '@/api/generated/model'
import {
  addWebClientCandidate,
  closeWebClientSession,
  openWebClientSession,
  pollWebClientCandidates,
} from '@/api/generated/web-client/web-client'

import { browserAudio, type AudioDeps } from '../audio/speaker'
import { decodeBase64 } from '../bytes'
import {
  IDENTITY_TIMEOUT_MS,
  startHandshake,
  type CloseCause,
  type Credential,
  type CredentialSlot,
  type HandshakeParams,
  type Link,
} from '../handshake/handshake'
import { supportedDecoding, type DecoderSupport } from '../handshake/login'
import {
  NO_INTENT,
  intentPatch,
  loginOption,
  loginSettings,
  optionPatch,
  type SessionIntent,
} from '../settings/protocol'
import {
  browserStorage,
  loadSettings,
  saveSettings,
  withPatch,
  type RemoteSettings,
} from '../settings/settings'
import { startSession, type Session, type SessionState } from '../session/session'
import { ValueStore, type Store } from '../session/store'
import { RelayTransport, type RelayOptions } from '../transport/relay'
import type { KeyedTransport } from '../transport/transport'
import { startTrickle, type Trickle, type TrickleApi } from '../transport/trickle'
import { WebRtcTransport, type WebRtcOfferer } from '../transport/webrtc'
import { probeVideoDecoders } from '../video/codec'
import { WEBRTC_CONNECT_TIMEOUT_MS, paired, preferWebRtc, withTimeout } from './race'
import {
  RECENT_MARGIN_MS,
  RECENT_MS,
  STEADY_MS,
  reconnects,
  retriesFailure,
  retryDelay,
} from './reconnect'

export type ConnectFailure =
  /** POST sessions failed; the error carries the API error code. */
  | { kind: 'api'; error: unknown }
  /** This browser decodes none of the codecs a peer can send. */
  | { kind: 'no_decoder' }
  /** The opened session could not be started (malformed keys or relay URL, no link at all). */
  | { kind: 'error'; reason: string }

export type RemotePhase =
  /** Not connecting: before the first connect, or after a disconnect while opening. */
  | { kind: 'idle' }
  | { kind: 'opening' }
  | { kind: 'reconnecting'; attempt: number; at: number }
  /** The session carries its own state from here on, closed included. */
  | { kind: 'session'; session: Session }
  | { kind: 'failed'; failure: ConnectFailure }

/** How the session reaches the peer; `connecting` until the race between the two is decided. */
export type TransportKind = 'connecting' | 'relay' | 'webrtc'

export interface WebRtcDeps extends TrickleApi {
  /** Rejects where WebRTC is unavailable or the offer cannot be sent: the relay goes alone. */
  createOfferer(): Promise<WebRtcOfferer>
}

export interface PageVisibility {
  readonly hidden: boolean
  onVisible(listener: () => void): () => void
}

export interface ConnectDeps {
  /** `offer` asks the device for a WebRTC answer alongside the relay. */
  openSession(deviceId: string, offer?: string): Promise<WebClientSession>
  closeSession(sessionId: string): Promise<void>
  probeDecoders(): Promise<DecoderSupport>
  createRelay(options: RelayOptions): KeyedTransport
  startSession(link: Link, params: HandshakeParams, intent: SessionIntent, maxFps: number): Session
  location: Pick<Location, 'protocol' | 'host'>
  /** Without it, the relay only. */
  webrtc?: WebRtcDeps
  storage?: Storage | null
  random?: () => number
  page?: PageVisibility
  audio?: AudioDeps
}

const documentPage: PageVisibility = {
  get hidden() {
    return document.hidden
  },
  onVisible(listener) {
    const onChange = () => {
      if (!document.hidden) listener()
    }
    document.addEventListener('visibilitychange', onChange)
    return () => {
      document.removeEventListener('visibilitychange', onChange)
    }
  },
}

export const browserDeps: ConnectDeps = {
  openSession: (deviceId, offer) =>
    openWebClientSession(
      offer === undefined ? { device_id: deviceId } : { device_id: deviceId, offer },
    ),
  // keepalive: the close must outlive the page when it goes away (pagehide).
  closeSession: (sessionId) => closeWebClientSession(sessionId, { keepalive: true }),
  probeDecoders: probeVideoDecoders,
  createRelay: (options) => new RelayTransport(options),
  startSession: (link, params, intent, maxFps) =>
    startSession(startHandshake(link, params), { viewOnly: intent.viewOnly, maxFps }),
  get location() {
    return window.location
  },
  get storage() {
    return browserStorage()
  },
  webrtc: {
    createOfferer: () => WebRtcTransport.create(),
    postCandidate: (sessionId, candidate) => addWebClientCandidate(sessionId, { candidate }),
    pollCandidates: (sessionId, after, signal) =>
      pollWebClientCandidates(sessionId, { after }, { signal }),
  },
  page: documentPage,
  audio: browserAudio,
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
  /** Closes what the attempt holds until a peer session owns the link. */
  readonly pending: Set<() => void>
  told: RemoteSettings | null
  toldIntent: SessionIntent | null
  readonly retry: number
  heardAt: number | null
}

/**
 * Connects the browser to one device: opens the server session (with a WebRTC offer where the
 * browser can make one and the settings do not ask for the relay), takes the data channel or the
 * relay, starts the handshake and the peer session on it, and closes the server session again
 * once the peer session ends or the page leaves. Each connect replaces the previous attempt.
 * After a handshake fails over WebRTC, connects go by the relay alone until a handshake over it
 * succeeds: such a failure (a fingerprint mismatch, a peer that drops the channel) tends to
 * repeat.
 */
export class RemoteController {
  readonly #phase = new ValueStore<RemotePhase>({ kind: 'idle' })
  readonly #transport = new ValueStore<TransportKind>('connecting')
  readonly #settings: ValueStore<RemoteSettings>
  readonly #intent = new ValueStore<SessionIntent>(NO_INTENT)
  readonly #storage: Storage | null
  readonly #deviceId: string
  readonly #deps: ConnectDeps
  #current: Attempt | null = null
  #relayOnly = false
  #sessionId = randomSessionId()
  #credential: Credential | null = null
  #armed = false
  #attempts = 0
  #lostAt: number | null = null
  #cancelRetry: (() => void) | null = null
  #cancelSteady: (() => void) | null = null
  #audioRunning = false

  constructor(deviceId: string, deps: ConnectDeps = browserDeps) {
    this.#deviceId = deviceId
    this.#deps = deps
    this.#storage = deps.storage === undefined ? browserStorage() : deps.storage
    this.#settings = new ValueStore(loadSettings(this.#storage))
  }

  get settings(): Store<RemoteSettings> {
    return this.#settings
  }

  updateSettings(patch: Partial<RemoteSettings>): void {
    const prev = this.#settings.get()
    const next = withPatch(prev, patch)
    this.#settings.set(next)
    saveSettings(next, this.#storage)
    if (next.transport !== prev.transport && this.#live()) this.#relink()
    else if (this.#current) this.#sync(this.#current)
  }

  get intent(): Store<SessionIntent> {
    return this.#intent
  }

  updateIntent(patch: Partial<SessionIntent>): void {
    this.#intent.set({ ...this.#intent.get(), ...patch })
    if (this.#current) this.#sync(this.#current)
  }

  setAudioRunning(running: boolean): void {
    if (running === this.#audioRunning) return
    this.#audioRunning = running
    if (this.#current) this.#sync(this.#current)
  }

  get phase(): Store<RemotePhase> {
    return this.#phase
  }

  /** The current attempt's transport. */
  get transport(): Store<TransportKind> {
    return this.#transport
  }

  hasCredential(): boolean {
    return this.#credential !== null
  }

  connect(): void {
    this.disconnect()
    this.#sessionId = randomSessionId()
    this.#start(0)
  }

  disconnect(): void {
    this.#manual()
    this.#drop()
    const { kind } = this.#phase.get()
    if (kind === 'opening' || kind === 'reconnecting') this.#phase.set({ kind: 'idle' })
  }

  #drop(): void {
    const attempt = this.#current
    this.#current = null
    if (!attempt) return
    for (const close of attempt.pending) close()
    attempt.pending.clear()
    attempt.session?.close()
    this.#release(attempt)
  }

  /** Connecting, reconnecting, or in a session that has not ended. */
  #live(): boolean {
    const phase = this.#phase.get()
    if (phase.kind === 'session') return phase.session.state.get().kind !== 'closed'
    return phase.kind === 'opening' || phase.kind === 'reconnecting'
  }

  /**
   * Starts over on the link the settings now ask for. Once logged in, it goes on like a
   * reconnect (same peer session, the kept credential within its window) but at once: the user
   * asked for it, so it neither waits nor counts as an attempt.
   */
  #relink(): void {
    if (!this.#armed) {
      this.connect()
      return
    }
    this.#lostAt ??= this.#current?.heardAt ?? null
    this.#cancelRetry?.()
    this.#cancelRetry = null
    this.#drop()
    this.#start(0)
  }

  #start(retry: number): void {
    const attempt: Attempt = {
      serverSession: null,
      released: false,
      session: null,
      pending: new Set(),
      told: null,
      toldIntent: null,
      heardAt: null,
      retry,
    }
    this.#current = attempt
    this.#transport.set('connecting')
    if (retry === 0) this.#phase.set({ kind: 'opening' })
    void this.#run(attempt)
  }

  async #run(attempt: Attempt): Promise<void> {
    const decoders = await this.#deps.probeDecoders()
    if (this.#stale(attempt)) return
    if (!supportedDecoding(decoders)) {
      this.#fail(attempt, { kind: 'no_decoder' })
      return
    }
    const relayOnly = this.#relayOnly || this.#settings.get().transport === 'relay'
    const offerer = relayOnly ? null : await this.#offerer()
    if (this.#stale(attempt)) {
      offerer?.close()
      return
    }
    if (offerer) {
      attempt.pending.add(() => {
        offerer.close()
      })
    }
    let opened: WebClientSession
    try {
      opened = await (offerer
        ? this.#deps.openSession(this.#deviceId, offerer.offer)
        : this.#deps.openSession(this.#deviceId))
    } catch (error) {
      offerer?.close()
      this.#fail(attempt, { kind: 'api', error })
      return
    }
    attempt.serverSession = opened.session_id
    if (this.#stale(attempt)) {
      this.#release(attempt)
      return
    }
    let params: HandshakeParams
    let link: Link
    try {
      const settings = this.#settings.get()
      params = this.#params(attempt, opened, decoders, settings)
      attempt.told = loginSettings(settings)
      attempt.toldIntent = this.#intent.get()
      const relay = this.#deps.createRelay(this.#relayOptions(opened))
      const webrtc = this.#deps.webrtc
      if (offerer && opened.answer && webrtc) {
        link = await this.#race(attempt, opened, opened.answer, offerer, relay, webrtc)
      } else {
        offerer?.close()
        link = { kind: 'relay', transport: relay }
      }
    } catch (error) {
      offerer?.close()
      if (this.#stale(attempt)) return
      this.#release(attempt)
      this.#fail(attempt, { kind: 'error', reason: errorText(error) })
      return
    }
    attempt.pending.clear()
    if (this.#stale(attempt)) {
      link.transport.close()
      return
    }
    this.#transport.set(link.kind)
    const session = this.#deps.startSession(link, params, attempt.toldIntent, attempt.told.fps)
    attempt.session = session
    let handshaken = false
    const unwatch = [session.permissions, session.audio.state].map((store) =>
      store.subscribe(() => {
        this.#sync(attempt)
      }),
    )
    const stop = session.state.subscribe(() => {
      const state = session.state.get()
      if (state.kind !== 'closed') {
        if (state.kind === 'connecting') return
        handshaken = true
        this.#relayOnly = false
        this.#loggedIn(attempt, session, state)
        return
      }
      stop()
      for (const off of unwatch) off()
      if (!handshaken && link.kind === 'webrtc' && state.cause.by !== 'user') {
        this.#relayOnly = true
      }
      this.#release(attempt)
      this.#ended(attempt, session, state.cause)
    })
    if (attempt.retry === 0) this.#phase.set({ kind: 'session', session })
  }

  #loggedIn(attempt: Attempt, session: Session, state: SessionState): void {
    if (this.#stale(attempt)) return
    if (state.kind === 'active' || state.kind === 'choose_session') {
      this.#armed = true
      this.#lostAt = null
      if (state.kind === 'active') this.#steady()
    } else {
      this.#manual()
    }
    if (this.#phase.get().kind === 'reconnecting') this.#phase.set({ kind: 'session', session })
    this.#sync(attempt)
  }

  #ended(attempt: Attempt, session: Session, cause: CloseCause): void {
    if (this.#stale(attempt)) return
    this.#cancelSteady?.()
    this.#cancelSteady = null
    if (this.#armed && reconnects(cause.by)) {
      this.#lostAt ??= attempt.heardAt
      if (this.#retry()) return
    }
    this.#manual()
    if (this.#phase.get().kind === 'reconnecting') this.#phase.set({ kind: 'session', session })
  }

  #retry(): boolean {
    const attempt = this.#attempts + 1
    const delay = retryDelay(attempt, this.#deps.random ?? Math.random)
    if (delay === null) return false
    this.#attempts = attempt
    this.#phase.set({ kind: 'reconnecting', attempt, at: Date.now() + delay })
    this.#cancelRetry = after(delay, () => {
      this.#cancelRetry = null
      this.#whenShown(() => {
        this.#start(attempt)
      })
    })
    return true
  }

  // Nobody watches a hidden page: retaking control of the device waits until it is shown.
  #whenShown(run: () => void): void {
    const page = this.#deps.page
    if (!page?.hidden) {
      run()
      return
    }
    const stop = page.onVisible(() => {
      stop()
      this.#cancelRetry = null
      run()
    })
    this.#cancelRetry = stop
  }

  #steady(): void {
    if (this.#cancelSteady) return
    this.#cancelSteady = after(STEADY_MS, () => {
      this.#cancelSteady = null
      this.#attempts = 0
    })
  }

  #manual(): void {
    this.#armed = false
    this.#attempts = 0
    this.#lostAt = null
    this.#credential = null
    this.#cancelRetry?.()
    this.#cancelRetry = null
    this.#cancelSteady?.()
    this.#cancelSteady = null
  }

  // Once, within 30 s: a stale proof counts toward lockout (connection.rs:1914-1917,4571-4583).
  #credentials(attempt: Attempt): CredentialSlot {
    return {
      take: () => {
        const credential = this.#credential
        this.#credential = null
        const recent =
          this.#lostAt !== null && Date.now() - this.#lostAt < RECENT_MS - RECENT_MARGIN_MS
        return recent && !this.#stale(attempt) ? credential : null
      },
      keep: (credential) => {
        if (!this.#stale(attempt)) this.#credential = credential
      },
    }
  }

  /** Null where the browser cannot offer: the relay goes alone. */
  async #offerer(): Promise<WebRtcOfferer | null> {
    const webrtc = this.#deps.webrtc
    if (!webrtc) return null
    try {
      return await webrtc.createOfferer()
    } catch {
      return null
    }
  }

  #params(
    attempt: Attempt,
    opened: WebClientSession,
    decoders: DecoderSupport,
    settings: RemoteSettings,
  ): HandshakeParams {
    // The target is the device the user picked, never the id the server echoes: the
    // handshake checks both signatures against it.
    return {
      targetId: this.#deviceId,
      serverPk: decodeBase64(opened.server_pk),
      signedPeerPk: decodeBase64(opened.signed_peer_pk),
      myId: opened.my_id,
      myName: opened.my_name,
      sessionId: this.#sessionId,
      decoders,
      loginOption: loginOption(settings, this.#intent.get()),
      credentials: this.#credentials(attempt),
      onHeard: (at) => {
        attempt.heardAt = at
      },
    }
  }

  #sync(attempt: Attempt): void {
    const { session, told, toldIntent } = attempt
    if (!session || !told || !toldIntent || session.state.get().kind !== 'active') return
    const chosen = this.#settings.get()
    // Before a click the AudioContext is suspended: a peer asked for audio would stream it unheard.
    const playable = session.permissions.get().audio && session.audio.state.get() === 'ok'
    const audio = chosen.audio && this.#audioRunning && playable
    const settings = { ...chosen, audio }
    const intent = this.#intent.get()
    const patches = [optionPatch(told, settings), intentPatch(toldIntent, intent)]
    attempt.told = settings
    attempt.toldIntent = intent
    for (const patch of patches) if (patch) session.setOptions(patch)
  }

  #relayOptions(opened: WebClientSession): RelayOptions {
    return {
      url: resolveRelayUrl(opened.relay_url, this.#deps.location),
      licenceKey: opened.server_pk,
      uuid: opened.uuid,
      targetId: this.#deviceId,
    }
  }

  /**
   * Brings up WebRTC and the relay side by side and keeps the one `preferWebRtc` picks, closing
   * the other. Throws when both fail. The handshake then runs once, on the winner.
   */
  async #race(
    attempt: Attempt,
    opened: WebClientSession,
    answer: string,
    offerer: WebRtcOfferer,
    relay: KeyedTransport,
    webrtc: WebRtcDeps,
  ): Promise<Link> {
    const exchange: { trickle: Trickle | null } = { trickle: null }
    attempt.pending.add(() => {
      relay.close()
    })
    attempt.pending.add(() => {
      exchange.trickle?.stop()
    })
    const channel = async () => {
      const fingerprint = await offerer.accept(answer, iceServers(opened.ice_servers))
      if (this.#stale(attempt)) throw new Error('connect replaced')
      exchange.trickle = startTrickle(webrtc, opened.session_id, offerer)
      await offerer.opened
      return fingerprint
    }
    try {
      const result = await preferWebRtc(
        withTimeout(channel(), WEBRTC_CONNECT_TIMEOUT_MS, 'WebRTC connect timeout'),
        withTimeout(paired(relay), IDENTITY_TIMEOUT_MS, 'relay pairing timeout'),
      )
      if (result.winner === 'webrtc') {
        relay.close()
        return { kind: 'webrtc', transport: offerer, remoteFingerprint: result.value }
      }
      offerer.close()
      return { kind: 'relay', transport: result.value }
    } catch (error) {
      relay.close()
      throw error
    } finally {
      // Once a link is chosen, candidates no longer matter (client.rs:1181 stops the bridge).
      exchange.trickle?.stop()
    }
  }

  /** A later connect or a disconnect replaced the attempt. */
  #stale(attempt: Attempt): boolean {
    return attempt !== this.#current
  }

  #fail(attempt: Attempt, failure: ConnectFailure): void {
    if (this.#stale(attempt)) return
    if (this.#armed && retriesFailure(failure) && this.#retry()) return
    this.#manual()
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

function iceServers(views: IceServerView[]): RTCIceServer[] {
  return views.map(({ urls, username, credential }) => ({
    urls,
    ...(username == null ? {} : { username }),
    ...(credential == null ? {} : { credential }),
  }))
}

const SESSION_ID_BYTES = 8

function randomSessionId(): bigint {
  const bytes = crypto.getRandomValues(new Uint8Array(SESSION_ID_BYTES))
  return new DataView(bytes.buffer).getBigUint64(0, true)
}

function after(ms: number, run: () => void): () => void {
  const id = setTimeout(run, ms)
  return () => {
    clearTimeout(id)
  }
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
