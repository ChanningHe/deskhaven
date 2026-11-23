import { create } from '@bufbuild/protobuf'

import {
  CaptureDisplaysSchema,
  ControlKey,
  DisplayResolutionSchema,
  KeyboardMode,
  KeyEventSchema,
  MessageSchema,
  MiscSchema,
  OptionMessageSchema,
  ResolutionSchema,
  SwitchDisplaySchema,
  type KeyEvent,
  type Message,
  type Misc,
  type OptionMessage,
} from '../proto/gen/message_pb.js'
import type { Quality } from '../settings/settings'
import { PLATFORM_WINDOWS, atLeast } from './peer'

function misc(union: Misc['union']): Message {
  return create(MessageSchema, { union: { case: 'misc', value: create(MiscSchema, { union }) } })
}

export interface OptionPatch {
  /** Peer options other than custom_fps and the image quality, which the governors own. */
  peer?: OptionMessage
  maxFps?: number
  quality?: Quality
}

export function optionMessage(option: OptionMessage): Message {
  return misc({ case: 'option', value: option })
}

export function customFps(fps: number): Message {
  return optionMessage(create(OptionMessageSchema, { customFps: fps }))
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

// message.proto:847-861: change_resolution is deprecated since 1.2.4 for change_display_resolution.
const MIN_VERSION_DISPLAY_RESOLUTION = '1.2.4'

export function changeResolution(
  display: number,
  width: number,
  height: number,
  peerVersion: string,
): Message {
  const resolution = create(ResolutionSchema, { width, height })
  return atLeast(peerVersion, MIN_VERSION_DISPLAY_RESOLUTION)
    ? misc({
        case: 'changeDisplayResolution',
        value: create(DisplayResolutionSchema, { display, resolution }),
      })
    : misc({ case: 'changeResolution', value: resolution })
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
