import { describe, expect, it } from 'vitest'

import { needsCsrfHeader, readCsrfToken } from './csrf'

describe('readCsrfToken', () => {
  it('finds the CSRF cookie among others', () => {
    expect(readCsrfToken('a=1; __Host-deskhaven_csrf=abc-_123; b=2')).toBe('abc-_123')
    expect(readCsrfToken('__Host-deskhaven_csrf=abc')).toBe('abc')
  })

  it('ignores missing, empty and look-alike cookies', () => {
    expect(readCsrfToken('')).toBeUndefined()
    expect(readCsrfToken('__Host-deskhaven_csrf=')).toBeUndefined()
    expect(readCsrfToken('x__Host-deskhaven_csrf=evil; deskhaven_csrf=evil')).toBeUndefined()
  })
})

describe('needsCsrfHeader', () => {
  it('is false for safe methods only', () => {
    for (const method of ['GET', 'head', 'OPTIONS']) expect(needsCsrfHeader(method)).toBe(false)
    for (const method of ['POST', 'put', 'PATCH', 'DELETE'])
      expect(needsCsrfHeader(method)).toBe(true)
  })
})
