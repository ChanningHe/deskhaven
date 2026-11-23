import type { CursorImages, CursorShape } from './cursor'

export const MAX_CSS_CURSOR = 128
const SCALE_STEPS = 8

export function cursorScale(shown: number, peer: number): number | null {
  if (!(shown > 0) || !(peer > 0)) return null
  return Math.max(1, Math.round((shown / peer) * SCALE_STEPS)) / SCALE_STEPS
}

export interface CursorFit {
  width: number
  height: number
  hotX: number
  hotY: number
}

export function cursorFit(shape: CursorShape, scale: number): CursorFit | null {
  const width = Math.max(1, Math.round(shape.width * scale))
  const height = Math.max(1, Math.round(shape.height * scale))
  if (width > MAX_CSS_CURSOR || height > MAX_CSS_CURSOR) return null
  return {
    width,
    height,
    hotX: Math.min(width - 1, Math.round(shape.hotX * scale)),
    hotY: Math.min(height - 1, Math.round(shape.hotY * scale)),
  }
}

export function cssCursor(url: string, fit: CursorFit): string {
  return `url("${url}") ${String(fit.hotX)} ${String(fit.hotY)}, auto`
}

export type RenderCursor = (shape: CursorShape, fit: CursorFit) => Promise<Blob | null>

type Canvas = OffscreenCanvas | HTMLCanvasElement

function canvasOf(width: number, height: number): Canvas {
  if (typeof OffscreenCanvas !== 'undefined') return new OffscreenCanvas(width, height)
  const canvas = document.createElement('canvas')
  canvas.width = width
  canvas.height = height
  return canvas
}

function context2d(
  canvas: Canvas,
): (CanvasDrawImage & CanvasImageData & CanvasImageSmoothing) | null {
  return canvas instanceof HTMLCanvasElement ? canvas.getContext('2d') : canvas.getContext('2d')
}

function png(canvas: Canvas): Promise<Blob | null> {
  if (canvas instanceof HTMLCanvasElement) {
    return new Promise((resolve) => {
      canvas.toBlob(resolve, 'image/png')
    })
  }
  return canvas.convertToBlob({ type: 'image/png' })
}

export const renderCursor: RenderCursor = async (shape, fit) => {
  const source = canvasOf(shape.width, shape.height)
  const target = canvasOf(fit.width, fit.height)
  const sourceCtx = context2d(source)
  const ctx = context2d(target)
  if (!sourceCtx || !ctx) return null
  sourceCtx.putImageData(new ImageData(shape.rgba, shape.width, shape.height), 0, 0)
  ctx.imageSmoothingQuality = 'high'
  ctx.drawImage(source, 0, 0, fit.width, fit.height)
  return png(target)
}

export class CssCursors {
  readonly #images: CursorImages
  readonly #onReady: () => void
  readonly #render: RenderCursor
  #latest: { shape: CursorShape; scale: number } | null = null
  #seq = 0

  constructor(images: CursorImages, onReady: () => void, render: RenderCursor = renderCursor) {
    this.#images = images
    this.#onReady = onReady
    this.#render = render
  }

  cursor(shape: CursorShape, scale: number): string | null {
    const fit = cursorFit(shape, scale)
    const url = fit && this.#images.image(shape, scale)
    return fit && url ? cssCursor(url, fit) : null
  }

  prepare(shape: CursorShape, scale: number): void {
    const fit = cursorFit(shape, scale)
    if (!fit || this.#images.image(shape, scale) !== null) return
    if (this.#latest?.shape === shape && this.#latest.scale === scale) return
    const seq = ++this.#seq
    this.#latest = { shape, scale }
    this.#render(shape, fit).then(
      (blob) => {
        if (!blob || seq !== this.#seq) return
        if (this.#images.attach(shape, scale, URL.createObjectURL(blob))) this.#onReady()
      },
      () => undefined,
    )
  }
}
