import { clipboardMessage, clipboardText } from '../clipboard/clipboard'
import { CursorTracker, type CursorView } from '../cursor/cursor'
import type { Handshake } from '../handshake/handshake'
import type { LoginState } from '../handshake/login'
import { encodeInput, type RemoteInput } from '../input/input'
import { KeyboardEncoder, keyboardModeFor, supportedKeyboardModes } from '../input/keyboard'
import {
  PermissionInfo_Permission,
  type KeyboardMode,
  type Message,
  type Misc,
  type PeerInfo,
} from '../proto/gen/message_pb.js'
import { qualityOption, ungoverned } from '../settings/protocol'
import type { Quality } from '../settings/settings'
import { createWebCodecsDecoder, type CreateDecoder } from '../video/decoder'
import { DEFAULT_FPS, FpsGovernor } from '../video/fps-governor'
import { VideoPipeline, type VideoMeter } from '../video/pipeline'
import { QualityGovernor } from '../video/quality-governor'
import { FrameQueue } from './frame-queue'
import {
  captureDisplay,
  closeReason,
  ctrlAltDel,
  customFps,
  keyMessage,
  optionMessage,
  refreshVideo,
  selectedSid,
  switchDisplay,
  type OptionPatch,
} from './messages'
import { peerView, withDisplays, withSwitch, type PeerView } from './peer-view'
import { realClock, SessionMeter, type SessionStats, type StatsClock } from './stats'
import { ValueStore, type Store } from './store'
import { windowsSessionChoice, type WindowsSessionChoice } from './windows-sessions'

export type SessionState =
  | Exclude<LoginState, { kind: 'active' } | { kind: 'failed' }>
  /** Logged in to a peer that waits for us to pick one of its Windows sessions. */
  | ({ kind: 'choose_session' } & WindowsSessionChoice)
  | { kind: 'active' }
  | { kind: 'closed'; reason: string }

/** What the peer's user lets us do; the peer reports only changes, so both start allowed. */
export interface Permissions {
  keyboard: boolean
  clipboard: boolean
}

export interface Session {
  readonly state: Store<SessionState>
  readonly peer: Store<PeerView | null>
  readonly cursor: Store<CursorView>
  readonly permissions: Store<Permissions>
  /** The text the peer last copied; null until it copies some. */
  readonly remoteClipboard: Store<string | null>
  readonly stats: Store<SessionStats>
  /**
   * Decoded frames of the peer's screen; the consumer closes each one. Ends on close. Draw with
   * drawImage at displayWidth × displayHeight: codedWidth carries padding and the pixel format
   * differs between browsers.
   */
  readonly frames: AsyncIterable<VideoFrame>
  login(password: string): Promise<void>
  /** Only while the state is need_2fa; ignored otherwise. */
  submit2fa(code: string): void
  /** Only while the state is choose_session, with one of its sids; ignored otherwise. */
  chooseWindowsSession(sid: number): void
  /** One of supportedKeyboardModes for the peer; keys held in the old mode go up first. */
  setKeyboardMode(mode: KeyboardMode): void
  /** Asks the peer to stream display `index` of PeerView.displays. */
  switchDisplay(index: number): void
  ctrlAltDel(): void
  setOptions(patch: OptionPatch): void
  /** Keyboard and mouse; dropped unless active and the peer allows keyboard (and mouse) input. */
  input(input: RemoteInput): void
  /** Sends our clipboard text; false when not active, not allowed or too long. */
  clipboard(text: string): boolean
  close(): void
}

export interface SessionOptions {
  createDecoder?: CreateDecoder
  clock?: StatsClock
}

export const CLOSED_BY_USER = 'closed'
/** What a peer streams only once it serves the connection. */
const SERVED = new Set<Message['union']['case']>([
  'videoFrame',
  'cursorData',
  'cursorId',
  'cursorPosition',
])
/** The peer dropped the link to restart in the Windows session we picked. */
export const WINDOWS_SESSION_SWITCH = 'windows_session_switch'

/** Runs a remote control session over a handshake from `startHandshake`, from login on. */
export function startSession(handshake: Handshake, options: SessionOptions = {}): Session {
  const session = new RemoteSession(
    handshake,
    options.createDecoder ?? createWebCodecsDecoder,
    options.clock ?? realClock,
  )
  void session.run()
  return session
}

class RemoteSession implements Session {
  readonly state = new ValueStore<SessionState>({ kind: 'connecting' })
  readonly peer = new ValueStore<PeerView | null>(null)
  readonly cursor = new ValueStore<CursorView>({ shape: null, position: null })
  readonly permissions = new ValueStore<Permissions>({ keyboard: true, clipboard: true })
  readonly remoteClipboard = new ValueStore<string | null>(null)
  readonly #frames = new FrameQueue()
  readonly #handshake: Handshake
  readonly #video: VideoPipeline
  readonly #meter: SessionMeter
  readonly #governor: FpsGovernor
  readonly #quality = new QualityGovernor()
  readonly #cursors = new CursorTracker()
  #keyboard: KeyboardEncoder | null = null
  #switchingSession = false

  constructor(handshake: Handshake, createDecoder: CreateDecoder, clock: StatsClock) {
    this.#handshake = handshake
    this.#meter = new SessionMeter(this.state, clock, () => ({
      decodeQueue: this.#video.decodeQueueSize,
      codec: this.#video.codec,
      width: this.peer.get()?.display?.width ?? 0,
      height: this.peer.get()?.display?.height ?? 0,
      quality: this.#quality.current,
    }))
    const governor = new FpsGovernor({
      direct: handshake.link === 'webrtc',
      userFps: DEFAULT_FPS,
    })
    this.#governor = governor
    this.#video = new VideoPipeline({
      createDecoder,
      onFrame: (frame) => {
        this.#frames.push(frame)
      },
      requestKeyFrame: () => {
        this.#meter.keyRequested()
        const peer = this.peer.get()
        this.#send(refreshVideo(peer?.current ?? 0, peer?.info.version ?? ''))
      },
      mayRefresh: () => governor.mayRefresh(clock.now()),
      onFatal: (error) => {
        this.#hangUp(`video decoding failed: ${error.message}`)
      },
      meter: governed(this.#meter, governor),
    })
    this.#meter.stats.subscribe(() => {
      this.#steer()
    })
  }

  get stats(): Store<SessionStats> {
    return this.#meter.stats
  }

  get frames(): AsyncIterable<VideoFrame> {
    return this.#frames
  }

  async run(): Promise<void> {
    for await (const event of this.#handshake.events) {
      try {
        if (event.kind === 'state') this.#onLoginState(event.state)
        else if (event.kind === 'message') this.#onMessage(event.message)
        else this.#finish(event.reason)
      } catch (error) {
        this.#hangUp(error instanceof Error ? error.message : String(error))
      }
    }
  }

  login(password: string): Promise<void> {
    return this.#handshake.login(password)
  }

  submit2fa(code: string): void {
    if (this.state.get().kind !== 'need_2fa') return
    this.#handshake.submit2fa(code)
  }

  chooseWindowsSession(sid: number): void {
    const state = this.state.get()
    if (state.kind !== 'choose_session' || !state.sessions.some((s) => s.sid === sid)) return
    this.#send(selectedSid(sid))
    if (this.state.get().kind !== 'choose_session') return
    // Another session: the peer restarts there and drops us, unless it no longer can (another
    // client joined, the session went away); then it serves this connection as it is.
    this.#switchingSession = sid !== state.current
    this.state.set(this.#switchingSession ? { kind: 'connecting' } : { kind: 'active' })
  }

  setKeyboardMode(mode: KeyboardMode): void {
    const peer = this.peer.get()
    if (!this.#keyboard || !peer || !this.#active()) return
    if (!supportedKeyboardModes(peer.info.platform, peer.info.version).includes(mode)) return
    for (const event of this.#keyboard.releaseAll()) this.#send(keyMessage(event))
    this.#keyboard = new KeyboardEncoder(mode, peer.info.platform)
    this.peer.set({ ...peer, keyboardMode: mode })
  }

  // rustdesk 1.5.0 src/ui_session_interface.rs:857-877: switch, then capture only that display.
  switchDisplay(index: number): void {
    const peer = this.peer.get()
    if (!peer || !this.#active() || !Number.isInteger(index)) return
    if (index < 0 || index >= peer.displays.length || index === peer.current) return
    this.#send(switchDisplay(index))
    this.#send(captureDisplay(index))
  }

  ctrlAltDel(): void {
    const peer = this.peer.get()
    if (!peer || !this.#active() || !this.permissions.get().keyboard) return
    this.#send(keyMessage(ctrlAltDel(peer.info.platform)))
  }

  setOptions(patch: OptionPatch): void {
    if (!this.#active()) return
    if (patch.peer) this.#send(optionMessage(ungoverned(patch.peer)))
    if (patch.maxFps !== undefined) this.#governor.setUserFps(patch.maxFps)
    if (patch.quality) this.#sendQuality(this.#quality.setCap(patch.quality))
  }

  input(input: RemoteInput): void {
    const peer = this.peer.get()
    if (!this.#keyboard || !peer || !this.#active() || !this.permissions.get().keyboard) return
    const context = {
      keyboard: this.#keyboard,
      display: peer.display,
      platform: peer.info.platform,
    }
    for (const message of encodeInput(input, context)) this.#send(message)
    if (input.kind === 'pointer' && input.action === 'move') this.#meter.pointerMoved()
  }

  clipboard(text: string): boolean {
    if (!this.#active() || !this.permissions.get().clipboard) return false
    const message = clipboardMessage(text)
    if (!message) return false
    this.#send(message)
    return true
  }

  close(): void {
    if (this.#active()) this.#send(closeReason())
    this.#hangUp(CLOSED_BY_USER)
  }

  #active(): boolean {
    return this.state.get().kind === 'active'
  }

  #steer(): void {
    const fps = this.#governor.tick({ fps: this.stats.get().fps, queued: this.#video.queued })
    if (fps !== null) this.#send(customFps(fps))
    this.#sendQuality(this.#quality.tick(this.#meter.takeDelay()))
  }

  #sendQuality(quality: Quality | null): void {
    if (quality) this.#send(optionMessage(qualityOption(quality)))
  }

  #onLoginState(state: LoginState): void {
    if (state.kind === 'failed') {
      this.#finish(state.reason)
      return
    }
    if (state.kind !== 'active') {
      this.state.set(state)
      return
    }
    this.#onPeerInfo(state.peerInfo)
    const choice = windowsSessionChoice(state.peerInfo)
    this.state.set(choice ? { kind: 'choose_session', ...choice } : { kind: 'active' })
  }

  #onPeerInfo(info: PeerInfo): void {
    const mode = keyboardModeFor(info.platform, info.version)
    this.#keyboard = new KeyboardEncoder(mode, info.platform)
    this.peer.set(peerView(info, mode))
  }

  #onMessage(message: Message): void {
    const { union } = message
    if (this.#switchingSession && SERVED.has(union.case)) {
      this.#switchingSession = false
      this.state.set({ kind: 'active' })
    }
    switch (union.case) {
      case 'videoFrame':
        this.#video.push(union.value)
        return
      case 'cursorData':
        this.cursor.set(this.#cursors.onData(union.value))
        return
      case 'cursorId':
        this.cursor.set(this.#cursors.onId(union.value))
        return
      case 'cursorPosition':
        this.cursor.set(this.#cursors.onPosition(union.value.x, union.value.y))
        return
      case 'clipboard':
      case 'multiClipboards': {
        const text = this.permissions.get().clipboard ? clipboardText(message) : null
        if (text !== null) this.remoteClipboard.set(text)
        return
      }
      case 'peerInfo': {
        const peer = this.peer.get()
        if (peer) this.peer.set(withDisplays(peer, union.value))
        return
      }
      case 'misc':
        this.#onMisc(union.value)
        return
      case 'testDelay':
        if (!union.value.fromClient) this.#meter.delay(union.value.lastDelay)
        return
      default:
        return
    }
  }

  #onMisc(misc: Misc): void {
    const { union } = misc
    switch (union.case) {
      case 'switchDisplay': {
        const peer = this.peer.get()
        if (peer) this.peer.set(withSwitch(peer, union.value))
        this.#video.reset()
        return
      }
      case 'permissionInfo':
        this.#onPermission(union.value.permission, union.value.enabled)
        return
      case 'closeReason':
        this.#hangUp(union.value)
        return
      default:
        return
    }
  }

  // rustdesk 1.5.0 src/client/io_loop.rs:2042-2062. The keyboard permission is the peer's
  // switch for all input: its server drops mouse events too while it is off.
  #onPermission(permission: PermissionInfo_Permission, enabled: boolean): void {
    const current = this.permissions.get()
    if (permission === PermissionInfo_Permission.Keyboard) {
      this.permissions.set({ ...current, keyboard: enabled })
    } else if (permission === PermissionInfo_Permission.Clipboard) {
      this.permissions.set({ ...current, clipboard: enabled })
    }
  }

  /** A link that fails under a send ends the session; callers (UI events, decoder) go on. */
  #send(message: Message): void {
    if (this.state.get().kind === 'closed') return
    try {
      this.#handshake.send(message)
    } catch (error) {
      this.#hangUp(error instanceof Error ? error.message : String(error))
    }
  }

  /** Ends the session from our side and drops the link. */
  #hangUp(reason: string): void {
    this.#finish(reason)
    this.#handshake.close(reason)
  }

  #finish(reason: string): void {
    if (this.state.get().kind === 'closed') return
    const switched = this.#switchingSession && reason !== CLOSED_BY_USER
    this.state.set({ kind: 'closed', reason: switched ? WINDOWS_SESSION_SWITCH : reason })
    this.#video.close()
    this.#frames.end()
  }
}

function governed(meter: VideoMeter, governor: FpsGovernor): VideoMeter {
  return {
    now: () => meter.now(),
    received: (bytes) => {
      meter.received(bytes)
    },
    decoded: (latencyMs, serviceMs) => {
      meter.decoded(latencyMs, serviceMs)
      governor.decoded(serviceMs)
    },
  }
}
