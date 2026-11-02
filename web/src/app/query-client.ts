import { MutationCache, QueryCache, QueryClient } from '@tanstack/react-query'

import { HTTP_UNAUTHORIZED, httpStatusOf, isClientError } from '@/lib/http-error'

const DEFAULT_STALE_TIME_MS = 30_000
export const MAX_QUERY_RETRIES = 1

/**
 * Never retry 4xx (the request itself is wrong, or auth is missing); retry
 * network errors and 5xx at most once.
 */
export function shouldRetryQuery(failureCount: number, error: unknown): boolean {
  const status = httpStatusOf(error)
  if (status !== undefined && isClientError(status)) return false
  return failureCount < MAX_QUERY_RETRIES
}

export interface QueryClientOptions {
  /**
   * Called when any query or mutation fails with 401. The app wires this to a
   * redirect to /login (see create-app.ts); it is a no-op when omitted.
   */
  onUnauthorized?: () => void
}

export function createQueryClient({ onUnauthorized }: QueryClientOptions = {}): QueryClient {
  const handleError = (error: unknown) => {
    if (httpStatusOf(error) === HTTP_UNAUTHORIZED) onUnauthorized?.()
  }

  return new QueryClient({
    queryCache: new QueryCache({ onError: handleError }),
    mutationCache: new MutationCache({ onError: handleError }),
    defaultOptions: {
      queries: {
        staleTime: DEFAULT_STALE_TIME_MS,
        retry: shouldRetryQuery,
        refetchOnWindowFocus: false,
      },
    },
  })
}
