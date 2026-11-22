import { clipboardMessage, clipboardText } from '../clipboard/clipboard'
import { CursorTracker, type CursorView } from '../cursor/cursor'
import type { Handshake } from '../handshake/handshake'
import type { LoginState } from '../handshake/login'
import { encodeInput, type RemoteInput } from '../input/input'
import { KeyboardEncoder, keyboardModeFor } from '../input/keyboard'
import {
  PermissionInfo_Permission,
  type Message,
  type Misc,
  type PeerInfo,
} from '../proto/gen/message_pb.js'
import { createWebCodecsDecoder, type CreateDecoder } from '../video/decoder'
import { VideoPipeline } from '../video/pipeline'
import { FrameQueue } from './frame-queue'
import { closeReason, refreshVideo, videoReceived } from './messages'
import { peerView, withDisplays, withSwitch, type PeerView } from './peer-view'
import { ValueStore, type Store } from './store'

export type SessionState =
  | Exclude<LoginState, { kind: 'active' } | { kind: 'failed' }>
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
  /**
   * Decoded frames of the peer's screen; the consumer closes each one. Ends on close. Draw with
   * drawImage at displayWidth × displayHeight: codedWidth carries padding and the pixel format
   * differs between browsers.
   */
  readonly frames: AsyncIterable<VideoFrame>
  login(password: string): Promise<void>
  submit2fa(code: string): void
  /** Keyboard and mouse; dropped unless active and the peer allows keyboard (and mouse) input. */
  input(input: RemoteInput): void
  /** Sends our clipboard text; false when not active, not allowed or too long. */
  clipboard(text: string): boolean
  close(): void
}

export interface SessionOptions {
  createDecoder?: CreateDecoder
}

export const CLOSED_BY_USER = 'closed'

/** Runs a remote control session over a handshake from `startHandshake`, from login on. */
export function startSession(handshake: Handshake, options: SessionOptions = {}): Session {
  const session = new RemoteSession(handshake, options.createDecoder ?? createWebCodecsDecoder)
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
  readonly #cursors = new CursorTracker()
  #keyboard: KeyboardEncoder | null = null

  constructor(handshake: Handshake, createDecoder: CreateDecoder) {
    this.#handshake = handshake
    this.#video = new VideoPipeline({
      createDecoder,
      onFrame: (frame) => {
        this.#frames.push(frame)
      },
      ack: () => {
        this.#send(videoReceived())
      },
      requestKeyFrame: () => {
        const peer = this.peer.get()
        this.#send(refreshVideo(peer?.current ?? 0, peer?.info.version ?? ''))
      },
      onFatal: (error) => {
        this.#hangUp(`video decoding failed: ${error.message}`)
      },
    })
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
    this.#handshake.submit2fa(code)
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
    this.state.set({ kind: 'active' })
  }

  #onPeerInfo(info: PeerInfo): void {
    const mode = keyboardModeFor(info.platform, info.version)
    this.#keyboard = new KeyboardEncoder(mode, info.platform)
    this.peer.set(peerView(info, mode))
  }

  #onMessage(message: Message): void {
    const { union } = message
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
    this.state.set({ kind: 'closed', reason })
    this.#video.close()
    this.#frames.end()
  }
}
