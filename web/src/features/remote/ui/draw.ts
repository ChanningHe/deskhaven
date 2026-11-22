import type { CursorShape } from '../cursor/cursor'

export interface Size {
  width: number
  height: number
}

/** The largest size of `frame`'s aspect ratio that fits in `box`, in whole CSS pixels. */
export function fitSize(frame: Size, box: Size): Size | null {
  if (frame.width <= 0 || frame.height <= 0 || box.width <= 0 || box.height <= 0) return null
  const scale = Math.min(box.width / frame.width, box.height / frame.height)
  return {
    width: Math.max(1, Math.floor(frame.width * scale)),
    height: Math.max(1, Math.floor(frame.height * scale)),
  }
}

/** Where a client point over `view` falls in the canvas's own pixels. */
export function canvasPoint(
  canvas: { width: number; height: number },
  view: DOMRect,
  clientX: number,
  clientY: number,
): { x: number; y: number } | null {
  if (view.width <= 0 || view.height <= 0) return null
  return {
    x: ((clientX - view.left) * canvas.width) / view.width,
    y: ((clientY - view.top) * canvas.height) / view.height,
  }
}

/**
 * Paints the peer's cursor with its hotspot at `at` (canvas pixels) on a cleared overlay.
 * putImageData copies alpha as is, which is right on an otherwise transparent layer.
 */
export function drawCursor(
  ctx: CanvasRenderingContext2D,
  shape: CursorShape | null,
  at: { x: number; y: number } | null,
): void {
  ctx.clearRect(0, 0, ctx.canvas.width, ctx.canvas.height)
  if (!shape || !at) return
  const image = new ImageData(shape.rgba, shape.width, shape.height)
  ctx.putImageData(image, Math.round(at.x - shape.hotX), Math.round(at.y - shape.hotY))
}

/** Draws one decoded frame at its display size, resizing the canvas when that changes. */
export function drawFrame(canvas: HTMLCanvasElement, frame: VideoFrame): Size | null {
  const width = frame.displayWidth
  const height = frame.displayHeight
  let resized: Size | null = null
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width
    canvas.height = height
    resized = { width, height }
  }
  canvas.getContext('2d')?.drawImage(frame, 0, 0, width, height)
  return resized
}
