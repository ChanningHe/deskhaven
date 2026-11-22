import { useSuspenseQuery } from '@tanstack/react-query'

import type { Me } from '@/api/generated/model'
import { meQueryOptions } from '@/features/auth/session'

/**
 * The signed-in user, fetched again each time the calling view mounts. For views that offer
 * the browser remote: an admin may turn it on or off at any time, and `useMe` keeps its copy
 * for minutes.
 */
export function useFreshMe(): Me {
  return useSuspenseQuery({ ...meQueryOptions(), refetchOnMount: 'always' }).data
}
