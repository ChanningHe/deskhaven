import { describe, expect, it } from 'vitest'

import { findMissingKeys, flattenKeys } from './locale-keys.ts'

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
