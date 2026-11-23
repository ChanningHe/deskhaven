import type { CursorImages, CursorView } from '../cursor/cursor'
import type { CloseCause } from '../handshake/handshake'
import type { LoginState } from '../handshake/login'
import type { RemoteInput } from '../input/input'
import type { KeyboardMode } from '../proto/gen/message_pb.js'
import type { CreateDecoder } from '../video/decoder'
import type { OptionPatch, SessionAction } from './messages'
import type { PeerView } from './peer-view'
import type { Permissions } from './permissions'
import type { SessionStats, StatsClock } from './stats'
import type { Store } from './store'
import type { WindowsSessionChoice } from './windows-sessions'

export type SessionState =
  | Exclude<LoginState, { kind: 'active' } | { kind: 'failed' }>
  /** Logged in to a peer that waits for us to pick one of its Windows sessions. */
  | ({ kind: 'choose_session' } & WindowsSessionChoice)
  | { kind: 'active' }
  | { kind: 'closed'; cause: CloseCause }

export interface Session {
  readonly state: Store<SessionState>
  readonly peer: Store<PeerView | null>
  readonly cursor: Store<CursorView>
  readonly cursorImages: CursorImages
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
  changeResolution(width: number, height: number): void
  action(kind: SessionAction): void
  restart(): void
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
  viewOnly?: boolean
}
