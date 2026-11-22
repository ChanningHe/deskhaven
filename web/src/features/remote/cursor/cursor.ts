import type { CursorData } from '../proto/gen/message_pb.js'
import { compressBound, inflate } from './zstd'

/** A cursor image in RGBA, ready for ImageData; the hotspot is the pixel that points. */
export interface CursorShape {
  id: string
  width: number
  height: number
  hotX: number
  hotY: number
  rgba: Uint8ClampedArray<ArrayBuffer>
}

/** What the peer's cursor looks like and where it is, in the peer's screen coordinates. */
export interface CursorView {
  shape: CursorShape | null
  position: { x: number; y: number } | null
}

// The controller's own limits (rustdesk 1.5.0 src/client/io_loop.rs:2794-2796): at most
// 512×512 RGBA, 1 MiB once decoded.
export const MAX_CURSOR_SIZE = 512
const RGBA_CHANNELS = 4
/** Shapes kept for CursorId; a peer could otherwise grow the cache without bound. */
export const MAX_CACHED_SHAPES = 64

function inRange(value: number, low: number, high: number): boolean {
  return Number.isInteger(value) && value >= low && value <= high
}

/** The peer compresses `colors` with zstd (rustdesk 1.5.0 src/server/input_service.rs:562). */
export function decodeCursor(data: CursorData): CursorShape | null {
  const { width, height, hotx, hoty } = data
  if (!inRange(width, 1, MAX_CURSOR_SIZE) || !inRange(height, 1, MAX_CURSOR_SIZE)) return null
  if (!inRange(hotx, 0, width - 1) || !inRange(hoty, 0, height - 1)) return null
  const expected = width * height * RGBA_CHANNELS
  if (data.colors.length > compressBound(expected)) return null
  const rgba = inflate(data.colors, expected)
  if (rgba?.length !== expected) return null
  return {
    id: data.id,
    width,
    height,
    hotX: hotx,
    hotY: hoty,
    rgba: new Uint8ClampedArray(rgba.buffer as ArrayBuffer, rgba.byteOffset, rgba.length),
  }
}

/** Follows CursorData, CursorId and CursorPosition from the peer. */
export class CursorTracker {
  readonly #shapes = new Map<string, CursorShape>()
  #view: CursorView = { shape: null, position: null }

  get view(): CursorView {
    return this.#view
  }

  /** A shape that does not decode shows nothing, and its id names nothing from then on. */
  onData(data: CursorData): CursorView {
    this.#shapes.delete(data.id)
    const shape = decodeCursor(data)
    if (shape) this.#keep(shape)
    return this.#show(shape)
  }

  onId(id: string): CursorView {
    const shape = this.#shapes.get(id) ?? null
    if (shape) this.#keep(shape)
    return this.#show(shape)
  }

  onPosition(x: number, y: number): CursorView {
    this.#view = { ...this.#view, position: { x, y } }
    return this.#view
  }

  #keep(shape: CursorShape): void {
    this.#shapes.delete(shape.id)
    this.#shapes.set(shape.id, shape)
    if (this.#shapes.size <= MAX_CACHED_SHAPES) return
    const [oldest] = this.#shapes.keys()
    if (oldest !== undefined) this.#shapes.delete(oldest)
  }

  #show(shape: CursorShape | null): CursorView {
    this.#view = { ...this.#view, shape }
    return this.#view
  }
}
