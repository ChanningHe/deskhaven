import { create, type MessageInitShape } from '@bufbuild/protobuf'

import { ControlKey, KeyboardMode, KeyEventSchema, type KeyEvent } from '../proto/gen/message_pb.js'
import {
  PLATFORM_ANDROID,
  PLATFORM_LINUX,
  PLATFORM_MACOS,
  PLATFORM_WINDOWS,
  atLeast,
} from '../session/peer'
import {
  LOCK_KEYS,
  controlKeyOf,
  isNumpadKey,
  legacyChar,
  lockModifiers,
  singleCodePoint,
} from './control-keys'
import { KEY_CODES } from './keymap'
import { modifierKeys, type Modifiers } from './mouse'

export interface KeyInput {
  down: boolean
  key: string
  code: string
  composing: boolean
  /** AltGr is held: Ctrl and Alt then belong to it, not to a shortcut. */
  altGraph: boolean
  capsLock: boolean
  numLock: boolean
  modifiers: Modifiers
  time: number
}

// Column of KEY_CODES for each peer platform that reads Map-mode position codes.
const TABLE_COLUMN: Readonly<Record<string, 0 | 1 | 2>> = {
  [PLATFORM_WINDOWS]: 0,
  [PLATFORM_LINUX]: 1,
  [PLATFORM_MACOS]: 2,
}

// Peers before 1.2.0 know only Legacy.
const MIN_VERSION_MAP_TRANSLATE = '1.2.0'

// Windows reports AltGr as a ControlLeft keydown from the OS and the AltRight keydown, both at
// the same moment. The desktop client drops that ControlLeft by its 0x021D scan code; a browser
// has no scan code, so the pair is told apart by time instead.
const ALTGR_CTRL_GAP_MS = 10

function isAltGrPair(ctrl: KeyInput, next: KeyInput): boolean {
  const gap = next.time - ctrl.time
  return (
    next.down &&
    next.code === 'AltRight' &&
    next.key === 'AltGraph' &&
    gap >= 0 &&
    gap <= ALTGR_CTRL_GAP_MS
  )
}

export function supportedKeyboardModes(platform: string, version: string): KeyboardMode[] {
  if (!atLeast(version, MIN_VERSION_MAP_TRANSLATE)) return [KeyboardMode.Legacy]
  const modes = [KeyboardMode.Legacy, KeyboardMode.Translate]
  if (platform !== PLATFORM_ANDROID && platform in TABLE_COLUMN) modes.push(KeyboardMode.Map)
  return modes
}

/**
 * Map when the peer takes it, as the desktop client defaults: position codes keep shortcuts and
 * the peer's layout intact. Otherwise Translate, which still types any character the local
 * layout produces.
 */
export function keyboardModeFor(platform: string, version: string): KeyboardMode {
  const modes = supportedKeyboardModes(platform, version)
  if (modes.includes(KeyboardMode.Map)) return KeyboardMode.Map
  if (modes.includes(KeyboardMode.Translate)) return KeyboardMode.Translate
  return KeyboardMode.Legacy
}

function keyEvent(fields: MessageInitShape<typeof KeyEventSchema>): KeyEvent {
  return create(KeyEventSchema, fields)
}

/** Ctrl held with AltGr is AltGr's own. */
function legacyModifiers(input: KeyInput, controlKey: ControlKey | null): ControlKey[] {
  const own: Readonly<Partial<Record<ControlKey, ControlKey>>> = {
    [ControlKey.RAlt]: ControlKey.Alt,
    [ControlKey.RShift]: ControlKey.Shift,
    [ControlKey.RControl]: ControlKey.Control,
    [ControlKey.RWin]: ControlKey.Meta,
  }
  const self = controlKey === null ? null : (own[controlKey] ?? controlKey)
  const modifiers = input.altGraph ? { ...input.modifiers, ctrl: false } : input.modifiers
  return [
    ...modifierKeys(modifiers).filter((k) => k !== self),
    ...lockModifiers(input.code, input.capsLock, input.numLock),
  ]
}

/** Remembers which keys it holds down on the peer so that each goes up exactly as it went down. */
export class KeyboardEncoder {
  readonly mode: KeyboardMode
  readonly #platform: string
  readonly #column: 0 | 1 | 2 | null
  readonly #pressed = new Map<string, KeyEvent>()
  /** A ControlLeft keydown kept back until the next input shows whether AltGr sent it. */
  #heldCtrl: KeyInput | null = null

  constructor(mode: KeyboardMode, platform: string) {
    const column = TABLE_COLUMN[platform] ?? null
    if (mode === KeyboardMode.Map && column === null) {
      throw new Error(`no Map-mode key table for a ${platform} peer`)
    }
    this.mode = mode
    this.#platform = platform
    this.#column = column
  }

  key(input: KeyInput): KeyEvent[] {
    if (input.composing || input.key === 'Process') return []
    const held = this.#heldCtrl
    this.#heldCtrl = null
    if (held && isAltGrPair(held, input)) return this.#encode(input)
    const flushed = held ? this.#encode(held) : []
    if (this.mode !== KeyboardMode.Map && input.down && input.code === 'ControlLeft') {
      this.#heldCtrl = input
      return flushed
    }
    return [...flushed, ...this.#encode(input)]
  }

  /** Sends a held-back Ctrl; called before any other input that Ctrl may modify. */
  flush(): KeyEvent[] {
    const held = this.#heldCtrl
    this.#heldCtrl = null
    return held ? this.#encode(held) : []
  }

  #encode(input: KeyInput): KeyEvent[] {
    if (!input.down) return this.#release(input.code)
    const event = this.#down(input)
    if (!event) return []
    if (event.mode !== KeyboardMode.Translate) this.#pressed.set(input.code, event)
    return [event]
  }

  /** Text an IME committed. Translate mode types sequences on every peer, whatever our mode. */
  text(text: string): KeyEvent[] {
    if (text === '') return []
    return [
      ...this.flush(),
      keyEvent({ mode: KeyboardMode.Translate, down: true, union: { case: 'seq', value: text } }),
    ]
  }

  releaseAll(): KeyEvent[] {
    this.#heldCtrl = null
    const released = [...this.#pressed.keys()].flatMap((code) => this.#release(code))
    this.#pressed.clear()
    return released
  }

  #release(code: string): KeyEvent[] {
    const pressed = this.#pressed.get(code)
    if (!pressed) return []
    this.#pressed.delete(code)
    return [keyEvent({ ...pressed, down: false })]
  }

  #down(input: KeyInput): KeyEvent | null {
    switch (this.mode) {
      case KeyboardMode.Map:
        return this.#map(input)
      case KeyboardMode.Translate:
        return this.#translate(input)
      default:
        return this.#legacy(input)
    }
  }

  // Pause has no scancode on a Windows peer and goes as a legacy control key; so does any other
  // key rdev has no position code for.
  #map(input: KeyInput): KeyEvent | null {
    const isWindowsPause = this.#platform === PLATFORM_WINDOWS && input.code === 'Pause'
    const position = this.#column === null ? null : (KEY_CODES[input.code]?.[this.#column] ?? null)
    if (isWindowsPause || position === null) return this.#legacyControl(input)
    return keyEvent({
      mode: KeyboardMode.Map,
      down: true,
      union: { case: 'chr', value: position },
      modifiers: lockModifiers(input.code, input.capsLock, input.numLock),
    })
  }

  // Keys without a character go as legacy control keys: the peer's Translate handler takes only
  // sequences and position codes, and position codes need Map's tables.
  #translate(input: KeyInput): KeyEvent | null {
    if (input.key === 'Dead' || (input.altGraph && input.code === 'AltRight')) return null
    if (isNumpadKey(input.code)) return this.#legacyControl(input)
    const controlKey = controlKeyOf(input.key, input.code)
    if (controlKey !== null) return this.#legacyControl(input)
    if (singleCodePoint(input.key) === null) return null
    const { alt, ctrl, meta } = input.modifiers
    if (!input.altGraph && (alt || ctrl || meta)) return this.#legacy(input)
    return keyEvent({
      mode: KeyboardMode.Translate,
      down: true,
      union: { case: 'seq', value: input.key },
    })
  }

  #legacy(input: KeyInput): KeyEvent | null {
    if (LOCK_KEYS.has(input.key) || LOCK_KEYS.has(input.code)) return null
    if (controlKeyOf(input.key, input.code) !== null) return this.#legacyControl(input)
    const chr = legacyChar(input.key, input.code)
    if (chr === null) return null
    return keyEvent({
      mode: KeyboardMode.Legacy,
      down: true,
      union: { case: 'chr', value: chr },
      modifiers: legacyModifiers(input, null),
    })
  }

  #legacyControl(input: KeyInput): KeyEvent | null {
    const controlKey = controlKeyOf(input.key, input.code)
    if (controlKey === null || LOCK_KEYS.has(input.code)) return null
    return keyEvent({
      mode: KeyboardMode.Legacy,
      down: true,
      union: { case: 'controlKey', value: controlKey },
      modifiers: legacyModifiers(input, controlKey),
    })
  }
}
