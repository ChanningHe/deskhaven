import { hashKey } from '@tanstack/react-query'
import type { RouterHistory } from '@tanstack/react-router'

import { isApiError } from '@/api/api-error'

import { LOGIN_PATH, ME_QUERY_KEY, safeRedirectTarget } from '@/features/auth/session'

import { createQueryClient } from './query-client'
import { createAppRouter } from './router'
import type { AppRouter } from './router'

/**
 * Sends the user to /login (replacing history) unless already there, with the
 * current location as the place to come back to.
 */
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

/** Builds the query client and router, wired together. Used by main.tsx and tests. */
export function createApp(history?: RouterHistory) {
  const queryClient = createQueryClient({
    // `router` is assigned below; these only run after a request fails.
    // The session is gone: drop everything fetched under it (not just `me`), so
    // nothing leaks into the next session. Login and logout (M3-02) do the same.
    onUnauthorized: () => {
      queryClient.clear()
      void redirectToLogin(router)
    },
    // A 403 may mean the role changed (e.g. demoted elsewhere): re-check it so
    // the navigation converges. Not for `me` itself, which would loop.
    // `setup_required` (an account setup step became due, e.g. 2FA was just
    // enforced): reload `me` and re-run the guards, which go to the setup page.
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
