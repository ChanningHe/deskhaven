import { describe, expect, it } from 'vitest'

import { catalogOption, strategyView } from './test-data'
import {
  draftOf,
  KEPT,
  keyProblem,
  optionCount,
  patchOf,
  RESET,
  setTo,
  valueProblem,
} from './option-draft'

describe('option drafts', () => {
  it('reads set, reset and withheld secret options from the server copy', () => {
    const strategy = strategyView(1, 'kiosk', {
      options: { 'enable-keyboard': 'N', whitelist: '' },
      secret_options: ['proxy-password'],
    })

    expect(draftOf(strategy)).toEqual({
      'enable-keyboard': setTo('N'),
      whitelist: RESET,
      'proxy-password': KEPT,
    })
  })

  it('patches only the changed keys: values, resets and removals', () => {
    const draft = {
      a: setTo('1'),
      b: RESET,
      c: undefined,
      kept: KEPT,
      untouched: setTo('x'),
    }

    expect(patchOf(draft, ['a', 'b', 'c'])).toEqual({ a: '1', b: '', c: null })
    expect(patchOf(draft, [])).toEqual({})
  })

  it('accepts the client key charset only', () => {
    for (const good of ['enable-keyboard', 'collapse_toolbar', 'hideAbTagsPanel', '2fa', 'a.b']) {
      expect(keyProblem(good)).toBeUndefined()
    }
    expect(keyProblem('')).toBe('empty')
    expect(keyProblem('has space')).toBe('charset')
    expect(keyProblem('a/b')).toBe('charset')
    expect(keyProblem('k'.repeat(129))).toBe('tooLong')
  })

  it('checks values against the option kind and the size limits', () => {
    const bool = catalogOption('enable-keyboard', 'bool')
    const port = catalogOption('direct-access-port', 'number', { min: 1, max: 65535 })
    const mode = catalogOption('approve-mode', 'choice', {
      choices: [{ value: 'click', label: { en: 'Click', zh: '点击' } }],
    })

    expect(valueProblem(bool, 'Y')).toBeUndefined()
    expect(valueProblem(bool, 'yes')).toBe('bool')
    expect(valueProblem(mode, 'tap')).toBe('choice')
    expect(valueProblem(port, '21118')).toBeUndefined()
    expect(valueProblem(port, '0')).toBe('range')
    expect(valueProblem(port, '1.5')).toBe('number')
    expect(valueProblem(undefined, 'v'.repeat(4097))).toBe('tooLong')
    expect(valueProblem(undefined, 'line\nbreak')).toBe('control')
    expect(valueProblem(undefined, 'del\u007f')).toBe('control')
    expect(valueProblem(undefined, 'c1\u0085')).toBe('control')
    expect(valueProblem(undefined, 'ü€ ')).toBeUndefined()
    expect(valueProblem(undefined, '')).toBeUndefined()
  })

  it('counts the options a draft keeps', () => {
    expect(optionCount({ a: setTo('1'), b: RESET, c: KEPT, gone: undefined })).toBe(3)
  })
})
