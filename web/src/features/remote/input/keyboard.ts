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

/** One key going down or up, as the browser reports it (KeyboardEvent). */
export interface KeyInput {
  down: boolean
  /** KeyboardEvent.key: what the local layout produces. */
  key: string
  /** KeyboardEvent.code: the physical key. */
  code: string
  /** An IME is composing; its result arrives as text. */
  composing: boolean
  /** AltGr is held: Ctrl and Alt then belong to it, not to a shortcut. */
  altGraph: boolean
  capsLock: boolean
  numLock: boolean
  modifiers: Modifiers
}

// Column of KEY_CODES for each peer platform that reads Map-mode position codes.
const TABLE_COLUMN: Readonly<Record<string, 0 | 1 | 2>> = {
  [PLATFORM_WINDOWS]: 0,
  [PLATFORM_LINUX]: 1,
  [PLATFORM_MACOS]: 2,
}

// Peers before 1.2.0 know only Legacy (rustdesk 1.5.0 src/common.rs:1910-1927).
const MIN_VERSION_MAP_TRANSLATE = '1.2.0'

export function supportedKeyboardModes(platform: string, version: string): KeyboardMode[] {
  if (!atLeast(version, MIN_VERSION_MAP_TRANSLATE)) return [KeyboardMode.Legacy]
  const modes = [KeyboardMode.Legacy, KeyboardMode.Translate]
  if (platform !== PLATFORM_ANDROID && platform in TABLE_COLUMN) modes.push(KeyboardMode.Map)
  return modes
}

/**
 * Map when the peer takes it, as the desktop client defaults (rustdesk 1.5.0
 * src/keyboard.rs:758-764): position codes keep shortcuts and the peer's layout intact.
 * Otherwise Translate, which still types any character the local layout produces.
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

/** The modifiers a legacy event lists: those held, except the key itself. */
function legacyModifiers(input: KeyInput, controlKey: ControlKey | null): ControlKey[] {
  const own: Readonly<Partial<Record<ControlKey, ControlKey>>> = {
    [ControlKey.RAlt]: ControlKey.Alt,
    [ControlKey.RShift]: ControlKey.Shift,
    [ControlKey.RControl]: ControlKey.Control,
    [ControlKey.RWin]: ControlKey.Meta,
  }
  const self = controlKey === null ? null : (own[controlKey] ?? controlKey)
  return [
    ...modifierKeys(input.modifiers).filter((k) => k !== self),
    ...lockModifiers(input.code, input.capsLock, input.numLock),
  ]
}

/**
 * Turns browser key events into KeyEvent messages in one keyboard mode, and remembers which
 * keys it holds down on the peer so that each key goes up exactly as it went down.
 */
export class KeyboardEncoder {
  readonly mode: KeyboardMode
  readonly #platform: string
  readonly #column: 0 | 1 | 2 | null
  readonly #pressed = new Map<string, KeyEvent>()

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
      keyEvent({ mode: KeyboardMode.Translate, down: true, union: { case: 'seq', value: text } }),
    ]
  }

  /** Key-up for everything held down on the peer, for when the view loses focus. */
  releaseAll(): KeyEvent[] {
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

  // rustdesk 1.5.0 src/keyboard.rs:1247-1359 (map_keyboard_mode). Pause has no scancode on a
  // Windows peer and goes as a legacy control key (src/keyboard.rs:1257-1289); so does any other
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

  // rustdesk 1.5.0 src/keyboard.rs:1459-1529 (translate_keyboard_mode), except that keys without
  // a character go as legacy control keys: the peer's Translate handler takes only sequences and
  // position codes (src/server/input_service.rs:2254-2375), and position codes need Map's tables.
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

  // rustdesk 1.5.0 src/keyboard.rs:1018-1245 (legacy_keyboard_mode).
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
