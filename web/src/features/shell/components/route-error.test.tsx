import { QueryClientProvider } from '@tanstack/react-query'
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
} from '@tanstack/react-router'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'

import { createQueryClient } from '@/app/query-client'
import { ROUTER_DEFAULTS } from '@/app/router'

// A throwaway route tree using the app's router defaults, with a page that
// throws until `failing.current` is cleared.
function renderThrowingRoute() {
  const failing = { current: true }
  function Flaky() {
    if (failing.current) throw new Error('boom')
    return <h1>Recovered</h1>
  }
  const rootRoute = createRootRoute({ component: Outlet })
  const routeTree = rootRoute.addChildren([
    createRoute({ getParentRoute: () => rootRoute, path: '/', component: Flaky }),
  ])
  const router = createRouter({
    routeTree,
    ...ROUTER_DEFAULTS,
    history: createMemoryHistory({ initialEntries: ['/'] }),
  })
  render(
    <QueryClientProvider client={createQueryClient()}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  )
  return failing
}

describe('default route error component', () => {
  it('renders a translated error state for a throwing route and recovers on retry', async () => {
    // React reports caught render errors to console.error; keep test output clean.
    vi.spyOn(console, 'error').mockImplementation(() => undefined)
    const failing = renderThrowingRoute()

    expect(await screen.findByRole('alert')).toBeInTheDocument()
    expect(screen.getByRole('heading', { level: 1, name: '出错了' })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: '返回概览' })).toHaveAttribute('href', '/')
    expect(screen.queryByText('boom')).not.toBeInTheDocument()

    failing.current = false
    await userEvent.click(screen.getByRole('button', { name: '重试' }))

    expect(await screen.findByRole('heading', { name: 'Recovered' })).toBeInTheDocument()
  })
})
