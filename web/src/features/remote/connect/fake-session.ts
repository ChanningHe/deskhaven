import { create } from '@bufbuild/protobuf'

import type { WebClientSession } from '@/api/generated/model'

import type { CursorImages, CursorShape, CursorView } from '../cursor/cursor'
import type { RemoteInput } from '../input/input'
import { DisplayInfoSchema, KeyboardMode, PeerInfoSchema } from '../proto/gen/message_pb.js'
import { FrameQueue } from '../session/frame-queue'
import type { OptionPatch } from '../session/messages'
import { peerView, type PeerView } from '../session/peer-view'
import type { Size } from '../session/resolution'
import type { CloseCause } from '../handshake/handshake'
import type { Permissions, Session, SessionState } from '../session/session'
import { NO_STATS, type SessionStats } from '../session/stats'
import { ValueStore } from '../session/store'

class KeptCursorImages implements CursorImages {
  readonly #kept = new WeakMap<CursorShape, { scale: number; url: string }>()

  image(shape: CursorShape, scale: number): string | null {
    const kept = this.#kept.get(shape)
    return kept?.scale === scale ? kept.url : null
  }

  attach(shape: CursorShape, scale: number, url: string): boolean {
    this.#kept.set(shape, { scale, url })
    return true
  }
}

/** A Session the test drives by hand; it records what the page asks of it. */
export class FakeSession implements Session {
  readonly state = new ValueStore<SessionState>({ kind: 'connecting' })
  readonly peer = new ValueStore<PeerView | null>(null)
  readonly cursor = new ValueStore<CursorView>({ shape: null, position: null })
  readonly cursorImages: CursorImages = new KeptCursorImages()
  readonly permissions = new ValueStore<Permissions>({ keyboard: true, clipboard: true })
  readonly remoteClipboard = new ValueStore<string | null>(null)
  readonly stats = new ValueStore<SessionStats>(NO_STATS)
  readonly queue = new FrameQueue()
  readonly logins: string[] = []
  readonly codes: string[] = []
  readonly inputs: RemoteInput[] = []
  readonly pasted: string[] = []
  readonly modes: KeyboardMode[] = []
  readonly chosenSessions: number[] = []
  readonly displays: number[] = []
  readonly resolutions: { width: number; height: number }[] = []
  readonly patches: OptionPatch[] = []
  ctrlAltDels = 0
  closed = 0

  get frames(): AsyncIterable<VideoFrame> {
    return this.queue
  }

  login(password: string): Promise<void> {
    this.logins.push(password)
    return Promise.resolve()
  }

  submit2fa(code: string): void {
    this.codes.push(code)
  }

  chooseWindowsSession(sid: number): void {
    this.chosenSessions.push(sid)
  }

  setKeyboardMode(mode: KeyboardMode): void {
    this.modes.push(mode)
    const peer = this.peer.get()
    if (peer) this.peer.set({ ...peer, keyboardMode: mode })
  }

  switchDisplay(index: number): void {
    this.displays.push(index)
  }

  changeResolution(width: number, height: number): void {
    this.resolutions.push({ width, height })
  }

  ctrlAltDel(): void {
    this.ctrlAltDels++
  }

  setOptions(patch: OptionPatch): void {
    this.patches.push(patch)
  }

  input(input: RemoteInput): void {
    this.inputs.push(input)
  }

  clipboard(text: string): boolean {
    this.pasted.push(text)
    return true
  }

  close(): void {
    this.closed++
    this.end({ by: 'user', reason: '' })
  }

  end(cause: CloseCause): void {
    if (this.state.get().kind === 'closed') return
    this.state.set({ kind: 'closed', cause })
    this.queue.end()
  }

  offerResolutions(sizes: readonly Size[], original: Size | null): void {
    const peer = this.peer.get()
    if (peer) this.peer.set({ ...peer, resolutions: sizes, originalResolution: original })
  }

  resize(width: number, height: number): void {
    const peer = this.peer.get()
    if (peer?.display) this.peer.set({ ...peer, display: { ...peer.display, width, height } })
  }

  /** Logs in: the peer is a 1.5.0 with `displays` screens of 1280×800. */
  activate(displays = 1, platform = 'Linux'): void {
    const peerInfo = create(PeerInfoSchema, {
      platform,
      version: '1.5.0',
      displays: Array.from({ length: displays }, (_, i) =>
        create(DisplayInfoSchema, { x: i * 1280, width: 1280, height: 800, name: `${i}` }),
      ),
      currentDisplay: 0,
    })
    this.peer.set(peerView(peerInfo, KeyboardMode.Map))
    this.state.set({ kind: 'active' })
  }
}

export const OPENED: WebClientSession = {
  session_id: 'sess-1',
  peer_id: '123456789',
  relay_url: '/ws/relay',
  server_pk: btoa('k'.repeat(32)),
  signed_peer_pk: btoa('signed'),
  uuid: 'uuid-1',
  my_id: 'alice',
  my_name: 'Alice',
  ice_servers: [],
}
