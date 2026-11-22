import { create } from '@bufbuild/protobuf'

import { MessageSchema, type KeyEvent, type Message } from '../proto/gen/message_pb.js'
import type { KeyboardEncoder, KeyInput } from './keyboard'
import {
  mouseEvent,
  mouseMask,
  toRemote,
  wheelNotches,
  type DisplayRect,
  type Modifiers,
  type MouseButton,
  type ViewPoint,
} from './mouse'

/** What the view hands the session; built from DOM events by the helpers in `dom.ts`. */
export type RemoteInput =
  | {
      kind: 'pointer'
      action: 'move' | 'down' | 'up'
      button: MouseButton | null
      point: ViewPoint
      modifiers: Modifiers
    }
  | { kind: 'wheel'; deltaX: number; deltaY: number; modifiers: Modifiers }
  | ({ kind: 'key' } & KeyInput)
  | { kind: 'text'; text: string }
  /** The view lost focus: let go of every key still held down on the peer. */
  | { kind: 'release' }

export interface InputContext {
  keyboard: KeyboardEncoder
  /** The displayed screen of the peer; pointer input needs it. */
  display: DisplayRect | null
  platform: string
}

function keyMessages(events: KeyEvent[]): Message[] {
  return events.map((value) => create(MessageSchema, { union: { case: 'keyEvent', value } }))
}

export function encodeInput(input: RemoteInput, ctx: InputContext): Message[] {
  switch (input.kind) {
    case 'pointer': {
      if (!ctx.display) return []
      const at = toRemote(input.point, ctx.display, ctx.platform)
      if (!at) return []
      const button = input.action === 'move' ? null : input.button
      if (input.action !== 'move' && button === null) return []
      return [mouseEvent(mouseMask(input.action, button), at.x, at.y, input.modifiers)]
    }
    case 'wheel': {
      const notches = wheelNotches(input.deltaX, input.deltaY)
      if (!notches) return []
      return [mouseEvent(mouseMask('wheel', null), notches.x, notches.y, input.modifiers)]
    }
    case 'key':
      return keyMessages(ctx.keyboard.key(input))
    case 'text':
      return keyMessages(ctx.keyboard.text(input.text))
    case 'release':
      return keyMessages(ctx.keyboard.releaseAll())
  }
}
