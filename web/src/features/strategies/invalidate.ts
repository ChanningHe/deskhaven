import type { QueryClient } from '@tanstack/react-query'

// Orval's query keys start with the request path.
const STRATEGY_PATHS = ['/admin/api/v1/strategies', '/admin/api/v1/strategy-preview'] as const
const DEVICE_PATH = '/admin/api/v1/devices/'

/**
 * Refetches every strategy query (lists, details, assignments, previews) and the device
 * details, whose effective strategy a strategy change moves.
 */
export function invalidateStrategies(queryClient: QueryClient): Promise<void> {
  return queryClient.invalidateQueries({
    predicate: ({ queryKey }) => {
      const [path] = queryKey
      if (typeof path !== 'string') return false
      return (
        STRATEGY_PATHS.some((prefix) => path.startsWith(prefix)) || path.startsWith(DEVICE_PATH)
      )
    },
  })
}
