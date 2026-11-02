import { createMemoryHistory, RouterProvider } from '@tanstack/react-router'
import { render } from '@testing-library/react'
import userEvent from '@testing-library/user-event'

import { createApp } from '@/app/create-app'
import { AppProviders } from '@/app/providers'

/** Renders the full app (providers + router) at `path` with in-memory history. */
export function renderApp(path = '/') {
  const { queryClient, router } = createApp(createMemoryHistory({ initialEntries: [path] }))
  const user = userEvent.setup()
  const result = render(
    <AppProviders queryClient={queryClient}>
      <RouterProvider router={router} />
    </AppProviders>,
  )
  return { ...result, queryClient, router, user }
}
