import { describe, expect, it } from 'vitest'

import { canvasPoint } from './draw'

const CANVAS = { width: 1280, height: 800 }

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
