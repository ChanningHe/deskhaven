import { MutationCache, QueryCache, QueryClient } from '@tanstack/react-query'
import type { QueryKey } from '@tanstack/react-query'

import { HTTP_UNAUTHORIZED, httpStatusOf } from '@/lib/http-error'

const DEFAULT_STALE_TIME_MS = 30_000
const HTTP_FORBIDDEN = 403
const FIRST_SERVER_ERROR = 500
export const MAX_QUERY_RETRIES = 1

/**
 * Retry network errors and 5xx at most once. Anything else with a status (4xx,
 * or a 2xx whose body was not the expected JSON) fails the same way again.
 */
export function shouldRetryQuery(failureCount: number, error: unknown): boolean {
  const status = httpStatusOf(error)
  if (status !== undefined && status < FIRST_SERVER_ERROR) return false
  return failureCount < MAX_QUERY_RETRIES
}

export interface QueryClientOptions {
  /**
   * Called when any query or mutation fails with 401. The app wires this to
   * clearing the cache and a redirect to /login (see create-app.ts).
   */
  onUnauthorized?: () => void
  /**
   * Called when any query or mutation fails with 403, with the failing query's
   * key (undefined for mutations). The app re-checks the session's role.
   */
  onForbidden?: (failedQueryKey: QueryKey | undefined, error: unknown) => void
}

export function createQueryClient({
  onUnauthorized,
  onForbidden,
}: QueryClientOptions = {}): QueryClient {
  const handleError = (error: unknown, failedQueryKey: QueryKey | undefined) => {
    const status = httpStatusOf(error)
    if (status === HTTP_UNAUTHORIZED) onUnauthorized?.()
    if (status === HTTP_FORBIDDEN) onForbidden?.(failedQueryKey, error)
  }

  return new QueryClient({
    queryCache: new QueryCache({
      onError: (error, query) => {
        handleError(error, query.queryKey)
      },
    }),
    mutationCache: new MutationCache({
      onError: (error) => {
        handleError(error, undefined)
      },
    }),
    defaultOptions: {
      queries: {
        staleTime: DEFAULT_STALE_TIME_MS,
        retry: shouldRetryQuery,
        refetchOnWindowFocus: false,
      },
    },
  })
}
