import type { DisplayRect } from '../input/mouse'
import type { DisplayInfo, KeyboardMode, PeerInfo, SwitchDisplay } from '../proto/gen/message_pb.js'
import { parseResolutions, parseSize, type Size } from './resolution'

export interface PeerView {
  info: PeerInfo
  displays: readonly DisplayInfo[]
  current: number
  display: DisplayRect | null
  keyboardMode: KeyboardMode
  resolutions: readonly Size[]
  originalResolution: Size | null
  cursorEmbedded: boolean
}

function rectOf(d: { x: number; y: number; width: number; height: number } | undefined) {
  if (!d || d.width <= 0 || d.height <= 0) return null
  return { x: d.x, y: d.y, width: d.width, height: d.height }
}

export function peerView(info: PeerInfo, keyboardMode: KeyboardMode): PeerView {
  return {
    info,
    displays: info.displays,
    current: info.currentDisplay,
    display: rectOf(info.displays[info.currentDisplay]),
    keyboardMode,
    resolutions: parseResolutions(info.resolutions?.resolutions),
    originalResolution: parseSize(info.displays[info.currentDisplay]?.originalResolution),
    cursorEmbedded: info.displays[info.currentDisplay]?.cursorEmbedded ?? false,
  }
}

/** A later Message.peer_info carries the displays again, the current one unchanged. */
export function withDisplays(view: PeerView, update: PeerInfo): PeerView {
  return {
    ...view,
    info: { ...view.info, displays: update.displays, platformAdditions: update.platformAdditions },
    displays: update.displays,
    display: rectOf(update.displays[view.current]) ?? view.display,
    resolutions: update.resolutions
      ? parseResolutions(update.resolutions.resolutions)
      : view.resolutions,
    originalResolution:
      parseSize(update.displays[view.current]?.originalResolution) ?? view.originalResolution,
    cursorEmbedded: update.displays[view.current]?.cursorEmbedded ?? view.cursorEmbedded,
  }
}

export function withSwitch(view: PeerView, s: SwitchDisplay): PeerView {
  return {
    ...view,
    current: s.display,
    display: rectOf(s) ?? rectOf(view.displays[s.display]),
    resolutions: parseResolutions(s.resolutions?.resolutions),
    originalResolution:
      parseSize(s.originalResolution) ?? parseSize(view.displays[s.display]?.originalResolution),
    cursorEmbedded: s.cursorEmbedded,
  }
}
