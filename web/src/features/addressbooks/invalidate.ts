import type { QueryClient } from '@tanstack/react-query'

// Orval's query keys start with the request path.
const BOOKS_PATH = '/admin/api/v1/address-books'

/**
 * Refetches every address book query (list, details, entries, tags, grants): the counts in
 * the list and the drawer depend on each other.
 */
export function invalidateAddressBooks(queryClient: QueryClient): Promise<void> {
  return queryClient.invalidateQueries({
    predicate: ({ queryKey }) => {
      const [path] = queryKey
      return typeof path === 'string' && path.startsWith(BOOKS_PATH)
    },
  })
}
