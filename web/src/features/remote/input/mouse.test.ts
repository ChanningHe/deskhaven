import { describe, expect, it } from 'vitest'

import { ControlKey } from '../proto/gen/message_pb.js'
import {
  NO_MODIFIERS,
  buttonFromDom,
  mouseEvent,
  mouseMask,
  toRemote,
  toView,
  wheelNotches,
  type MouseAction,
  type MouseButton,
} from './mouse'

// Types in the low 3 bits, buttons above them: mask = type | button << 3.
describe('mouseMask', () => {
  it.each<[MouseAction, MouseButton | null, number]>([
    ['move', null, 0],
    ['down', 'left', 1 | (0x01 << 3)],
    ['up', 'left', 2 | (0x01 << 3)],
    ['down', 'right', 1 | (0x02 << 3)],
    ['up', 'middle', 2 | (0x04 << 3)],
    ['down', 'back', 1 | (0x08 << 3)],
    ['up', 'forward', 2 | (0x10 << 3)],
    ['wheel', null, 3],
  ])('%s %s is %i', (action, button, mask) => {
    expect(mouseMask(action, button)).toBe(mask)
  })
})

describe('buttonFromDom', () => {
  it.each<[number, MouseButton | null]>([
    [0, 'left'],
    [1, 'middle'],
    [2, 'right'],
    [3, 'back'],
    [4, 'forward'],
    [5, null],
  ])('MouseEvent.button %i is %s', (dom, button) => {
    expect(buttonFromDom(dom)).toBe(button)
  })
})

const DISPLAY = { x: 1920, y: -200, width: 1280, height: 800 }

// The point in the display rect, as the client computes it: scale, add the display origin,
// clamp to the last pixel (Windows: one past it).
describe('toRemote', () => {
  it.each([
    ['top-left corner', 0, 0, 'Linux', { x: 1920, y: -200 }],
    ['view centre at half scale', 320, 200, 'Linux', { x: 2560, y: 200 }],
    ['bottom-right edge on Linux', 640, 400, 'Linux', { x: 3199, y: 599 }],
    ['bottom-right edge on Windows', 640, 400, 'Windows', { x: 3200, y: 600 }],
    ['outside the view, clamped', -10, 900, 'Mac OS', { x: 1920, y: 599 }],
    ['a fractional pixel, truncated', 0.75, 1.75, 'Linux', { x: 1921, y: -197 }],
  ])('%s', (_, x, y, platform, expected) => {
    expect(toRemote({ x, y, width: 640, height: 400 }, DISPLAY, platform)).toEqual(expected)
  })

  it('is null for an empty view or display', () => {
    expect(toRemote({ x: 1, y: 1, width: 0, height: 400 }, DISPLAY, 'Linux')).toBeNull()
    expect(
      toRemote({ x: 1, y: 1, width: 640, height: 400 }, { ...DISPLAY, width: 0 }, 'Linux'),
    ).toBeNull()
  })

  it('maps back to the view with toView', () => {
    expect(toView({ x: 2560, y: 200 }, DISPLAY, { width: 640, height: 400 })).toEqual({
      x: 320,
      y: 200,
    })
  })
})

// One notch per event on the dominant axis, scrolling down (positive delta) sent as -1.
describe('wheelNotches', () => {
  it.each([
    [0, 120, { x: 0, y: -1 }],
    [0, -3, { x: 0, y: 1 }],
    [40, 10, { x: -1, y: 0 }],
    [-40, 50, { x: 0, y: -1 }],
  ])('delta (%f, %f) is %o', (dx, dy, expected) => {
    expect(wheelNotches(dx, dy)).toEqual(expected)
  })

  it('is null without movement', () => {
    expect(wheelNotches(0, 0)).toBeNull()
  })
})

describe('mouseEvent', () => {
  it('lists held modifiers as rustdesk 1.5.0 src/client.rs:4343-4354 does', () => {
    const event = mouseEvent(9, 10, 20, { alt: true, ctrl: true, shift: true, meta: true })
    expect(event.union.case).toBe('mouseEvent')
    expect(event.union.value).toMatchObject({
      mask: 9,
      x: 10,
      y: 20,
      modifiers: [ControlKey.Alt, ControlKey.Shift, ControlKey.Control, ControlKey.Meta],
    })
  })

  it('lists none when none is held', () => {
    expect(mouseEvent(0, 1, 2, NO_MODIFIERS).union.value).toMatchObject({ modifiers: [] })
  })
})
