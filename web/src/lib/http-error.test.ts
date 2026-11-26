import { describe, expect, it } from 'vitest'

import { httpStatusOf, isClientError } from './http-error'

describe('http-error', () => {
  it('reads a numeric status from any error-like object and nothing else', () => {
    expect(httpStatusOf({ status: 502 })).toBe(502)
    expect(httpStatusOf({ status: '502' })).toBeUndefined()
    expect(httpStatusOf(new TypeError('fetch failed'))).toBeUndefined()
    expect(httpStatusOf(null)).toBeUndefined()
  })

  it('classifies 4xx as client errors', () => {
    expect(isClientError(400)).toBe(true)
    expect(isClientError(499)).toBe(true)
    expect(isClientError(500)).toBe(false)
    expect(isClientError(399)).toBe(false)
  })
})
