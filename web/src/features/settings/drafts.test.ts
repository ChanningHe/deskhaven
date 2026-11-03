import { describe, expect, it } from 'vitest'

import { discardDrafts, editSetting, NO_DRAFTS, rebase, stageReset, toChanges } from './drafts'
import { fromEditor, isSecretSet, listItems, problemOf, sameEditor, toEditor } from './editor'
import { sampleSettings } from './test-data'

const SETTINGS = sampleSettings()
function find(key: string) {
  const found = SETTINGS.find((s) => s.key === key)
  if (found === undefined) throw new Error(`no ${key}`)
  return found
}

describe('editor values', () => {
  it('start from the server value in the shape of the control', () => {
    expect(toEditor(find('auth.session_ttl'))).toBe('30d')
    expect(toEditor(find('auth.password.min_length'))).toBe('10')
    expect(toEditor(find('devices.verify_uuid'))).toBe(true)
    expect(toEditor({ ...find('signal.relay_servers'), value: ['a:1', 'b:2'] })).toBe('a:1\nb:2')
  })

  it('never start a secret from its value', () => {
    const secret = { ...find('smtp.password'), value: { set: true } }
    expect(toEditor(secret)).toBe('')
    expect(isSecretSet(secret)).toBe(true)
    expect(isSecretSet(find('smtp.password'))).toBe(false)
  })

  it('turn back into the value the API takes', () => {
    expect(fromEditor(find('auth.password.min_length'), ' 12 ')).toBe(12)
    expect(fromEditor(find('signal.relay_servers'), 'a:1, b:2\n\n c:3 ')).toEqual([
      'a:1',
      'b:2',
      'c:3',
    ])
    expect(fromEditor(find('devices.verify_uuid'), false)).toBe(false)
    expect(listItems('x:1\ny:2')).toEqual(['x:1', 'y:2'])
  })

  it('compare without caring for blanks or list layout', () => {
    expect(sameEditor(find('auth.session_ttl'), ' 30d ', '30d')).toBe(true)
    expect(sameEditor(find('signal.relay_servers'), 'a:1, b:2', 'a:1\nb:2')).toBe(true)
    expect(sameEditor(find('devices.verify_uuid'), true, false)).toBe(false)
  })
})

describe('client-side checks', () => {
  it('range-check whole numbers', () => {
    const length = find('auth.password.min_length')
    expect(problemOf(length, '12')).toBeUndefined()
    expect(problemOf(length, '7')).toEqual({ kind: 'range', min: 8, max: 256, zero: false })
    expect(problemOf(length, '')).toEqual({ kind: 'required' })
    expect(problemOf(length, '1.5')).toEqual({ kind: 'whole_number' })
    expect(problemOf(length, '-9')).toEqual({ kind: 'whole_number' })
  })

  it('accept zero where the setting allows it', () => {
    const forever = {
      ...find('auth.password.min_length'),
      schema: { type: 'u32', min: 1, max: 9, zero_allowed: true },
    } as const
    expect(problemOf(forever, '0')).toBeUndefined()
  })

  it('want a unit on durations', () => {
    const ttl = find('auth.session_ttl')
    for (const ok of ['15m', '1h30m', '7d', '2 weeks', '0'])
      expect(problemOf(ttl, ok)).toBeUndefined()
    for (const bad of ['60', '', 'soon', 'm5'])
      expect(problemOf(ttl, bad)).toEqual({ kind: 'duration' })
  })

  it('check enum variants, text length and list size', () => {
    expect(problemOf(find('auth.registration'), 'open')).toBeUndefined()
    expect(problemOf(find('auth.registration'), 'all')).toEqual({ kind: 'variant' })
    expect(problemOf(find('smtp.host'), 'x'.repeat(255))).toEqual({ kind: 'max_length', max: 254 })
    const items = Array.from({ length: 33 }, (_, i) => `h${i}:1`).join('\n')
    expect(problemOf(find('signal.relay_servers'), items)).toEqual({ kind: 'max_items', max: 32 })
  })
})

describe('drafts', () => {
  it('keep the version the edit started from and drop out when edited back', () => {
    const ttl = { ...find('auth.session_ttl'), version: 4 }
    const edited = editSetting(NO_DRAFTS, ttl, '60d')
    expect(edited['auth.session_ttl']).toEqual({
      version: 4,
      change: { kind: 'set', editor: '60d' },
    })

    // A refetch moved the setting on; the draft still answers for the version it was started from.
    const again = editSetting(edited, { ...ttl, version: 5 }, '90d')
    expect(again['auth.session_ttl']?.version).toBe(4)

    expect(editSetting(edited, ttl, '30d')).toEqual({})
  })

  it('stage a reset next to edits and list both in server order', () => {
    const min = find('auth.password.min_length')
    const registration = find('auth.registration')
    let drafts = editSetting(NO_DRAFTS, registration, 'open')
    drafts = stageReset(drafts, { ...min, version: 2 })

    expect(toChanges(drafts, SETTINGS)).toEqual([
      { key: 'auth.password.min_length', version: 2, reset: true },
      { key: 'auth.registration', version: 0, value: 'open' },
    ])
  })

  it('rebase onto newer versions and discard by key', () => {
    const ttl = find('auth.session_ttl')
    const drafts = editSetting(
      editSetting(NO_DRAFTS, ttl, '60d'),
      find('auth.registration'),
      'open',
    )

    const rebased = rebase(drafts, { 'auth.session_ttl': 7 })

    expect(rebased['auth.session_ttl']?.version).toBe(7)
    expect(rebased['auth.registration']?.version).toBe(0)
    expect(Object.keys(discardDrafts(rebased, ['auth.session_ttl']))).toEqual(['auth.registration'])
  })
})
