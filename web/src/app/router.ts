import { createRouter } from '@tanstack/react-router'
import type { RouterHistory } from '@tanstack/react-router'
import type { QueryClient } from '@tanstack/react-query'

import { RouteError } from '@/features/shell/components/route-error'
import { RoutePending } from '@/features/shell/components/route-pending'
import { routeTree } from '@/routeTree.gen'

export interface RouterContext {
  queryClient: QueryClient
}

/** Router-wide defaults, shared with tests that build ad-hoc route trees. */
export const ROUTER_DEFAULTS = {
  defaultPreload: 'intent',
  // Loaders should read through TanStack Query, which owns caching.
  defaultPreloadStaleTime: 0,
  defaultErrorComponent: RouteError,
  defaultPendingComponent: RoutePending,
  scrollRestoration: true,
} as const

export function createAppRouter(queryClient: QueryClient, history?: RouterHistory) {
  return createRouter({
    routeTree,
    context: { queryClient },
    ...ROUTER_DEFAULTS,
    ...(history ? { history } : {}),
  })
}

export type AppRouter = ReturnType<typeof createAppRouter>

declare module '@tanstack/react-router' {
  interface Register {
    router: AppRouter
  }
}
