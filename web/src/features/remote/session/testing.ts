import { create } from '@bufbuild/protobuf'
import { vi } from 'vitest'

import { fixtureBytes, fixtureSession } from '../__fixtures__/fixtures'
import { msg, replayToKeyed, sealed, type Replay } from '../__fixtures__/harness'
import {
  DisplayInfoSchema,
  LoginResponseSchema,
  MiscSchema,
  PeerInfoSchema,
  type Message,
  type Misc,
  type PeerInfo,
} from '../proto/gen/message_pb.js'
import { FakeDecoders } from '../video/fake-decoder'
import { startSession, type Session, type SessionState } from './session'

/** The 1.4.5 capture's peer as its PeerInfo would read (the real one was not kept). */
export const PEER_INFO = create(PeerInfoSchema, {
  platform: 'Linux',
  version: '1.4.5',
  displays: [create(DisplayInfoSchema, { width: 1280, height: 800, online: true, name: 'screen' })],
  currentDisplay: 0,
})

export interface ActiveSession extends Replay {
  session: Session
  decoders: FakeDecoders
  /** Seals and delivers a message from the peer. */
  peerSends(message: Message): void
  /** Delivers the captured VP9 key frame (peer frame 6), sealing fillers up to it. */
  peerSendsCapturedFrame(): void
  /** What we sent the peer after logging in, opened as the peer would. */
  sentAfterLogin(): Message[]
}

export function misc(union: Misc['union']): Message {
  return msg({ case: 'misc', value: create(MiscSchema, { union }) })
}

const CAPTURED_FRAME_SEQ = 6

/**
 * Replays the captured 1.4.5 peer through the real relay transport and handshake, then logs in
 * with a synthetic LoginResponse: the session is active over the fixture's keys.
 */
export async function activeSession(
  peerInfo: PeerInfo = PEER_INFO,
  until: SessionState['kind'] = 'active',
): Promise<ActiveSession> {
  const s = fixtureSession('1.4.5')
  const replay = await replayToKeyed('1.4.5', { password: s.password })
  const decoders = new FakeDecoders()
  const session = startSession(replay.handshake, { createDecoder: decoders.create })
  let peerSeq = 1
  replay.socket.receive(fixtureBytes('1.4.5', 'hbbr-enc-01-Hash'))
  replay.peer.seal(new Uint8Array(2)) // frame 1 was the captured Hash; keep the peer's count
  const peerSends = (message: Message) => {
    peerSeq++
    replay.socket.receive(sealed(replay.peer, message))
  }
  await replay.socket.waitForSent(3)
  peerSends(
    msg({
      case: 'loginResponse',
      value: create(LoginResponseSchema, { union: { case: 'peerInfo', value: peerInfo } }),
    }),
  )
  await vi.waitFor(() => {
    if (session.state.get().kind !== until) throw new Error(`session not ${until} yet`)
  })
  return {
    ...replay,
    session,
    decoders,
    peerSends,
    peerSendsCapturedFrame: () => {
      if (peerSeq >= CAPTURED_FRAME_SEQ) throw new Error('the peer already sent frame 6')
      while (peerSeq < CAPTURED_FRAME_SEQ - 1) peerSends(misc({ case: undefined }))
      peerSeq++
      replay.socket.receive(fixtureBytes('1.4.5', 'hbbr-enc-06-VideoFrame'))
    },
    sentAfterLogin: () => replay.peerReceived().slice(1),
  }
}

/** Waits until the session has handled everything delivered so far. */
export async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0))
}
