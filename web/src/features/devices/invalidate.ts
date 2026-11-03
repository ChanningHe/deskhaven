import type { QueryClient } from '@tanstack/react-query'

// Orval's query keys start with the request path.
const DEVICE_PATHS = ['/admin/api/v1/devices', '/admin/api/v1/device-groups'] as const

/**
 * Refetches every device and device group query (lists, details): moving a device changes
 * the counts of the groups, and a group change shows in the device rows.
 */
export function invalidateDevices(queryClient: QueryClient): Promise<void> {
  return queryClient.invalidateQueries({
    predicate: ({ queryKey }) => {
      const [path] = queryKey
      return typeof path === 'string' && DEVICE_PATHS.some((prefix) => path.startsWith(prefix))
    },
  })
}
