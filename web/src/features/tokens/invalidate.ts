import type { QueryClient } from '@tanstack/react-query'

import { getListApiTokensQueryKey } from '@/api/generated/api-tokens/api-tokens'

export function invalidateTokens(queryClient: QueryClient): Promise<void> {
  return queryClient.invalidateQueries({ queryKey: getListApiTokensQueryKey() })
}
