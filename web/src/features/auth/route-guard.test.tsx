import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { ApiError } from '@/api/api-error'
import { ADMIN_ME, OWNER_ME, USER_ME } from '@/test/fixtures'
import { callsTo, errorEnvelope } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import { hasPermission } from './permissions'
import { ME_QUERY_KEY, safeRedirectTarget } from './session'

function navLabels() {
  const nav = screen.getByRole('navigation', { name: '主导航' })
  return within(nav)
    .getAllByRole('link')
    .map((link) => link.textContent)
}

describe('console route guard', () => {
  it('sends visitors without a session to /login and remembers where they were going', async () => {
    const { router } = renderApp('/devices?page=2', { me: null })

    expect(await screen.findByRole('heading', { name: '登录 deskhaven' })).toBeInTheDocument()
    expect(router.state.location.pathname).toBe('/login')
    expect(router.state.location.search).toEqual({ redirect: '/devices?page=2' })
    expect(screen.queryByRole('navigation')).not.toBeInTheDocument()
  })

  it('shows "access denied" inside the shell when the role may not open the page', async () => {
    const { router } = renderApp('/settings', { me: ADMIN_ME })

    expect(await screen.findByRole('heading', { level: 1, name: '无权访问' })).toBeInTheDocument()
    expect(router.state.location.pathname).toBe('/settings')
    expect(navLabels()).not.toContain('设置')
    expect(navLabels()).toContain('设备')
  })

  it('lets the owner open owner-only pages', async () => {
    renderApp('/settings')

    expect(await screen.findByRole('heading', { level: 1, name: '设置' })).toBeInTheDocument()
  })

  it('sends plain users from the overview to their own page and hides admin pages', async () => {
    const { router } = renderApp('/', { me: USER_ME })

    expect(await screen.findByRole('heading', { level: 1, name: '我的' })).toBeInTheDocument()
    expect(router.state.location.pathname).toBe('/me')
    expect(navLabels()).toEqual(['我的'])
  })

  it('denies plain users every admin page', async () => {
    renderApp('/devices', { me: USER_ME })

    expect(await screen.findByRole('heading', { level: 1, name: '无权访问' })).toBeInTheDocument()
  })

  it('shows a retryable error when the session cannot be loaded', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => undefined)
    renderApp('/devices', {
      api: { 'GET /admin/api/v1/me': { status: 503, body: errorEnvelope('unavailable') } },
    })

    // 5xx is retried once (query policy, ~1 s delay) before the error shows; the
    // global asyncUtilTimeout (src/test/setup.ts) covers that.
    expect(await screen.findByRole('heading', { level: 1, name: '出错了' })).toBeInTheDocument()
    expect(screen.getByText('服务暂时不可用，请稍后重试。')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '重试' })).toBeInTheDocument()
  })

  it('forgets the session and all cached data when any request returns 401', async () => {
    const { queryClient, router } = renderApp('/devices')
    await screen.findByRole('heading', { level: 1, name: '设备' })
    expect(queryClient.getQueryData(ME_QUERY_KEY)).toBeDefined()
    queryClient.setQueryData(['users', { page: 1 }], { items: ['secret'] })

    await expect(
      queryClient.query({
        queryKey: ['devices'],
        queryFn: () => Promise.reject(new ApiError(401, { code: 'unauthorized' })),
      }),
    ).rejects.toThrow()

    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/login')
    })
    expect(router.state.location.search).toEqual({ redirect: '/devices' })
    expect(queryClient.getQueryData(ME_QUERY_KEY)).toBeUndefined()
    // Data fetched under the old session must not survive into the next one.
    expect(queryClient.getQueryData(['users', { page: 1 }])).toBeUndefined()
  })

  it('re-checks the role after any API 403, so a demoted user sees fewer pages', async () => {
    const session = { current: OWNER_ME }
    const { queryClient } = renderApp('/devices', {
      api: { 'GET /admin/api/v1/me': () => ({ body: session.current }) },
    })
    await screen.findByRole('heading', { level: 1, name: '设备' })
    expect(navLabels()).toContain('设置')

    // An owner demoted to admin elsewhere; the next owner-only call is refused.
    session.current = ADMIN_ME
    await expect(
      queryClient.query({
        queryKey: ['settings'],
        queryFn: () => Promise.reject(new ApiError(403, { code: 'forbidden' })),
      }),
    ).rejects.toThrow()

    await waitFor(() => {
      expect(navLabels()).not.toContain('设置')
    })
  })

  it('shows "access denied" when the session itself is refused, without refetch loops', async () => {
    const { fetchSpy } = renderApp('/devices', {
      api: { 'GET /admin/api/v1/me': { status: 403, body: errorEnvelope('forbidden') } },
    })

    expect(await screen.findByRole('heading', { level: 1, name: '无权访问' })).toBeInTheDocument()
    await new Promise((resolve) => setTimeout(resolve, 200))
    expect(callsTo(fetchSpy, '/admin/api/v1/me')).toBe(1)
  })

  it('shows the translated reason, not "access denied", for other 403s', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => undefined)
    renderApp('/devices', {
      api: { 'GET /admin/api/v1/me': { status: 403, body: errorEnvelope('csrf_failed') } },
    })

    expect(await screen.findByRole('heading', { level: 1, name: '出错了' })).toBeInTheDocument()
    expect(screen.getByText('请求校验失败，请刷新页面后重试。')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '重试' })).toBeInTheDocument()
    expect(screen.queryByText('无权访问')).not.toBeInTheDocument()
  })
})

describe('login redirect target', () => {
  it('accepts same-origin paths with query and hash', () => {
    expect(safeRedirectTarget('/devices?page=2')).toBe('/devices?page=2')
    expect(safeRedirectTarget('/users?q=a%20b#row-3')).toBe('/users?q=a%20b#row-3')
    // An encoded tab stays an encoded path character, still on this origin.
    expect(safeRedirectTarget('/%09/x')).toBe('/%09/x')
  })

  it('rejects anything that can leave the origin', () => {
    for (const bad of [
      '//evil.example',
      '/\\evil.example',
      // URL parsers strip TAB/CR/LF: these would resolve to https://evil.example.
      '/\t/evil.example',
      '/\n/evil.example',
      '/\r\\evil.example',
      '/\n\\evil.example',
      '/\u0000x',
      '/\u007fx',
      'https://evil.example',
      'javascript:alert(1)',
      'devices',
      '',
    ]) {
      expect(safeRedirectTarget(bad), JSON.stringify(bad)).toBeUndefined()
    }
    expect(safeRedirectTarget('/login?redirect=/x')).toBeUndefined()
    expect(safeRedirectTarget(42)).toBeUndefined()
  })

  it.each([
    ['%2F%2Fevil.example'],
    ['%2F%09%2Fevil.example'],
    ['%2F%0A%2Fevil.example'],
    ['%2F%5Cevil.example'],
    ['javascript%3Aalert(1)'],
  ])('drops an unsafe redirect from the login URL (%s)', async (encoded) => {
    const { router } = renderApp(`/login?redirect=${encoded}`, { me: null })

    await screen.findByRole('heading', { name: '登录 deskhaven' })
    // The page reads the validated search of its match, not the raw URL.
    expect(router.state.matches.at(-1)?.search).toEqual({ redirect: undefined })
  })
})

describe('hasPermission', () => {
  it('orders roles user < admin < owner', () => {
    expect(hasPermission('user', 'self')).toBe(true)
    expect(hasPermission('user', 'admin')).toBe(false)
    expect(hasPermission('admin', 'admin')).toBe(true)
    expect(hasPermission('admin', 'owner')).toBe(false)
    expect(hasPermission('owner', 'owner')).toBe(true)
  })
})
