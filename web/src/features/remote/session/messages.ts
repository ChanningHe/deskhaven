import { create } from '@bufbuild/protobuf'

import {
  CaptureDisplaysSchema,
  ControlKey,
  KeyboardMode,
  KeyEventSchema,
  MessageSchema,
  MiscSchema,
  SwitchDisplaySchema,
  type KeyEvent,
  type Message,
  type Misc,
} from '../proto/gen/message_pb.js'
import { PLATFORM_WINDOWS, atLeast } from './peer'

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

/** Answers PeerInfo.windows_sessions (rustdesk 1.5.1 src/ui_session_interface.rs:1629-1636). */
export function selectedSid(sid: number): Message {
  return misc({ case: 'selectedSid', value: sid })
}

export function keyMessage(event: KeyEvent): Message {
  return create(MessageSchema, { union: { case: 'keyEvent', value: event } })
}

export function switchDisplay(display: number): Message {
  return misc({ case: 'switchDisplay', value: create(SwitchDisplaySchema, { display }) })
}

export function captureDisplay(display: number): Message {
  return misc({ case: 'captureDisplays', value: create(CaptureDisplaysSchema, { set: [display] }) })
}

/**
 * rustdesk 1.5.0 src/keyboard.rs:433-445: a Windows peer takes the CtrlAltDel key (it raises
 * the secure attention sequence); others get Delete pressed with Ctrl and Alt.
 */
export function ctrlAltDel(platform: string): KeyEvent {
  if (platform === PLATFORM_WINDOWS) {
    return create(KeyEventSchema, {
      mode: KeyboardMode.Legacy,
      down: true,
      union: { case: 'controlKey', value: ControlKey.CtrlAltDel },
    })
  }
  return create(KeyEventSchema, {
    mode: KeyboardMode.Legacy,
    press: true,
    union: { case: 'controlKey', value: ControlKey.Delete },
    modifiers: [ControlKey.Alt, ControlKey.Control],
  })
}
