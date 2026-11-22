import type { DisplayRect } from '../input/mouse'
import type { DisplayInfo, KeyboardMode, PeerInfo, SwitchDisplay } from '../proto/gen/message_pb.js'

/** What we know of the peer: its PeerInfo and the display we are looking at. */
export interface PeerView {
  info: PeerInfo
  displays: readonly DisplayInfo[]
  /** Index into `displays` of the display the peer streams. */
  current: number
  /** That display's rectangle in the peer's virtual screen, null while unknown. */
  display: DisplayRect | null
  keyboardMode: KeyboardMode
}

function rectOf(d: { x: number; y: number; width: number; height: number } | undefined) {
  if (!d || d.width <= 0 || d.height <= 0) return null
  return { x: d.x, y: d.y, width: d.width, height: d.height }
}

/** From LoginResponse.peer_info. */
export function peerView(info: PeerInfo, keyboardMode: KeyboardMode): PeerView {
  return {
    info,
    displays: info.displays,
    current: info.currentDisplay,
    display: rectOf(info.displays[info.currentDisplay]),
    keyboardMode,
  }
}

/**
 * A later Message.peer_info carries the displays again, the current one unchanged (rustdesk
 * 1.5.0 src/client/io_loop.rs:2335-2338).
 */
export function withDisplays(view: PeerView, update: PeerInfo): PeerView {
  return {
    ...view,
    info: { ...view.info, displays: update.displays, platformAdditions: update.platformAdditions },
    displays: update.displays,
    display: rectOf(update.displays[view.current]) ?? view.display,
  }
}

/** Misc.switch_display: the peer now streams `s.display` at this rectangle. */
export function withSwitch(view: PeerView, s: SwitchDisplay): PeerView {
  return {
    ...view,
    current: s.display,
    display: rectOf(s) ?? rectOf(view.displays[s.display]),
  }
}
