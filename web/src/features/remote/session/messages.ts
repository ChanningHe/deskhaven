import { create } from '@bufbuild/protobuf'

import { MessageSchema, MiscSchema, type Message, type Misc } from '../proto/gen/message_pb.js'
import { atLeast } from './peer'

function misc(union: Misc['union']): Message {
  return create(MessageSchema, { union: { case: 'misc', value: create(MiscSchema, { union }) } })
}

/** One VideoFrame message consumed (Misc.video_received; we log in with video_ack_required). */
export function videoReceived(): Message {
  return misc({ case: 'videoReceived', value: true })
}

// Peers from 1.2.4 refresh one display (rustdesk 1.5.0 src/ui_session_interface.rs:442-450,
// MIN_VER_MULTI_UI_SESSION at src/common.rs:65).
const MIN_VERSION_REFRESH_DISPLAY = '1.2.4'

/** Asks the peer to start its stream over with a key frame. */
export function refreshVideo(display: number, peerVersion: string): Message {
  return atLeast(peerVersion, MIN_VERSION_REFRESH_DISPLAY)
    ? misc({ case: 'refreshVideoDisplay', value: display })
    : misc({ case: 'refreshVideo', value: true })
}

/** Sent before we hang up; empty, as the client sends it (src/client/io_loop.rs:726-728). */
export function closeReason(reason = ''): Message {
  return misc({ case: 'closeReason', value: reason })
}
