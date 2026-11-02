import { describe, expect, it, vi } from 'vitest'

import { errorEnvelope, mockApi } from '@/test/mock-api'

import { ApiError } from './api-error'
import { apiFetch } from './client'
import { ANONYMOUS_CSRF_TOKEN, CSRF_HEADER } from './csrf'

function sentHeaders(spy: ReturnType<typeof mockApi>, index = 0): Headers {
  const init = spy.mock.calls[index]?.[1]
  return new Headers(init?.headers)
}

function stubCookie(value: string) {
  vi.spyOn(document, 'cookie', 'get').mockReturnValue(value)
}

describe('apiFetch', () => {
  it('returns the parsed body of a GET without a CSRF header', async () => {
    const spy = mockApi({ 'GET /admin/api/v1/me': { body: { id: 1 } } })
    stubCookie('__Host-deskhaven_csrf=token-1')

    await expect(apiFetch('/admin/api/v1/me')).resolves.toEqual({ id: 1 })

    expect(sentHeaders(spy).has(CSRF_HEADER)).toBe(false)
    expect(sentHeaders(spy).get('Accept')).toBe('application/json')
    expect(spy.mock.calls[0]?.[1]?.credentials).toBe('same-origin')
  })

  it('echoes the CSRF cookie in the header of state-changing requests', async () => {
    const spy = mockApi({
      'POST /admin/api/v1/things': { status: 201, body: { ok: true } },
      'DELETE /admin/api/v1/things': { status: 204 },
    })
    stubCookie('theme=dark; __Host-deskhaven_csrf=token-1; other=x')

    await apiFetch('/admin/api/v1/things', { method: 'POST', body: '{}' })
    await expect(apiFetch('/admin/api/v1/things', { method: 'delete' })).resolves.toBeUndefined()

    expect(sentHeaders(spy, 0).get(CSRF_HEADER)).toBe('token-1')
    expect(sentHeaders(spy, 0).get('Content-Type')).toBe('application/json')
    expect(sentHeaders(spy, 1).get(CSRF_HEADER)).toBe('token-1')
    expect(spy.mock.calls[1]?.[1]?.method).toBe('DELETE')
  })

  it('sends a non-empty placeholder when there is no CSRF cookie yet', async () => {
    const spy = mockApi({ 'POST /admin/api/v1/auth/login': { body: {} } })
    stubCookie('')

    await apiFetch('/admin/api/v1/auth/login', { method: 'POST', body: '{}' })

    expect(sentHeaders(spy).get(CSRF_HEADER)).toBe(ANONYMOUS_CSRF_TOKEN)
  })

  it('throws ApiError parsed from the error envelope', async () => {
    mockApi({
      'PATCH /admin/api/v1/x': {
        status: 422,
        body: {
          error: {
            code: 'validation_failed',
            message: 'bad',
            details: { fields: { name: ['required'] } },
          },
        },
      },
    })

    const error = await apiFetch('/admin/api/v1/x', { method: 'PATCH' }).catch((e: unknown) => e)

    expect(error).toBeInstanceOf(ApiError)
    expect(error).toMatchObject({
      status: 422,
      code: 'validation_failed',
      message: 'bad',
      details: { fields: { name: ['required'] } },
      retryAfterSeconds: undefined,
    })
  })

  it('surfaces Retry-After on 429', async () => {
    mockApi({
      'GET /admin/api/v1/me': {
        status: 429,
        body: errorEnvelope('rate_limited'),
        headers: { 'Retry-After': '17' },
      },
    })

    await expect(apiFetch('/admin/api/v1/me')).rejects.toMatchObject({
      status: 429,
      code: 'rate_limited',
      retryAfterSeconds: 17,
    })
  })

  it('keeps the status but no code for unknown codes and non-JSON bodies', async () => {
    mockApi({ 'GET /a': { status: 409, body: errorEnvelope('brand_new_code') } })
    await expect(apiFetch('/a')).rejects.toMatchObject({ status: 409, code: undefined })

    vi.stubGlobal(
      'fetch',
      vi.fn(() => Promise.resolve(new Response('<html>Bad gateway</html>', { status: 502 }))),
    )
    await expect(apiFetch('/a')).rejects.toMatchObject({
      name: 'ApiError',
      status: 502,
      code: undefined,
      message: 'HTTP 502',
    })
  })

  it('lets network failures through unchanged (no status)', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(() => Promise.reject(new TypeError('Failed to fetch'))),
    )

    await expect(apiFetch('/a')).rejects.toThrow(TypeError)
  })
})
