import { describe, expect, it, vi } from 'vitest'

import type { CursorShape } from '../cursor/cursor'
import { canvasPoint, drawCursor } from './draw'

const CANVAS = { width: 1280, height: 800 }

class FakeImageData {
  readonly data: Uint8ClampedArray
  readonly width: number
  readonly height: number
  constructor(data: Uint8ClampedArray, width: number, height: number) {
    this.data = data
    this.width = width
    this.height = height
  }
}

describe('canvasPoint', () => {
  it('scales a point on a screen fitted into the window', () => {
    const view = DOMRect.fromRect({ x: 20, y: 60, width: 640, height: 400 })
    expect(canvasPoint(CANVAS, view, 340, 260)).toEqual({ x: 640, y: 400 })
  })

  it('takes a scrolled original-size screen one to one', () => {
    const view = DOMRect.fromRect({ x: -300, y: -200, width: 1280, height: 800 })
    expect(canvasPoint(CANVAS, view, 100, 50)).toEqual({ x: 400, y: 250 })
  })
})

describe('drawCursor', () => {
  const shape: CursorShape = {
    id: '1',
    width: 12,
    height: 18,
    hotX: 2,
    hotY: 3,
    rgba: new Uint8ClampedArray(12 * 18 * 4),
  }

  function context() {
    vi.stubGlobal('ImageData', FakeImageData)
    const ctx = { canvas: CANVAS, clearRect: vi.fn(), putImageData: vi.fn() }
    return { ctx, as2d: ctx as unknown as CanvasRenderingContext2D }
  }

  it('draws the hotspot at the point and returns the rectangle it covered', () => {
    const { ctx, as2d } = context()
    const drawn = drawCursor(as2d, shape, { x: 100.4, y: 50.6 }, null)
    expect(drawn).toEqual({ x: 98, y: 48, width: 12, height: 18 })
    expect(ctx.putImageData).toHaveBeenCalledWith(expect.anything(), 98, 48)
    expect(ctx.clearRect).not.toHaveBeenCalled()
  })

  it('clears only the rectangle it drew last, never the whole canvas', () => {
    const { ctx, as2d } = context()
    const first = drawCursor(as2d, shape, { x: 100, y: 50 }, null)
    const second = drawCursor(as2d, shape, { x: 300, y: 200 }, first)
    expect(ctx.clearRect.mock.calls).toEqual([[98, 47, 12, 18]])
    expect(drawCursor(as2d, null, { x: 1, y: 1 }, second)).toBeNull()
    expect(drawCursor(as2d, shape, null, null)).toBeNull()
    expect(ctx.clearRect.mock.calls).toEqual([
      [98, 47, 12, 18],
      [298, 197, 12, 18],
    ])
    expect(ctx.putImageData).toHaveBeenCalledTimes(2)
  })
})
