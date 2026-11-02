import { describe, expect, it } from 'vitest'

import { HttpError, httpStatusOf, isClientError } from './http-error'

describe('http-error', () => {
  it('carries the response status', () => {
    const error = new HttpError(404)

    expect(error).toBeInstanceOf(Error)
    expect(error.name).toBe('HttpError')
    expect(error.message).toBe('HTTP 404')
    expect(httpStatusOf(error)).toBe(404)
  })

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
