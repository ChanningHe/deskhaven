import type { RouterHistory } from '@tanstack/react-router'

import { createQueryClient } from './query-client'
import { createAppRouter } from './router'
import type { AppRouter } from './router'

const LOGIN_PATH = '/login'

/** Sends the user to /login (replacing history) unless already there. */
export async function redirectToLogin(router: AppRouter): Promise<void> {
  if (router.state.location.pathname === LOGIN_PATH) return
  await router.navigate({ to: LOGIN_PATH, replace: true })
}

/** Builds the query client and router, wired together. Used by main.tsx and tests. */
export function createApp(history?: RouterHistory) {
  const queryClient = createQueryClient({
    // `router` is assigned below; this only runs after a request fails.
    onUnauthorized: () => {
      void redirectToLogin(router)
    },
  })
  const router = createAppRouter(queryClient, history)
  return { queryClient, router }
}
