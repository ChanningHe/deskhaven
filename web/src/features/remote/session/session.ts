import { webCodecsAudio } from '../audio/decoder'
import { AudioPipeline } from '../audio/pipeline'
import { clipboardMessage, clipboardText } from '../clipboard/clipboard'
import { CursorTracker, type CursorImages, type CursorView } from '../cursor/cursor'
import type { CloseBy, CloseCause, Handshake } from '../handshake/handshake'
import type { LoginState } from '../handshake/login'
import { encodeInput, type RemoteInput } from '../input/input'
import type { MouseButton } from '../input/mouse'
import { KeyboardEncoder, keyboardModeFor, supportedKeyboardModes } from '../input/keyboard'
import type { KeyboardMode, Message, Misc, PeerInfo } from '../proto/gen/message_pb.js'
import { qualityOption, ungoverned } from '../settings/protocol'
import type { Quality } from '../settings/settings'
import { TransportError } from '../transport/transport'
import { createWebCodecsDecoder } from '../video/decoder'
import { DEFAULT_FPS, FpsGovernor } from '../video/fps-governor'
import { VideoPipeline } from '../video/pipeline'
import { QualityGovernor } from '../video/quality-governor'
import { FrameQueue } from './frame-queue'
import { governed } from './governed'
import {
  captureDisplay,
  changeResolution,
  closeReason,
  actionKeys,
  customFps,
  keyMessage,
  optionMessage,
  refreshVideo,
  restartRemote,
  selectedSid,
  switchDisplay,
  type OptionPatch,
  type SessionAction,
} from './messages'
import { peerView, withDisplays, withSwitch, type PeerView } from './peer-view'
import { ALL_ALLOWED, withPermission, type Permissions } from './permissions'
import { offersResolution } from './resolution'
import { realClock, SessionMeter, type SessionStats, type StatsClock } from './stats'
import { ValueStore, type Store } from './store'
import type { Session, SessionOptions, SessionState } from './types'
import { windowsSessionChoice } from './windows-sessions'

export type { Session, SessionOptions, SessionState } from './types'

// rustdesk 1.5.0 src/client.rs:110 RESTART_REMOTE_DEVICE_GRACE
export const RESTART_GRACE_MS = 5 * 60_000

/** What a peer streams only once it serves the connection. */
const SERVED = new Set<Message['union']['case']>([
  'videoFrame',
  'cursorData',
  'cursorId',
  'cursorPosition',
])

/** Runs a remote control session over a handshake from `startHandshake`, from login on. */
export function startSession(handshake: Handshake, options: SessionOptions = {}): Session {
  const session = new RemoteSession(handshake, options)
  void session.run()
  return session
}

class RemoteSession implements Session {
  readonly state = new ValueStore<SessionState>({ kind: 'connecting' })
  readonly peer = new ValueStore<PeerView | null>(null)
  readonly cursor = new ValueStore<CursorView>({ shape: null, position: null })
  readonly permissions = new ValueStore<Permissions>(ALL_ALLOWED)
  readonly remoteClipboard = new ValueStore<string | null>(null)
  readonly audio: AudioPipeline
  readonly #frames = new FrameQueue()
  readonly #handshake: Handshake
  readonly #video: VideoPipeline
  readonly #meter: SessionMeter
  readonly #governor: FpsGovernor
  readonly #quality = new QualityGovernor()
  readonly #cursors = new CursorTracker()
  readonly #clock: StatsClock
  #keyboard: KeyboardEncoder | null = null
  #switchingSession = false
  #viewOnly: boolean
  readonly #held = new Set<MouseButton>()
  #pointer: Extract<RemoteInput, { kind: 'pointer' }> | null = null
  #restartedAt: number | null = null

  constructor(handshake: Handshake, options: SessionOptions) {
    const clock = options.clock ?? realClock
    this.#handshake = handshake
    this.#clock = clock
    this.#viewOnly = options.viewOnly ?? false
    this.audio = new AudioPipeline(options.audioDecoders ?? webCodecsAudio, () => clock.now())
    this.#meter = new SessionMeter(this.state, clock, () => ({
      decodeQueue: this.#video.decodeQueueSize,
      codec: this.#video.codec,
      width: this.peer.get()?.display?.width ?? 0,
      height: this.peer.get()?.display?.height ?? 0,
      quality: this.#quality.current,
    }))
    const governor = new FpsGovernor({
      direct: handshake.link === 'webrtc',
      userFps: options.maxFps ?? DEFAULT_FPS,
    })
    this.#governor = governor
    this.#video = new VideoPipeline({
      createDecoder: options.createDecoder ?? createWebCodecsDecoder,
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
        this.#hangUp('local', `video decoding failed: ${error.message}`)
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

  get cursorImages(): CursorImages {
    return this.#cursors
  }

  async run(): Promise<void> {
    for await (const event of this.#handshake.events) {
      try {
        if (event.kind === 'state') this.#onLoginState(event.state)
        else if (event.kind === 'message') this.#onMessage(event.message)
        else this.#finish(event.cause)
      } catch (error) {
        this.#hangUp('local', error)
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

  changeResolution(width: number, height: number): void {
    const peer = this.peer.get()
    if (!peer || !this.#inputAllowed()) return
    if (!offersResolution(peer, width, height)) return
    this.#send(changeResolution(peer.current, width, height, peer.info.version))
  }

  action(kind: SessionAction): void {
    const peer = this.peer.get()
    if (!peer || !this.#inputAllowed()) return
    for (const event of actionKeys(kind, peer.info.platform)) this.#send(keyMessage(event))
  }

  restart(): void {
    if (!this.#active() || !this.permissions.get().restart) return
    this.#restartedAt = this.#clock.now()
    this.#send(restartRemote())
  }

  setOptions(patch: OptionPatch): void {
    if (!this.#active()) return
    if (patch.viewOnly !== undefined) this.#setViewOnly(patch.viewOnly)
    if (patch.peer) this.#send(optionMessage(ungoverned(patch.peer)))
    if (patch.maxFps !== undefined) this.#governor.setUserFps(patch.maxFps)
    if (patch.quality) this.#sendQuality(this.#quality.setCap(patch.quality))
  }

  input(input: RemoteInput): void {
    if (!this.#inputAllowed()) return
    this.#encode(input)
    if (input.kind !== 'pointer') return
    this.#pointer = input
    if (input.action === 'move') this.#meter.pointerMoved()
    else if (input.action === 'down' && input.button) this.#held.add(input.button)
    else if (input.button) this.#held.delete(input.button)
  }

  clipboard(text: string): boolean {
    if (!this.#active() || this.#viewOnly || !this.permissions.get().clipboard) return false
    const message = clipboardMessage(text)
    if (!message) return false
    this.#send(message)
    return true
  }

  close(): void {
    if (this.#active()) this.#send(closeReason())
    this.#hangUp('user', '')
  }

  #active(): boolean {
    return this.state.get().kind === 'active'
  }

  #inputAllowed(): boolean {
    return this.#active() && !this.#viewOnly && this.permissions.get().keyboard
  }

  #encode(input: RemoteInput): void {
    const peer = this.peer.get()
    if (!this.#keyboard || !peer) return
    const context = {
      keyboard: this.#keyboard,
      display: peer.display,
      platform: peer.info.platform,
    }
    for (const message of encodeInput(input, context)) this.#send(message)
  }

  #setViewOnly(on: boolean): void {
    if (on && !this.#viewOnly) {
      this.#encode({ kind: 'release' })
      if (this.#pointer) {
        for (const button of this.#held) this.#encode({ ...this.#pointer, action: 'up', button })
      }
      this.#held.clear()
    }
    this.#viewOnly = on
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
    // The handshake's `closed` event, right behind it, says who failed.
    if (state.kind !== 'active') {
      if (state.kind !== 'failed') this.state.set(state)
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
    if (this.audio.receive(message)) return
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
        this.permissions.set(
          withPermission(this.permissions.get(), union.value.permission, union.value.enabled),
        )
        return
      case 'closeReason':
        this.#hangUp('peer', union.value)
        return
      default:
        return
    }
  }

  /** A link that fails under a send ends the session; callers (UI events, decoder) go on. */
  #send(message: Message): void {
    if (this.state.get().kind === 'closed') return
    try {
      this.#handshake.send(message)
    } catch (error) {
      this.#hangUp(error instanceof TransportError ? 'link' : 'local', error)
    }
  }

  /** Ends the session from our side and drops the link. */
  #hangUp(by: CloseBy, error: unknown): void {
    const cause = { by, reason: error instanceof Error ? error.message : String(error) }
    this.#finish(cause)
    this.#handshake.close(cause)
  }

  #restarting({ by, reason }: CloseCause): CloseCause | null {
    const asked =
      this.#restartedAt !== null && this.#clock.now() - this.#restartedAt < RESTART_GRACE_MS
    return asked && (by === 'link' || by === 'peer') ? { by: 'restart', reason } : null
  }

  #finish(cause: CloseCause): void {
    if (this.state.get().kind === 'closed') return
    const by = this.#switchingSession && cause.by !== 'user' ? 'session_switch' : cause.by
    this.state.set({
      kind: 'closed',
      cause: this.#restarting(cause) ?? { by, reason: cause.reason },
    })
    this.#video.close()
    this.audio.close()
    this.#frames.end()
    this.#cursors.close()
  }
}
