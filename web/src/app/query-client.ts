import { MutationCache, QueryCache, QueryClient } from '@tanstack/react-query'
import type { QueryKey } from '@tanstack/react-query'

import { HTTP_UNAUTHORIZED, httpStatusOf } from '@/lib/http-error'

const DEFAULT_STALE_TIME_MS = 30_000
const HTTP_FORBIDDEN = 403
const FIRST_SERVER_ERROR = 500
export const MAX_QUERY_RETRIES = 1

/** Network errors and 5xx retry once; any other status would fail the same way again. */
export function shouldRetryQuery(failureCount: number, error: unknown): boolean {
  const status = httpStatusOf(error)
  if (status !== undefined && status < FIRST_SERVER_ERROR) return false
  return failureCount < MAX_QUERY_RETRIES
}

export interface QueryClientOptions {
  onUnauthorized?: () => void
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
