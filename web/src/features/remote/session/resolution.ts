import type { PeerView } from './peer-view'

export interface Size {
  width: number
  height: number
}

export const MAX_RESOLUTIONS = 64
const MAX_SIDE = 16384

function validSize(r: Size | undefined): r is Size {
  if (!r) return false
  return [r.width, r.height].every((n) => Number.isInteger(n) && n >= 1 && n <= MAX_SIDE)
}

export function parseSize(r: Size | undefined): Size | null {
  return validSize(r) ? { width: r.width, height: r.height } : null
}

export function parseResolutions(list: readonly Size[] | undefined): Size[] {
  const seen = new Map<string, Size>()
  for (const r of list ?? []) {
    const size = parseSize(r)
    if (size) seen.set(`${size.width}x${size.height}`, size)
  }
  return [...seen.values()]
    .sort((a, b) => b.width * b.height - a.width * a.height)
    .slice(0, MAX_RESOLUTIONS)
}

export function sameSize(a: Size, b: Size): boolean {
  return a.width === b.width && a.height === b.height
}

/** The peer checks nothing, so we only ask for a size it listed or its original one. */
export function offersResolution(peer: PeerView, width: number, height: number): boolean {
  const wanted = { width, height }
  return (
    peer.resolutions.some((r) => sameSize(r, wanted)) ||
    (peer.originalResolution !== null && sameSize(peer.originalResolution, wanted))
  )
}
