import type { QueryClient } from '@tanstack/react-query'

import { getGetMyAccountQueryKey, getListMySessionsQueryKey } from '@/api/generated/me/me'
import type { Me } from '@/api/generated/model'
import { ME_QUERY_KEY } from '@/features/auth/session'

/**
 * The account changed: refetch the account page's data and the session list (a password
 * change or a TOTP enrolment replaces the console session). `me`, when the response carried
 * it, is the newest copy of the shell's user.
 */
export async function refreshAccount(queryClient: QueryClient, me?: Me): Promise<void> {
  if (me !== undefined) queryClient.setQueryData(ME_QUERY_KEY, me)
  await Promise.all([
    queryClient.invalidateQueries({ queryKey: getGetMyAccountQueryKey() }),
    queryClient.invalidateQueries({ queryKey: getListMySessionsQueryKey() }),
  ])
}
