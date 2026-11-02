import { createRootRouteWithContext, Outlet } from '@tanstack/react-router'

import type { RouterContext } from '@/app/router'
import { NotFoundPage } from '@/features/shell/components/not-found'

export const Route = createRootRouteWithContext<RouterContext>()({
  component: Outlet,
  // Unknown URLs that no layout claims (e.g. below /login) get a bare page.
  notFoundComponent: NotFoundPage,
})
