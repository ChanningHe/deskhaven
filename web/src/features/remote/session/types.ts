import type { AudioDecoders } from '../audio/decoder'
import type { AudioPlayback } from '../audio/pipeline'
import type { CursorImages, CursorView } from '../cursor/cursor'
import type { CloseCause } from '../handshake/handshake'
import type { DecoderSupport, LoginState } from '../handshake/login'
import type { RemoteInput } from '../input/input'
import type { KeyboardMode } from '../proto/gen/message_pb.js'
import type { CodecView } from '../video/codec'
import type { CreateDecoder } from '../video/decoder'
import type { SampleProgress, VideoSample } from '../video/sample-export'
import type { OptionPatch, SessionAction } from './messages'
import type { PeerView } from './peer-view'
import type { Permissions } from './permissions'
import type { SessionStats, StatsClock } from './stats'
import type { Store } from './store'
import type { WindowsSessionChoice } from './windows-sessions'

export type SessionState =
  | Exclude<LoginState, { kind: 'active' } | { kind: 'failed' }>
  | ({ kind: 'choose_session' } & WindowsSessionChoice)
  | { kind: 'active' }
  | { kind: 'closed'; cause: CloseCause }

export interface Session {
  readonly state: Store<SessionState>
  readonly peer: Store<PeerView | null>
  readonly cursor: Store<CursorView>
  readonly cursorImages: CursorImages
  readonly permissions: Store<Permissions>
  readonly remoteClipboard: Store<string | null>
  readonly stats: Store<SessionStats>
  readonly codecs: Store<CodecView>
  readonly sampling: Store<SampleProgress>
  readonly audio: AudioPlayback
  /**
   * The consumer closes each frame. Draw at displayWidth × displayHeight: codedWidth carries
   * padding and the pixel format differs between browsers.
   */
  readonly frames: AsyncIterable<VideoFrame>
  login(password: string): Promise<void>
  /** Only while the state is need_2fa; ignored otherwise. */
  submit2fa(code: string): void
  /** Only while the state is choose_session, with one of its sids; ignored otherwise. */
  chooseWindowsSession(sid: number): void
  /** One of supportedKeyboardModes for the peer; keys held in the old mode go up first. */
  setKeyboardMode(mode: KeyboardMode): void
  switchDisplay(index: number): void
  changeResolution(width: number, height: number): void
  action(kind: SessionAction): void
  restart(): void
  setOptions(patch: OptionPatch): void
  /** Keyboard and mouse; dropped unless active and the peer allows keyboard (and mouse) input. */
  input(input: RemoteInput): void
  /** Null when not active, cancelled, started again or closed first. */
  recordSample(): Promise<VideoSample | null>
  cancelSample(): void
  /** Sends our clipboard text; false when not active, not allowed or too long. */
  clipboard(text: string): boolean
  close(): void
}

export interface SessionOptions {
  createDecoder?: CreateDecoder
  audioDecoders?: AudioDecoders
  clock?: StatsClock
  viewOnly?: boolean
  /** The fps the login asked for: the governor's ceiling until setOptions moves it. */
  maxFps?: number
  /** What the login declared this browser decodes; VP9 alone by default. */
  decoders?: DecoderSupport
}
