import type { QueryClient } from '@tanstack/react-query'

// Orval's query keys start with the request path.
const DEVICE_PATHS = ['/admin/api/v1/devices', '/admin/api/v1/device-groups'] as const

/** Moving a device changes the counts of the groups, and a group change shows in the rows. */
export function invalidateDevices(queryClient: QueryClient): Promise<void> {
  return queryClient.invalidateQueries({
    predicate: ({ queryKey }) => {
      const [path] = queryKey
      return typeof path === 'string' && DEVICE_PATHS.some((prefix) => path.startsWith(prefix))
    },
  })
}
