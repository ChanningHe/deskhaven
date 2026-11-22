import { ControlKey } from '../proto/gen/message_pb.js'

// DOM KeyboardEvent.code → ControlKey where the key's position matters (left/right modifiers,
// numpad), after the rdev Key → ControlKey match of rustdesk 1.5.0 src/keyboard.rs:1059-1150.
const BY_CODE: Readonly<Record<string, ControlKey>> = {
  AltLeft: ControlKey.Alt,
  AltRight: ControlKey.RAlt,
  ControlLeft: ControlKey.Control,
  ControlRight: ControlKey.RControl,
  ShiftLeft: ControlKey.Shift,
  ShiftRight: ControlKey.RShift,
  MetaLeft: ControlKey.Meta,
  MetaRight: ControlKey.RWin,
  OSLeft: ControlKey.Meta,
  OSRight: ControlKey.RWin,
  Space: ControlKey.Space,
  NumpadEnter: ControlKey.NumpadEnter,
  Numpad0: ControlKey.Numpad0,
  Numpad1: ControlKey.Numpad1,
  Numpad2: ControlKey.Numpad2,
  Numpad3: ControlKey.Numpad3,
  Numpad4: ControlKey.Numpad4,
  Numpad5: ControlKey.Numpad5,
  Numpad6: ControlKey.Numpad6,
  Numpad7: ControlKey.Numpad7,
  Numpad8: ControlKey.Numpad8,
  Numpad9: ControlKey.Numpad9,
  NumpadDivide: ControlKey.Divide,
  NumpadMultiply: ControlKey.Multiply,
  NumpadSubtract: ControlKey.Subtract,
  NumpadAdd: ControlKey.Add,
  NumpadDecimal: ControlKey.Decimal,
}

// DOM KeyboardEvent.key → ControlKey for the remaining keys of the same match.
const BY_KEY: Readonly<Record<string, ControlKey>> = {
  Backspace: ControlKey.Backspace,
  Tab: ControlKey.Tab,
  Enter: ControlKey.Return,
  Escape: ControlKey.Escape,
  Delete: ControlKey.Delete,
  Insert: ControlKey.Insert,
  Home: ControlKey.Home,
  End: ControlKey.End,
  PageUp: ControlKey.PageUp,
  PageDown: ControlKey.PageDown,
  ArrowUp: ControlKey.UpArrow,
  ArrowDown: ControlKey.DownArrow,
  ArrowLeft: ControlKey.LeftArrow,
  ArrowRight: ControlKey.RightArrow,
  F1: ControlKey.F1,
  F2: ControlKey.F2,
  F3: ControlKey.F3,
  F4: ControlKey.F4,
  F5: ControlKey.F5,
  F6: ControlKey.F6,
  F7: ControlKey.F7,
  F8: ControlKey.F8,
  F9: ControlKey.F9,
  F10: ControlKey.F10,
  F11: ControlKey.F11,
  F12: ControlKey.F12,
  ContextMenu: ControlKey.Apps,
  Pause: ControlKey.Pause,
  PrintScreen: ControlKey.Snapshot,
  Cancel: ControlKey.Cancel,
  Clear: ControlKey.Clear,
  Help: ControlKey.Help,
  Select: ControlKey.Select,
  Execute: ControlKey.Execute,
  Standby: ControlKey.Sleep,
  KanaMode: ControlKey.Kana,
  HangulMode: ControlKey.Hangul,
  HanjaMode: ControlKey.Hanja,
  JunjaMode: ControlKey.Junja,
  FinalMode: ControlKey.Final,
  KanjiMode: ControlKey.Kanji,
  Convert: ControlKey.Convert,
}

/** Lock keys send no event of their own; their state rides on other keys as modifiers. */
export const LOCK_KEYS: ReadonlySet<string> = new Set(['CapsLock', 'NumLock', 'ScrollLock'])

export function controlKeyOf(key: string, code: string): ControlKey | null {
  return BY_CODE[code] ?? BY_KEY[key] ?? null
}

// Keys that carry the CapsLock state (rustdesk 1.5.0 src/keyboard.rs:817-858) and the NumLock
// state (src/keyboard.rs:795-815).
const CAPS_LOCK_KEYS: ReadonlySet<string> = new Set([
  ...'ABCDEFGHIJKLMNOPQRSTUVWXYZ'.split('').map((c) => `Key${c}`),
  'BracketLeft',
  'BracketRight',
  'Semicolon',
  'Quote',
  'Comma',
  'Period',
])
const NUM_LOCK_KEYS: ReadonlySet<string> = new Set([
  ...'0123456789'.split('').map((d) => `Numpad${d}`),
  'NumpadSubtract',
  'NumpadMultiply',
  'NumpadDivide',
  'NumpadAdd',
  'NumpadDecimal',
])

export function lockModifiers(code: string, capsLock: boolean, numLock: boolean): ControlKey[] {
  const keys: ControlKey[] = []
  if (capsLock && CAPS_LOCK_KEYS.has(code)) keys.push(ControlKey.CapsLock)
  if (numLock && NUM_LOCK_KEYS.has(code)) keys.push(ControlKey.NumLock)
  return keys
}

export function isNumpadKey(code: string): boolean {
  return code.startsWith('Numpad')
}

// The character of a key by position when the layout gives none we can send
// (rustdesk 1.5.0 src/keyboard.rs:1172-1224).
const BY_POSITION: Readonly<Record<string, string>> = {
  ...Object.fromEntries('0123456789'.split('').map((d) => [`Digit${d}`, d])),
  ...Object.fromEntries(
    'abcdefghijklmnopqrstuvwxyz'.split('').map((c) => [`Key${c.toUpperCase()}`, c]),
  ),
  Comma: ',',
  Period: '.',
  Semicolon: ';',
  Quote: "'",
  BracketLeft: '[',
  BracketRight: ']',
  Slash: '/',
  Backslash: '\\',
  Minus: '-',
  Equal: '=',
  Backquote: '`',
}

// The client sends the produced character only when it is one or two UTF-8 bytes, to keep CJK
// input out of this path, and '·' (Chinese layouts' backquote) as '`' (src/keyboard.rs:1154-1171).
const MAX_LEGACY_CODE_POINT = 0x7ff
const MIDDLE_DOT = 0xb7

/** The character code a legacy key event carries, or null for a key without one. */
export function legacyChar(key: string, code: string): number | null {
  const point = singleCodePoint(key)
  if (point === MIDDLE_DOT) return '`'.charCodeAt(0)
  if (point !== null && point >= 0x20 && point <= MAX_LEGACY_CODE_POINT) return point
  return BY_POSITION[code]?.charCodeAt(0) ?? null
}

/** The code point of `text` when it is exactly one, else null. */
export function singleCodePoint(text: string): number | null {
  const point = text.codePointAt(0)
  if (point === undefined) return null
  return String.fromCodePoint(point).length === text.length ? point : null
}
