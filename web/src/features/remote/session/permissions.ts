import { PermissionInfo_Permission } from '../proto/gen/message_pb.js'

/** The peer reports only denials, so all start allowed. */
export interface Permissions {
  keyboard: boolean
  clipboard: boolean
  restart: boolean
  audio: boolean
}

export const ALL_ALLOWED: Permissions = {
  keyboard: true,
  clipboard: true,
  restart: true,
  audio: true,
}

// The keyboard permission is the peer's switch for all input: its server drops mouse events too
// while it is off.
export function withPermission(
  current: Permissions,
  permission: PermissionInfo_Permission,
  enabled: boolean,
): Permissions {
  switch (permission) {
    case PermissionInfo_Permission.Keyboard:
      return { ...current, keyboard: enabled }
    case PermissionInfo_Permission.Clipboard:
      return { ...current, clipboard: enabled }
    case PermissionInfo_Permission.Restart:
      return { ...current, restart: enabled }
    case PermissionInfo_Permission.Audio:
      return { ...current, audio: enabled }
    default:
      return current
  }
}
