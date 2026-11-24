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
import { PLATFORM_MACOS, PLATFORM_WINDOWS, atLeast } from './peer'

function misc(union: Misc['union']): Message {
  return create(MessageSchema, { union: { case: 'misc', value: create(MiscSchema, { union }) } })
}

export interface OptionPatch {
  /** Peer options other than custom_fps and the image quality, which the governors own. */
  peer?: OptionMessage
  maxFps?: number
  quality?: Quality
  viewOnly?: boolean
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

export function restartRemote(): Message {
  return misc({ case: 'restartRemoteDevice', value: true })
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

export type SessionAction = 'ctrl_alt_del' | 'lock_screen' | 'win' | 'alt_tab' | 'print_screen'

export const SESSION_ACTIONS: readonly SessionAction[] = [
  'ctrl_alt_del',
  'lock_screen',
  'win',
  'alt_tab',
  'print_screen',
]

function legacyKey(
  controlKey: ControlKey,
  fields: { down?: boolean; press?: boolean; modifiers?: ControlKey[] },
): KeyEvent {
  return create(KeyEventSchema, {
    mode: KeyboardMode.Legacy,
    union: { case: 'controlKey', value: controlKey },
    ...fields,
  })
}

/**
 * Always Legacy, as the official client sends them. `press` is down then up on the peer, except
 * for a modifier: a Linux peer reads its press as a release (rustdesk 1.5.1 src/server/connection.rs:3367).
 */
export function actionKeys(kind: SessionAction, platform: string): KeyEvent[] {
  switch (kind) {
    case 'ctrl_alt_del':
      // rustdesk 1.5.1 src/keyboard.rs:487-500
      return [
        platform === PLATFORM_WINDOWS
          ? legacyKey(ControlKey.CtrlAltDel, { down: true })
          : legacyKey(ControlKey.Delete, {
              press: true,
              modifiers: [ControlKey.Alt, ControlKey.Control],
            }),
      ]
    case 'lock_screen':
      // rustdesk 1.5.1 src/keyboard.rs:473-478
      return [legacyKey(ControlKey.LockScreen, { down: true })]
    case 'win':
      // no official menu entry; Meta is ControlKey.Meta at src/keyboard.rs:1149
      return [
        legacyKey(ControlKey.Meta, { down: true }),
        legacyKey(ControlKey.Meta, { down: false }),
      ]
    case 'alt_tab':
      // no official menu entry; press as src/keyboard.rs:487-500, Cmd is the Meta modifier (:450-454)
      return [
        legacyKey(ControlKey.Tab, {
          press: true,
          modifiers: [platform === PLATFORM_MACOS ? ControlKey.Meta : ControlKey.Alt],
        }),
      ]
    case 'print_screen':
      // no official menu entry; the peer maps Snapshot to PrintScreen at src/server/input_service.rs:2697
      return [legacyKey(ControlKey.Snapshot, { press: true })]
  }
}
