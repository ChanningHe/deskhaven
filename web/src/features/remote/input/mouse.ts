import { create } from '@bufbuild/protobuf'

import {
  ControlKey,
  MessageSchema,
  MouseEventSchema,
  type Message,
} from '../proto/gen/message_pb.js'

export type MouseAction = 'move' | 'down' | 'up' | 'wheel'
export type MouseButton = 'left' | 'right' | 'middle' | 'back' | 'forward'

export interface Modifiers {
  alt: boolean
  ctrl: boolean
  shift: boolean
  meta: boolean
}

export const NO_MODIFIERS: Modifiers = { alt: false, ctrl: false, shift: false, meta: false }

/** A display's rectangle in the peer's virtual screen (DisplayInfo / SwitchDisplay). */
export interface DisplayRect {
  x: number
  y: number
  width: number
  height: number
}

/** A point in the rendered view of the display, with the view's size, both in CSS pixels. */
export interface ViewPoint {
  x: number
  y: number
  width: number
  height: number
}

// MouseEvent.mask: the event type in the low 3 bits, the button above them (rustdesk 1.5.0
// src/common.rs:67-91, combined as in src/flutter_ffi.rs:2031-2048).
const MOUSE_TYPE: Readonly<Record<MouseAction, number>> = { move: 0, down: 1, up: 2, wheel: 3 }
const MOUSE_BUTTON: Readonly<Record<MouseButton, number>> = {
  left: 0x01,
  right: 0x02,
  middle: 0x04,
  back: 0x08,
  forward: 0x10,
}
const BUTTON_SHIFT = 3

export function mouseMask(action: MouseAction, button: MouseButton | null): number {
  return MOUSE_TYPE[action] | ((button ? MOUSE_BUTTON[button] : 0) << BUTTON_SHIFT)
}

// DOM MouseEvent.button numbering (UI Events §5.1.3.1).
const DOM_BUTTONS: readonly MouseButton[] = ['left', 'middle', 'right', 'back', 'forward']

export function buttonFromDom(button: number): MouseButton | null {
  return DOM_BUTTONS[button] ?? null
}

const PLATFORM_WINDOWS = 'Windows'

/**
 * The peer screen pixel under `point`, clamped into `rect`. A Windows peer takes one pixel past
 * the right and bottom edges, for window snapping (rustdesk 1.5.0
 * flutter/lib/models/input_model.dart:2068-2075).
 */
export function toRemote(
  point: ViewPoint,
  rect: DisplayRect,
  platform: string,
): { x: number; y: number } | null {
  if (point.width <= 0 || point.height <= 0 || rect.width <= 0 || rect.height <= 0) return null
  const edge = platform === PLATFORM_WINDOWS ? 0 : 1
  const x = rect.x + (point.x * rect.width) / point.width
  const y = rect.y + (point.y * rect.height) / point.height
  return {
    x: Math.floor(Math.min(Math.max(x, rect.x), rect.x + rect.width - edge)),
    y: Math.floor(Math.min(Math.max(y, rect.y), rect.y + rect.height - edge)),
  }
}

/** Where a peer screen point (such as CursorPosition) falls in a view of `rect`. */
export function toView(
  remote: { x: number; y: number },
  rect: DisplayRect,
  view: { width: number; height: number },
): { x: number; y: number } {
  return {
    x: ((remote.x - rect.x) * view.width) / rect.width,
    y: ((remote.y - rect.y) * view.height) / rect.height,
  }
}

/**
 * One wheel notch on the dominant axis per event; the peer reads a positive y as scrolling up
 * (rustdesk 1.5.0 flutter/lib/models/input_model.dart:1683-1698).
 */
export function wheelNotches(deltaX: number, deltaY: number): { x: number; y: number } | null {
  if (deltaX === 0 && deltaY === 0) return null
  if (Math.abs(deltaX) > Math.abs(deltaY)) return { x: -Math.sign(deltaX), y: 0 }
  return { x: 0, y: -Math.sign(deltaY) }
}

/** Held modifiers in the order rustdesk 1.5.0 src/client.rs:4343-4354 lists them. */
export function modifierKeys(m: Modifiers): ControlKey[] {
  const keys: ControlKey[] = []
  if (m.alt) keys.push(ControlKey.Alt)
  if (m.shift) keys.push(ControlKey.Shift)
  if (m.ctrl) keys.push(ControlKey.Control)
  if (m.meta) keys.push(ControlKey.Meta)
  return keys
}

export function mouseEvent(mask: number, x: number, y: number, modifiers: Modifiers): Message {
  return create(MessageSchema, {
    union: {
      case: 'mouseEvent',
      value: create(MouseEventSchema, { mask, x, y, modifiers: modifierKeys(modifiers) }),
    },
  })
}
