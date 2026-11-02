import { describe, expect, it, vi } from 'vitest'

import { HttpError } from '@/lib/http-error'

import { createQueryClient, MAX_QUERY_RETRIES, shouldRetryQuery } from './query-client'

describe('shouldRetryQuery', () => {
  it('never retries client errors (4xx)', () => {
    for (const status of [400, 401, 403, 404, 409, 422, 429]) {
      expect(shouldRetryQuery(0, new HttpError(status))).toBe(false)
    }
  })

  it('retries server errors and network failures at most once', () => {
    for (const error of [new HttpError(500), new HttpError(503), new TypeError('fetch failed')]) {
      expect(shouldRetryQuery(0, error)).toBe(true)
      expect(shouldRetryQuery(MAX_QUERY_RETRIES, error)).toBe(false)
    }
  })
})

describe('createQueryClient onUnauthorized hook', () => {
  const failingQuery = (error: Error) => ({
    queryKey: ['test', error.message],
    queryFn: () => Promise.reject(error),
    retry: false,
  })

  it('fires when a query fails with 401', async () => {
    const onUnauthorized = vi.fn()
    const client = createQueryClient({ onUnauthorized })

    await expect(client.query(failingQuery(new HttpError(401)))).rejects.toThrow()

    expect(onUnauthorized).toHaveBeenCalledTimes(1)
  })

  it('fires when a mutation fails with 401', async () => {
    const onUnauthorized = vi.fn()
    const client = createQueryClient({ onUnauthorized })
    const mutation = client.getMutationCache().build(client, {
      mutationFn: () => Promise.reject(new HttpError(401)),
    })

    await expect(mutation.execute(undefined)).rejects.toThrow()

    expect(onUnauthorized).toHaveBeenCalledTimes(1)
  })

  it('ignores other failures', async () => {
    const onUnauthorized = vi.fn()
    const client = createQueryClient({ onUnauthorized })

    await expect(client.query(failingQuery(new HttpError(403)))).rejects.toThrow()
    await expect(client.query(failingQuery(new HttpError(500)))).rejects.toThrow()

    expect(onUnauthorized).not.toHaveBeenCalled()
  })

  it('is safe without a handler', async () => {
    const client = createQueryClient()

    await expect(client.query(failingQuery(new HttpError(401)))).rejects.toThrow('HTTP 401')
  })
})

describe('createQueryClient onForbidden hook', () => {
  it('passes the failing query key, and undefined for mutations', async () => {
    const onForbidden = vi.fn()
    const client = createQueryClient({ onForbidden })

    await expect(
      client.query({
        queryKey: ['settings'],
        queryFn: () => Promise.reject(new HttpError(403)),
        retry: false,
      }),
    ).rejects.toThrow()
    const mutation = client.getMutationCache().build(client, {
      mutationFn: () => Promise.reject(new HttpError(403)),
    })
    await expect(mutation.execute(undefined)).rejects.toThrow()

    expect(onForbidden.mock.calls).toEqual([[['settings']], [undefined]])
  })
})

describe('shouldRetryQuery for statuses below 400', () => {
  it('does not retry a 2xx that failed to parse', () => {
    expect(shouldRetryQuery(0, new HttpError(200))).toBe(false)
  })
})
