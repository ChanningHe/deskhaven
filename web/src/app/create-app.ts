import type { RouterHistory } from '@tanstack/react-router'

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

/** Builds the query client and router, wired together. Used by main.tsx and tests. */
export function createApp(history?: RouterHistory) {
  const queryClient = createQueryClient({
    // `router` is assigned below; this only runs after a request fails. The
    // session is gone, so drop the cached one: the console guard must ask again.
    onUnauthorized: () => {
      queryClient.removeQueries({ queryKey: ME_QUERY_KEY })
      void redirectToLogin(router)
    },
  })
  const router = createAppRouter(queryClient, history)
  return { queryClient, router }
}
