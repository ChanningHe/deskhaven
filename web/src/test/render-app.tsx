import { createMemoryHistory, RouterProvider } from '@tanstack/react-router'
import { render } from '@testing-library/react'
import userEvent from '@testing-library/user-event'

import type { Me } from '@/api/generated/model'
import { createApp } from '@/app/create-app'
import { AppProviders } from '@/app/providers'

import { OWNER_ME } from './fixtures'
import { errorEnvelope, mockApi } from './mock-api'
import type { MockHandler, MockResponse } from './mock-api'

export interface RenderAppOptions {
  /** The signed-in user (`GET /me`); null answers 401 (no session). Default: owner. */
  me?: Me | null
  /** Further API routes, keyed like `mockApi`. */
  api?: Record<string, MockResponse | MockHandler>
}

/**
 * Renders the full app (providers + router) at `path` with in-memory history
 * and a stubbed admin API.
 */
export function renderApp(path = '/', { me = OWNER_ME, api = {} }: RenderAppOptions = {}) {
  const fetchSpy = mockApi({
    'GET /admin/api/v1/me':
      me === null ? { status: 401, body: errorEnvelope('unauthorized') } : { body: me },
    ...api,
  })
  const { queryClient, router } = createApp(createMemoryHistory({ initialEntries: [path] }))
  const user = userEvent.setup()
  const result = render(
    <AppProviders queryClient={queryClient}>
      <RouterProvider router={router} />
    </AppProviders>,
  )
  return { ...result, queryClient, router, user, fetchSpy }
}
