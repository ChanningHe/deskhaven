import { describe, expect, it } from 'vitest'

import { HttpError, httpStatusOf } from '@/lib/http-error'

import { ApiError, parseErrorEnvelope, parseRetryAfter } from './api-error'

describe('parseErrorEnvelope', () => {
  it('keeps only well-typed fields', () => {
    expect(parseErrorEnvelope({ error: { code: 'forbidden', message: 'no' } })).toEqual({
      code: 'forbidden',
      message: 'no',
    })
    expect(parseErrorEnvelope({ error: { code: 42, message: null, details: [] } })).toEqual({})
    expect(parseErrorEnvelope('nope')).toEqual({})
    expect(parseErrorEnvelope({ error: 'flat' })).toEqual({})
  })
})

describe('parseRetryAfter', () => {
  it('reads delta seconds, with a floor of 1', () => {
    expect(parseRetryAfter('30')).toBe(30)
    expect(parseRetryAfter('0')).toBe(1)
  })

  it('reads an HTTP date relative to now', () => {
    const now = Date.parse('2026-11-03T10:00:00Z')
    expect(parseRetryAfter('Tue, 03 Nov 2026 10:01:30 GMT', now)).toBe(90)
  })

  it('ignores absent or garbage values', () => {
    expect(parseRetryAfter(null)).toBeUndefined()
    expect(parseRetryAfter(' ')).toBeUndefined()
    expect(parseRetryAfter('soon')).toBeUndefined()
    expect(parseRetryAfter('-5')).toBeUndefined()
  })
})

describe('ApiError', () => {
  it('is a HttpError, so the query retry and 401 policies apply', () => {
    const error = new ApiError(401, { code: 'unauthorized' })
    expect(error).toBeInstanceOf(HttpError)
    expect(httpStatusOf(error)).toBe(401)
  })
})
