import { screen, waitFor } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { OWNER_ME } from '@/test/fixtures'
import { callsTo } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import { ME_QUERY_KEY } from './session'

const COMPLETE = 'POST /admin/api/v1/auth/oidc/complete'

describe('OIDC completion page', () => {
  it('completes once with the forwarded answer and drops it from the address bar', async () => {
    let sent: unknown
    const { router, fetchSpy } = renderApp('/login/oidc?state=st-1&code=co-1', {
      me: null,
      api: {
        [COMPLETE]: ({ init }) => {
          sent = JSON.parse(typeof init.body === 'string' ? init.body : 'null')
          return { body: { status: 'signed_in', user: OWNER_ME } }
        },
      },
    })

    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/')
    })
    expect(sent).toEqual({ state: 'st-1', code: 'co-1', error: null })
    expect(callsTo(fetchSpy, '/admin/api/v1/auth/oidc/complete')).toBe(1)
  })

  it('explains why a login failed', async () => {
    const { router } = renderApp('/login/oidc?state=st-1&error=access_denied', {
      me: null,
      api: {
        [COMPLETE]: {
          status: 403,
          body: { error: { code: 'oidc_failed', message: 'x', details: { reason: 'denied' } } },
        },
      },
    })

    expect(await screen.findByText('身份提供方取消或拒绝了登录。')).toBeInTheDocument()
    expect(router.state.location.search).toEqual({})
    expect(screen.getByRole('link', { name: '返回登录' })).toBeInTheDocument()
  })
})

describe('sign out', () => {
  it('revokes the session, forgets the cache and goes to the login page', async () => {
    const { user, router, queryClient, fetchSpy } = renderApp('/devices', {
      api: { 'POST /admin/api/v1/auth/logout': { status: 204 } },
    })

    await user.click(await screen.findByRole('button', { name: '账户菜单' }))
    await user.click(await screen.findByRole('menuitem', { name: '退出登录' }))

    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/login')
      expect(queryClient.getQueryData(ME_QUERY_KEY)).toBeUndefined()
    })
    expect(callsTo(fetchSpy, '/admin/api/v1/auth/logout')).toBe(1)
  })
})
