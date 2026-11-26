import type { CursorData } from '../proto/gen/message_pb.js'
import { compressBound, inflate } from './zstd'

export interface CursorShape {
  id: string
  width: number
  height: number
  hotX: number
  hotY: number
  rgba: Uint8ClampedArray<ArrayBuffer>
}

export interface CursorView {
  shape: CursorShape | null
  position: { x: number; y: number } | null
}

// The controller's own limits: at most 512×512 RGBA, 1 MiB once decoded.
export const MAX_CURSOR_SIZE = 512
const RGBA_CHANNELS = 4
/** Shapes kept for CursorId; a peer could otherwise grow the cache without bound. */
export const MAX_CACHED_SHAPES = 64

function inRange(value: number, low: number, high: number): boolean {
  return Number.isInteger(value) && value >= low && value <= high
}

/** The peer compresses `colors` with zstd. */
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

export interface CursorImages {
  image(shape: CursorShape, scale: number): string | null
  attach(shape: CursorShape, scale: number, url: string): boolean
}

interface Cached {
  shape: CursorShape
  image: { scale: number; url: string } | null
}

export class CursorTracker implements CursorImages {
  readonly #shapes = new Map<string, Cached>()
  #view: CursorView = { shape: null, position: null }
  #closed = false

  get view(): CursorView {
    return this.#view
  }

  /** A shape that does not decode shows nothing, and its id names nothing from then on. */
  onData(data: CursorData): CursorView {
    this.#drop(data.id)
    const shape = decodeCursor(data)
    if (shape) this.#keep({ shape, image: null })
    return this.#show(shape)
  }

  onId(id: string): CursorView {
    const cached = this.#shapes.get(id)
    if (cached) this.#keep(cached)
    return this.#show(cached?.shape ?? null)
  }

  onPosition(x: number, y: number): CursorView {
    this.#view = { ...this.#view, position: { x, y } }
    return this.#view
  }

  image(shape: CursorShape, scale: number): string | null {
    const cached = this.#shapes.get(shape.id)
    if (cached?.shape !== shape || cached.image?.scale !== scale) return null
    return cached.image.url
  }

  attach(shape: CursorShape, scale: number, url: string): boolean {
    const cached = this.#shapes.get(shape.id)
    if (this.#closed || cached?.shape !== shape) {
      URL.revokeObjectURL(url)
      return false
    }
    if (cached.image) URL.revokeObjectURL(cached.image.url)
    cached.image = { scale, url }
    return true
  }

  close(): void {
    this.#closed = true
    for (const id of [...this.#shapes.keys()]) this.#drop(id)
  }

  #keep(cached: Cached): void {
    this.#shapes.delete(cached.shape.id)
    this.#shapes.set(cached.shape.id, cached)
    if (this.#shapes.size <= MAX_CACHED_SHAPES) return
    const [oldest] = this.#shapes.keys()
    if (oldest !== undefined) this.#drop(oldest)
  }

  #drop(id: string): void {
    const image = this.#shapes.get(id)?.image
    this.#shapes.delete(id)
    if (image) URL.revokeObjectURL(image.url)
  }

  #show(shape: CursorShape | null): CursorView {
    this.#view = { ...this.#view, shape }
    return this.#view
  }
}
