import { describe, expect, it } from 'vitest'

import {
  findMissingKeys,
  findPlaceholderMismatches,
  flattenKeys,
  placeholdersOf,
} from './locale-keys.ts'

describe('placeholdersOf', () => {
  it('extracts sorted unique interpolation names, ignoring spacing and formats', () => {
    expect(placeholdersOf('{{count}} of {{ total }} ({{count, number}})')).toEqual([
      'count',
      'total',
    ])
    expect(placeholdersOf('no variables')).toEqual([])
  })
})

describe('findPlaceholderMismatches', () => {
  it('returns nothing when every locale uses the same placeholders', () => {
    const mismatches = findPlaceholderMismatches({
      en: { a: 'Hi {{name}}' },
      'zh-CN': { a: '你好，{{ name }}' },
    })

    expect(mismatches).toEqual([])
  })

  it('reports keys whose placeholder sets differ between locales', () => {
    const mismatches = findPlaceholderMismatches({
      en: { a: '{{count}} devices', b: 'ok' },
      'zh-CN': { a: '{{num}} 台设备', b: '好' },
    })

    expect(mismatches).toEqual([{ key: 'a', placeholders: { en: ['count'], 'zh-CN': ['num'] } }])
  })

  it('skips keys missing from a locale (reported by findMissingKeys instead)', () => {
    const mismatches = findPlaceholderMismatches({
      en: { a: '{{x}}', onlyEn: '{{y}}' },
      'zh-CN': { a: '{{x}}' },
    })

    expect(mismatches).toEqual([])
  })
})

describe('flattenKeys', () => {
  it('returns dotted paths for nested string leaves', () => {
    const keys = flattenKeys({ a: 'x', b: { c: 'y', d: { e: 'z' } } })

    expect(keys).toEqual(['a', 'b.c', 'b.d.e'])
  })

  it('throws on non-string leaves', () => {
    expect(() => flattenKeys({ a: { b: 1 } })).toThrow(/a\.b/)
    expect(() => flattenKeys({ a: ['x'] })).toThrow(/a/)
    expect(() => flattenKeys({ a: null })).toThrow(/a/)
  })

  it('throws when the root is not an object', () => {
    expect(() => flattenKeys('nope')).toThrow()
  })
})

describe('findMissingKeys', () => {
  it('returns no problems when all locales share the same keys', () => {
    const problems = findMissingKeys({
      en: { nav: { devices: 'Devices' } },
      'zh-CN': { nav: { devices: '设备' } },
    })

    expect(problems).toEqual([])
  })

  it('reports a key present in one locale but missing in another', () => {
    const problems = findMissingKeys({
      en: { nav: { devices: 'Devices', users: 'Users' } },
      'zh-CN': { nav: { devices: '设备' } },
    })

    expect(problems).toEqual([{ locale: 'zh-CN', missing: ['nav.users'] }])
  })

  it('reports missing keys in both directions', () => {
    const problems = findMissingKeys({
      en: { a: 'A', onlyEn: 'x' },
      'zh-CN': { a: 'A', onlyZh: 'y' },
    })

    expect(problems).toEqual([
      { locale: 'en', missing: ['onlyZh'] },
      { locale: 'zh-CN', missing: ['onlyEn'] },
    ])
  })

  it('treats a string-vs-object shape mismatch as missing keys', () => {
    const problems = findMissingKeys({
      en: { a: { b: 'x' } },
      'zh-CN': { a: 'x' },
    })

    expect(problems).toEqual([
      { locale: 'en', missing: ['a'] },
      { locale: 'zh-CN', missing: ['a.b'] },
    ])
  })
})
