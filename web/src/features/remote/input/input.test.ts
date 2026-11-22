import { describe, expect, it, vi } from 'vitest'

import { KeyboardMode } from '../proto/gen/message_pb.js'
import { keyInput, pointerInput, textInput, wheelInput } from './dom'
import { encodeInput, type InputContext, type RemoteInput } from './input'
import { KeyboardEncoder } from './keyboard'
import { NO_MODIFIERS } from './mouse'

function context(display: InputContext['display'] = { x: 0, y: 0, width: 1280, height: 800 }) {
  return {
    keyboard: new KeyboardEncoder(KeyboardMode.Map, 'Linux'),
    display,
    platform: 'Linux',
  }
}

function view(): HTMLElement {
  const element = document.createElement('canvas')
  vi.spyOn(element, 'getBoundingClientRect').mockReturnValue(
    DOMRect.fromRect({ x: 100, y: 50, width: 640, height: 400 }),
  )
  return element
}

describe('DOM adapters', () => {
  it('reads a key event with its lock and AltGr state', () => {
    const e = new KeyboardEvent('keydown', {
      key: '@',
      code: 'KeyQ',
      ctrlKey: true,
      altKey: true,
      modifierAltGraph: true,
      modifierCapsLock: true,
    })
    expect(keyInput(e)).toEqual({
      kind: 'key',
      down: true,
      key: '@',
      code: 'KeyQ',
      composing: false,
      altGraph: true,
      capsLock: true,
      numLock: false,
      modifiers: { alt: true, ctrl: true, shift: false, meta: false },
    })
    expect(keyInput(new KeyboardEvent('keyup', { key: 'a' }))).toMatchObject({ down: false })
  })

  it('places a pointer event in the view', () => {
    const e = new MouseEvent('mousedown', { clientX: 420, clientY: 250, button: 2, shiftKey: true })
    expect(pointerInput(e, view())).toEqual({
      kind: 'pointer',
      action: 'down',
      button: 'right',
      point: { x: 320, y: 200, width: 640, height: 400 },
      modifiers: { alt: false, ctrl: false, shift: true, meta: false },
    })
    expect(pointerInput(new MouseEvent('click'), view())).toBeNull()
  })

  it('reads wheel deltas and composed text', () => {
    expect(wheelInput(new WheelEvent('wheel', { deltaY: 100 }))).toMatchObject({
      deltaX: 0,
      deltaY: 100,
    })
    expect(textInput(new CompositionEvent('compositionend', { data: '你好' }))).toEqual({
      kind: 'text',
      text: '你好',
    })
  })
})

describe('encodeInput', () => {
  const pointer = (action: 'move' | 'down' | 'up', button: 'left' | null): RemoteInput => ({
    kind: 'pointer',
    action,
    button,
    point: { x: 320, y: 200, width: 640, height: 400 },
    modifiers: NO_MODIFIERS,
  })

  it('sends pointer events at the peer position', () => {
    expect(encodeInput(pointer('down', 'left'), context())[0]?.union.value).toMatchObject({
      mask: 1 | (1 << 3),
      x: 640,
      y: 400,
    })
    expect(encodeInput(pointer('move', 'left'), context())[0]?.union.value).toMatchObject({
      mask: 0,
    })
  })

  it('drops pointer events without a display or a known button', () => {
    expect(encodeInput(pointer('move', null), context(null))).toEqual([])
    expect(encodeInput(pointer('up', null), context())).toEqual([])
  })

  it('sends a wheel notch and nothing for a still wheel', () => {
    const wheel = (deltaY: number): RemoteInput => ({
      kind: 'wheel',
      deltaX: 0,
      deltaY,
      modifiers: NO_MODIFIERS,
    })
    expect(encodeInput(wheel(-53), context())[0]?.union.value).toMatchObject({
      mask: 3,
      x: 0,
      y: 1,
    })
    expect(encodeInput(wheel(0), context())).toEqual([])
  })

  it('wraps key events, text and releases as KeyEvent messages', () => {
    const ctx = context()
    const key = keyInput(new KeyboardEvent('keydown', { key: 'a', code: 'KeyA' }))
    expect(encodeInput(key, ctx).map((m) => m.union.case)).toEqual(['keyEvent'])
    expect(encodeInput({ kind: 'text', text: 'é' }, ctx)).toHaveLength(1)
    expect(encodeInput({ kind: 'release' }, ctx)).toHaveLength(1)
  })
})
