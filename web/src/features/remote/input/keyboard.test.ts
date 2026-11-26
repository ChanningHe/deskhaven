import { describe, expect, it } from 'vitest'

import { ControlKey, KeyboardMode, type KeyEvent } from '../proto/gen/message_pb.js'
import { KeyboardEncoder, keyboardModeFor, supportedKeyboardModes, type KeyInput } from './keyboard'
import { KEY_CODES } from './keymap'
import { NO_MODIFIERS, type Modifiers } from './mouse'

function press(key: string, code: string, extra: Partial<KeyInput> = {}): KeyInput {
  return {
    down: true,
    key,
    code,
    composing: false,
    altGraph: false,
    capsLock: false,
    numLock: false,
    modifiers: NO_MODIFIERS,
    time: 0,
    ...extra,
  }
}

function release(input: KeyInput): KeyInput {
  return { ...input, down: false }
}

function mods(m: Partial<Modifiers>): Modifiers {
  return { ...NO_MODIFIERS, ...m }
}

function brief(e: KeyEvent): Record<string, unknown> {
  return {
    mode: e.mode,
    down: e.down,
    [e.union.case ?? 'none']: e.union.value,
    modifiers: e.modifiers,
  }
}

describe('keyboard mode', () => {
  it.each([
    ['Windows', '1.5.0', KeyboardMode.Map],
    ['Linux', '1.2.0', KeyboardMode.Map],
    ['Mac OS', '1.4.5', KeyboardMode.Map],
    ['Android', '1.5.0', KeyboardMode.Translate],
    ['Linux', '1.1.9', KeyboardMode.Legacy],
  ])('%s %s defaults to %i', (platform, version, mode) => {
    expect(keyboardModeFor(platform, version)).toBe(mode)
  })

  it('offers Map only for a platform with a key table', () => {
    expect(supportedKeyboardModes('Android', '1.5.0')).toEqual([
      KeyboardMode.Legacy,
      KeyboardMode.Translate,
    ])
    expect(() => new KeyboardEncoder(KeyboardMode.Map, 'Android')).toThrow()
  })
})

describe('Map mode', () => {
  // Spot checks against rdev's keycode tables.
  it.each([
    ['KeyA', [0x1e, 38, 0]],
    ['ArrowUp', [0xe048, 111, 126]],
    ['ControlRight', [0xe01d, 105, 62]],
    ['Numpad5', [0x4c, 84, 87]],
    ['Backquote', [0x29, 49, 50]],
    ['IntlBackslash', [0x56, 94, 10]],
    ['F13', [0x64, 191, 105]],
    ['Lang1', [0xf2, 130, 104]],
  ])('%s is %o', (code, expected) => {
    expect(KEY_CODES[code]).toEqual(expected)
  })

  it.each([
    ['Windows', 0x1e],
    ['Linux', 38],
    ['Mac OS', 0],
  ])('sends the %s position code down and up', (platform, chr) => {
    const encoder = new KeyboardEncoder(KeyboardMode.Map, platform)
    const a = press('a', 'KeyA')
    expect(encoder.key(a).map(brief)).toEqual([
      { mode: KeyboardMode.Map, down: true, chr, modifiers: [] },
    ])
    expect(encoder.key(release(a)).map(brief)).toEqual([
      { mode: KeyboardMode.Map, down: false, chr, modifiers: [] },
    ])
  })

  it('adds CapsLock to letter keys and NumLock to numpad keys only', () => {
    const encoder = new KeyboardEncoder(KeyboardMode.Map, 'Linux')
    const locks = { capsLock: true, numLock: true }
    expect(encoder.key(press('A', 'KeyA', locks))[0]?.modifiers).toEqual([ControlKey.CapsLock])
    expect(encoder.key(press('1', 'Numpad1', locks))[0]?.modifiers).toEqual([ControlKey.NumLock])
    expect(encoder.key(press('1', 'Digit1', locks))[0]?.modifiers).toEqual([])
  })

  it('sends Pause to a Windows peer as a legacy control key', () => {
    const encoder = new KeyboardEncoder(KeyboardMode.Map, 'Windows')
    expect(encoder.key(press('Pause', 'Pause')).map(brief)).toEqual([
      { mode: KeyboardMode.Legacy, down: true, controlKey: ControlKey.Pause, modifiers: [] },
    ])
  })

  it('sends nothing for a key in neither table', () => {
    const encoder = new KeyboardEncoder(KeyboardMode.Map, 'Mac OS')
    expect(encoder.key(press('Unidentified', 'F24'))).toEqual([])
  })
})

describe('Translate mode', () => {
  const encoder = () => new KeyboardEncoder(KeyboardMode.Translate, 'Android')

  it('types characters as a sequence on key down only', () => {
    const e = encoder()
    const q = press('Q', 'KeyQ', { modifiers: mods({ shift: true }) })
    expect(e.key(q).map(brief)).toEqual([
      { mode: KeyboardMode.Translate, down: true, seq: 'Q', modifiers: [] },
    ])
    expect(e.key(release(q))).toEqual([])
  })

  it('sends control keys and modifiers as legacy control keys, down and up', () => {
    const e = encoder()
    const shift = press('Shift', 'ShiftRight', { modifiers: mods({ shift: true }) })
    expect(e.key(shift).map(brief)).toEqual([
      { mode: KeyboardMode.Legacy, down: true, controlKey: ControlKey.RShift, modifiers: [] },
    ])
    expect(e.key(press('Enter', 'Enter', { modifiers: mods({ shift: true }) })).map(brief)).toEqual(
      [
        {
          mode: KeyboardMode.Legacy,
          down: true,
          controlKey: ControlKey.Return,
          modifiers: [ControlKey.Shift],
        },
      ],
    )
    expect(e.key(release(shift)).map(brief)).toEqual([
      { mode: KeyboardMode.Legacy, down: false, controlKey: ControlKey.RShift, modifiers: [] },
    ])
  })

  it('sends a shortcut as a legacy character so the held modifier applies', () => {
    const e = encoder()
    const c = press('c', 'KeyC', { modifiers: mods({ ctrl: true }) })
    expect(e.key(c).map(brief)).toEqual([
      {
        mode: KeyboardMode.Legacy,
        down: true,
        chr: 'c'.codePointAt(0),
        modifiers: [ControlKey.Control],
      },
    ])
    expect(e.key(release({ ...c, modifiers: NO_MODIFIERS }))).toHaveLength(1)
  })

  it('types AltGr characters as a sequence and drops AltGr itself and dead keys', () => {
    const e = encoder()
    const altGraph = { altGraph: true, modifiers: mods({ alt: true, ctrl: true }) }
    expect(e.key(press('AltGraph', 'AltRight', altGraph))).toEqual([])
    expect(e.key(press('@', 'KeyQ', altGraph)).map(brief)).toEqual([
      { mode: KeyboardMode.Translate, down: true, seq: '@', modifiers: [] },
    ])
    expect(e.key(press('Dead', 'BracketLeft'))).toEqual([])
  })

  it('sends numpad keys as numpad control keys with the NumLock state', () => {
    expect(
      encoder()
        .key(press('7', 'Numpad7', { numLock: true }))
        .map(brief),
    ).toEqual([
      {
        mode: KeyboardMode.Legacy,
        down: true,
        controlKey: ControlKey.Numpad7,
        modifiers: [ControlKey.NumLock],
      },
    ])
  })
})

describe('Legacy mode', () => {
  const encoder = () => new KeyboardEncoder(KeyboardMode.Legacy, 'Linux')

  it.each([
    ['ArrowLeft', 'ArrowLeft', ControlKey.LeftArrow],
    ['Backspace', 'Backspace', ControlKey.Backspace],
    [' ', 'Space', ControlKey.Space],
    ['Enter', 'NumpadEnter', ControlKey.NumpadEnter],
    ['Control', 'ControlLeft', ControlKey.Control],
    ['Meta', 'MetaRight', ControlKey.RWin],
    ['F12', 'F12', ControlKey.F12],
    ['ContextMenu', 'ContextMenu', ControlKey.Apps],
    ['PrintScreen', 'PrintScreen', ControlKey.Snapshot],
  ])('%s (%s) is a control key', (key, code, controlKey) => {
    const e = encoder()
    expect([...e.key(press(key, code)), ...e.flush()][0]?.union).toEqual({
      case: 'controlKey',
      value: controlKey,
    })
  })

  it.each([
    ['a', 'KeyA', 'a'],
    ['A', 'KeyA', 'A'],
    ['é', 'Digit2', 'é'],
    ['·', 'Backquote', '`'],
    ['中', 'KeyZ', 'z'],
    ['Unidentified', 'Slash', '/'],
  ])('%s (%s) is the character %s', (key, code, chr) => {
    expect(encoder().key(press(key, code))[0]?.union).toEqual({
      case: 'chr',
      value: chr.codePointAt(0),
    })
  })

  it('lists held modifiers but not the key itself', () => {
    const all = mods({ alt: true, ctrl: true, shift: true, meta: true })
    expect(encoder().key(press('Alt', 'AltLeft', { modifiers: all }))[0]?.modifiers).toEqual([
      ControlKey.Shift,
      ControlKey.Control,
      ControlKey.Meta,
    ])
  })

  it('sends nothing for lock keys and unknown keys', () => {
    expect(encoder().key(press('CapsLock', 'CapsLock'))).toEqual([])
    expect(encoder().key(press('Unidentified', 'Lang3'))).toEqual([])
  })
})

describe('any mode', () => {
  it.each([KeyboardMode.Map, KeyboardMode.Translate, KeyboardMode.Legacy])(
    'mode %i ignores keys while an IME composes and sends its result as a sequence',
    (mode) => {
      const e = new KeyboardEncoder(mode, 'Windows')
      expect(e.key(press('Process', 'KeyN', { composing: true }))).toEqual([])
      expect(e.text('你好').map(brief)).toEqual([
        { mode: KeyboardMode.Translate, down: true, seq: '你好', modifiers: [] },
      ])
      expect(e.text('')).toEqual([])
    },
  )

  it('releases every key it holds down on the peer', () => {
    const e = new KeyboardEncoder(KeyboardMode.Map, 'Windows')
    e.key(press('Control', 'ControlLeft', { modifiers: mods({ ctrl: true }) }))
    e.key(press('a', 'KeyA', { modifiers: mods({ ctrl: true }) }))
    e.key(press('a', 'KeyA', { modifiers: mods({ ctrl: true }) }))
    expect(e.releaseAll().map((k) => [k.down, k.union.value])).toEqual([
      [false, 0x1d],
      [false, 0x1e],
    ])
    expect(e.releaseAll()).toEqual([])
    expect(e.key(release(press('a', 'KeyA')))).toEqual([])
  })
})

// Windows browsers report AltGr as a ControlLeft keydown and an AltRight keydown at the same
// time; the desktop client drops that ControlLeft.
describe('AltGr on a Windows browser', () => {
  const ctrl = (time: number) =>
    press('Control', 'ControlLeft', { time, modifiers: mods({ ctrl: true }) })
  const withAltGr = (time: number): Partial<KeyInput> => ({
    time,
    altGraph: true,
    modifiers: mods({ ctrl: true, alt: true }),
  })
  const altGr = (time: number) => press('AltGraph', 'AltRight', withAltGr(time))

  it.each([KeyboardMode.Translate, KeyboardMode.Legacy])(
    'mode %i never sends the ControlLeft that comes with AltGr',
    (mode) => {
      const e = new KeyboardEncoder(mode, 'Linux')
      expect(e.key(ctrl(100))).toEqual([])
      const sent = [...e.key(altGr(100)), ...e.key(press('@', 'KeyQ', withAltGr(101)))]
      sent.push(...e.key(release(ctrl(200))), ...e.key(release(altGr(200))))
      const controls = sent.filter((k) => k.union.case === 'controlKey')
      expect(controls.map((k) => k.union.value)).not.toContain(ControlKey.Control)
      expect(sent.flatMap((k) => k.modifiers)).not.toContain(ControlKey.Control)
    },
  )

  it('types the AltGr character as a sequence in Translate mode', () => {
    const e = new KeyboardEncoder(KeyboardMode.Translate, 'Linux')
    e.key(ctrl(100))
    e.key(altGr(100))
    expect(e.key(press('@', 'KeyQ', withAltGr(101))).map(brief)).toEqual([
      { mode: KeyboardMode.Translate, down: true, seq: '@', modifiers: [] },
    ])
  })

  it('sends a Ctrl the user pressed once the next key shows it is not AltGr', () => {
    const e = new KeyboardEncoder(KeyboardMode.Legacy, 'Linux')
    expect(e.key(ctrl(100))).toEqual([])
    const c = press('c', 'KeyC', { time: 300, modifiers: mods({ ctrl: true }) })
    expect(e.key(c).map((k) => [k.down, k.union.case, k.union.value])).toEqual([
      [true, 'controlKey', ControlKey.Control],
      [true, 'chr', 'c'.codePointAt(0)],
    ])
    e.key(ctrl(400))
    expect(e.key(release(ctrl(500))).map((k) => k.down)).toEqual([true, false])
    e.key(ctrl(600))
    expect(e.key(altGr(900)).map((k) => k.union.value)).toEqual([
      ControlKey.Control,
      ControlKey.RAlt,
    ])
  })

  it('sends a held Ctrl before other input, and forgets it when keys are let go', () => {
    const e = new KeyboardEncoder(KeyboardMode.Translate, 'Linux')
    e.key(ctrl(100))
    expect(e.flush().map((k) => k.union.value)).toEqual([ControlKey.Control])
    expect(e.flush()).toEqual([])
    expect(e.releaseAll()).toHaveLength(1)
    e.key(ctrl(200))
    expect(e.releaseAll()).toEqual([])
    expect(e.key(release(ctrl(300)))).toEqual([])
  })

  it('leaves Map mode alone, as the desktop client does', () => {
    const e = new KeyboardEncoder(KeyboardMode.Map, 'Windows')
    expect(e.key(ctrl(100)).map((k) => k.union.value)).toEqual([0x1d])
  })
})
