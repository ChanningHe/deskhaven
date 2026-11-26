import type { PeerInfo } from '../proto/gen/message_pb.js'

/** The peer's own list; anything past these bounds is not a real Windows host's. */
export const MAX_WINDOWS_SESSIONS = 64
const MAX_SESSION_NAME = 256

export interface WindowsSessionOption {
  sid: number
  name: string
}

export interface WindowsSessionChoice {
  sessions: readonly WindowsSessionOption[]
  /** The session the peer runs in now: picking it starts the stream on this connection. */
  current: number
}

/**
 * An installed Windows peer sharing RDP sessions, with more than one session and this as its only
 * connection, starts no service until the client answers with Misc.selected_sid. Picking another
 * session restarts the peer there and drops this connection.
 */
export function windowsSessionChoice(info: PeerInfo): WindowsSessionChoice | null {
  const offered = info.windowsSessions?.sessions ?? []
  if (offered.length === 0) return null
  const bySid = new Map<number, WindowsSessionOption>()
  for (const { sid, name } of offered) {
    if (bySid.size === MAX_WINDOWS_SESSIONS) break
    if (!bySid.has(sid)) bySid.set(sid, { sid, name: name.slice(0, MAX_SESSION_NAME) })
  }
  return {
    sessions: [...bySid.values()],
    current: info.windowsSessions?.currentSid ?? 0,
  }
}
