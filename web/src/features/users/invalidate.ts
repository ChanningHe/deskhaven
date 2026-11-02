import type { QueryClient } from '@tanstack/react-query'

// Orval's query keys start with the request path.
const ACCOUNT_PATHS = ['/admin/api/v1/users', '/admin/api/v1/groups'] as const

/**
 * Refetches every user and group query (lists, details, members) after a
 * change: an account change shows in group member lists and vice versa.
 */
export function invalidateAccounts(queryClient: QueryClient): Promise<void> {
  return queryClient.invalidateQueries({
    predicate: ({ queryKey }) => {
      const [path] = queryKey
      return typeof path === 'string' && ACCOUNT_PATHS.some((prefix) => path.startsWith(prefix))
    },
  })
}
