import { useSuspenseQuery } from '@tanstack/react-query'

import type { Me } from '@/api/generated/model'
import { meQueryOptions } from '@/features/auth/session'

/** Refetched on mount: an admin may toggle the browser remote any time; `useMe` keeps its copy for minutes. */
export function useFreshMe(): Me {
  return useSuspenseQuery({ ...meQueryOptions(), refetchOnMount: 'always' }).data
}
