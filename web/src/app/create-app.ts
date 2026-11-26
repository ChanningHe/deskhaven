import { hashKey } from '@tanstack/react-query'
import type { RouterHistory } from '@tanstack/react-router'

import { isApiError } from '@/api/api-error'

import { LOGIN_PATH, ME_QUERY_KEY, safeRedirectTarget } from '@/features/auth/session'

import { createQueryClient } from './query-client'
import { createAppRouter } from './router'
import type { AppRouter } from './router'

export async function redirectToLogin(router: AppRouter): Promise<void> {
  const { pathname, href } = router.state.location
  if (pathname === LOGIN_PATH) return
  const redirect = safeRedirectTarget(href)
  await router.navigate({
    to: LOGIN_PATH,
    search: redirect === undefined ? {} : { redirect },
    replace: true,
  })
}

const ME_KEY_HASH = hashKey(ME_QUERY_KEY)

export function createApp(history?: RouterHistory) {
  const queryClient = createQueryClient({
    // `router` is assigned below; these only run after a request fails. Drop everything fetched
    // under the session, not just `me`, so nothing leaks into the next one.
    onUnauthorized: () => {
      queryClient.clear()
      void redirectToLogin(router)
    },
    // A 403 may mean the role changed: re-check `me` (never on `me` itself, which would loop).
    // `setup_required` also re-runs the guards, which go to the setup page.
    onForbidden: (failedQueryKey, error) => {
      if (failedQueryKey !== undefined && hashKey(failedQueryKey) === ME_KEY_HASH) return
      const reloaded = queryClient.invalidateQueries({ queryKey: ME_QUERY_KEY })
      if (isApiError(error) && error.code === 'setup_required') {
        void reloaded.then(() => router.invalidate())
      }
    },
  })
  const router = createAppRouter(queryClient, history)
  return { queryClient, router }
}
