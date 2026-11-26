import type { RemoteInput } from './input'
import { buttonFromDom, type Modifiers } from './mouse'

export function modifiersOf(e: MouseEvent | KeyboardEvent): Modifiers {
  return { alt: e.altKey, ctrl: e.ctrlKey, shift: e.shiftKey, meta: e.metaKey }
}

export function keyInput(e: KeyboardEvent): RemoteInput {
  return {
    kind: 'key',
    down: e.type === 'keydown',
    key: e.key,
    code: e.code,
    composing: e.isComposing,
    altGraph: e.getModifierState('AltGraph'),
    capsLock: e.getModifierState('CapsLock'),
    numLock: e.getModifierState('NumLock'),
    modifiers: modifiersOf(e),
    time: e.timeStamp,
  }
}

export function textInput(e: CompositionEvent): RemoteInput {
  return { kind: 'text', text: e.data }
}

const POINTER_ACTIONS: Readonly<Record<string, 'move' | 'down' | 'up'>> = {
  mousemove: 'move',
  pointermove: 'move',
  mousedown: 'down',
  pointerdown: 'down',
  mouseup: 'up',
  pointerup: 'up',
}

export function pointerInput(e: MouseEvent, view: Element): RemoteInput | null {
  const action = POINTER_ACTIONS[e.type]
  if (!action) return null
  const box = view.getBoundingClientRect()
  return {
    kind: 'pointer',
    action,
    button: action === 'move' ? null : buttonFromDom(e.button),
    point: {
      x: e.clientX - box.left,
      y: e.clientY - box.top,
      width: box.width,
      height: box.height,
    },
    modifiers: modifiersOf(e),
  }
}

export function wheelInput(e: WheelEvent): RemoteInput {
  return { kind: 'wheel', deltaX: e.deltaX, deltaY: e.deltaY, modifiers: modifiersOf(e) }
}
