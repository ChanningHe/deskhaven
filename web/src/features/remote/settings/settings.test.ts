import { describe, expect, it } from 'vitest'

import {
  DEFAULT_SETTINGS,
  loadSettings,
  saveSettings,
  settingsFrom,
  SETTINGS_KEY,
  withPatch,
  type RemoteSettings,
} from './settings'
import { memoryStorage } from './testing'

function storageWith(raw: string): Storage {
  const storage = memoryStorage()
  storage.setItem(SETTINGS_KEY, raw)
  return storage
}

const stored = (settings: unknown, v: unknown = 1) => JSON.stringify({ v, settings })

describe('defaults', () => {
  it('are the balanced, fit-to-window, 30 fps profile', () => {
    expect(DEFAULT_SETTINGS).toEqual({
      quality: 'balanced',
      fps: 30,
      scale: 'fit',
      showRemoteCursor: true,
      clipboardSync: true,
      audio: false,
    })
  })
})

describe('loadSettings', () => {
  it('round-trips what saveSettings wrote', () => {
    const storage = memoryStorage()
    const settings: RemoteSettings = {
      quality: { custom: 55 },
      fps: 60,
      scale: 'original',
      showRemoteCursor: false,
      clipboardSync: false,
      audio: true,
    }
    expect(saveSettings(settings, storage)).toBe(true)
    expect(JSON.parse(storage.getItem(SETTINGS_KEY) ?? '')).toEqual({ v: 1, settings })
    expect(loadSettings(storage)).toEqual(settings)
  })

  it.each([
    ['nothing stored', undefined],
    ['bad JSON', '{"v":1,'],
    ['not an object', '42'],
    ['null', 'null'],
    ['an array', '[]'],
    ['a version we do not know', stored({ fps: 60 }, 2)],
    ['a string version', stored({ fps: 60 }, '1')],
    ['no version', JSON.stringify({ settings: { fps: 60 } })],
    ['settings that are not an object', stored('x')],
    ['settings that are an array', stored([])],
    ['missing settings', JSON.stringify({ v: 1 })],
  ])('falls back to the defaults on %s', (_, raw) => {
    const storage = raw === undefined ? memoryStorage() : storageWith(raw)
    expect(loadSettings(storage)).toEqual(DEFAULT_SETTINGS)
  })

  it('falls back to the defaults when the storage throws', () => {
    const broken = {
      getItem: () => {
        throw new DOMException('denied', 'SecurityError')
      },
    } as unknown as Storage
    expect(loadSettings(broken)).toEqual(DEFAULT_SETTINGS)
  })

  it('keeps the valid fields next to an invalid one', () => {
    const storage = storageWith(stored({ fps: 61, scale: 'original', audio: true }))
    expect(loadSettings(storage)).toEqual({
      ...DEFAULT_SETTINGS,
      scale: 'original',
      audio: true,
    })
  })
})

describe('saveSettings', () => {
  it('reports a storage that refuses the write instead of throwing', () => {
    const full = {
      setItem: () => {
        throw new DOMException('quota', 'QuotaExceededError')
      },
    } as unknown as Storage
    expect(saveSettings(DEFAULT_SETTINGS, full)).toBe(false)
  })

  it('reports a missing storage', () => {
    expect(saveSettings(DEFAULT_SETTINGS, null)).toBe(false)
    expect(loadSettings(null)).toEqual(DEFAULT_SETTINGS)
  })
})

describe('settingsFrom', () => {
  const field = (name: string, value: unknown) => settingsFrom({ [name]: value })

  it.each([
    ['fps', 5, 5],
    ['fps', 60, 60],
    ['fps', 30, 30],
    ['fps', 0, 30],
    ['fps', 4, 30],
    ['fps', 61, 30],
    ['fps', 12.5, 30],
    ['fps', Number.NaN, 30],
    ['fps', Number.POSITIVE_INFINITY, 30],
    ['fps', '30', 30],
    ['fps', null, 30],
  ])('%s %o -> %o', (name, value, expected) => {
    expect(field(name, value).fps).toBe(expected)
  })

  it.each([
    ['low', 'low'],
    ['balanced', 'balanced'],
    ['best', 'best'],
    ['Best', 'balanced'],
    ['custom', 'balanced'],
    [{ custom: 10 }, { custom: 10 }],
    [{ custom: 100 }, { custom: 100 }],
    [{ custom: 9 }, 'balanced'],
    [{ custom: 101 }, 'balanced'],
    [{ custom: 50.5 }, 'balanced'],
    [{ custom: Number.NaN }, 'balanced'],
    [{ custom: '50' }, 'balanced'],
    [{}, 'balanced'],
    [{ custom: 50, extra: 1 }, { custom: 50 }],
    [50, 'balanced'],
    [null, 'balanced'],
    [['low'], 'balanced'],
  ])('quality %o -> %o', (value, expected) => {
    expect(field('quality', value).quality).toEqual(expected)
  })

  it.each([
    ['scale', 'original', 'original'],
    ['scale', 'fit', 'fit'],
    ['scale', 'stretch', 'fit'],
    ['scale', 1, 'fit'],
    ['showRemoteCursor', false, false],
    ['showRemoteCursor', 0, true],
    ['showRemoteCursor', 'false', true],
    ['clipboardSync', false, false],
    ['clipboardSync', null, true],
    ['audio', true, true],
    ['audio', 1, false],
    ['audio', 'true', false],
  ])('%s %o -> %o', (name, value, expected) => {
    expect(field(name, value)).toHaveProperty(name, expected)
  })

  it('drops unknown fields, and the settings we do not persist', () => {
    const settings = settingsFrom({ viewOnly: true, lockAfterEnd: true, other: 1, fps: 20 })
    expect(settings).toEqual({ ...DEFAULT_SETTINGS, fps: 20 })
    expect(Object.keys(settings).sort()).toEqual(Object.keys(DEFAULT_SETTINGS).sort())
  })

  it('does not let __proto__ or constructor keys reach the prototype', () => {
    const raw = JSON.stringify({
      v: 1,
      settings: {
        __proto__: { polluted: 'yes', fps: 5 },
        constructor: { prototype: { polluted: 'yes' } },
        quality: { __proto__: { custom: 20 }, constructor: 'x' },
        fps: 40,
      },
    })
    const settings = loadSettings(storageWith(raw))
    expect(settings).toEqual({ ...DEFAULT_SETTINGS, fps: 40 })
    expect(Reflect.get(Object.prototype, 'polluted')).toBeUndefined()
    expect(Reflect.get(settings, 'polluted')).toBeUndefined()
    expect(Object.getPrototypeOf(settings)).toBe(Object.prototype)
    expect(settingsFrom(JSON.parse('{"__proto__":{"fps":5}}'))).toEqual(DEFAULT_SETTINGS)
    expect(settingsFrom(JSON.parse('{"constructor":{"fps":5}}'))).toEqual(DEFAULT_SETTINGS)
  })
})

describe('withPatch', () => {
  it('goes back to 30 fps when a preset quality is picked', () => {
    const fast: RemoteSettings = { ...DEFAULT_SETTINGS, quality: { custom: 40 }, fps: 60 }
    expect(withPatch(fast, { quality: 'best' })).toEqual({ ...fast, quality: 'best', fps: 30 })
  })

  it('keeps the fps for a custom quality, and an fps that comes with the preset', () => {
    const fast: RemoteSettings = { ...DEFAULT_SETTINGS, fps: 60 }
    expect(withPatch(fast, { quality: { custom: 40 } }).fps).toBe(60)
    expect(withPatch(fast, { quality: 'low', fps: 45 }).fps).toBe(45)
  })

  it('leaves the other fields alone, and rejects an invalid value like a load would', () => {
    expect(withPatch(DEFAULT_SETTINGS, { scale: 'original' })).toEqual({
      ...DEFAULT_SETTINGS,
      scale: 'original',
    })
    const prev: RemoteSettings = { ...DEFAULT_SETTINGS, fps: 20, quality: 'low' }
    expect(withPatch(prev, { fps: 500 })).toEqual(prev)
    expect(withPatch(prev, { quality: { custom: 3 } })).toEqual(prev)
  })
})
