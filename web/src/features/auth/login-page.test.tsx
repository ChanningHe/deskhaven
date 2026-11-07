import { screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { assignLocation } from '@/lib/browser-location'
import { OWNER_ME, meFor } from '@/test/fixtures'
import { callsTo, errorEnvelope } from '@/test/mock-api'
import type { MockHandler } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import { ME_QUERY_KEY } from './session'

vi.mock('@/lib/browser-location', () => ({ assignLocation: vi.fn() }))

const LOGIN = 'POST /admin/api/v1/auth/login'
const TOTP = 'POST /admin/api/v1/auth/login/totp'

function bodyOf(init: RequestInit): unknown {
  return JSON.parse(typeof init.body === 'string' ? init.body : 'null')
}

async function signIn(user: ReturnType<typeof renderApp>['user'], username = 'owner') {
  await user.type(await screen.findByLabelText('用户名'), username)
  await user.type(screen.getByLabelText('密码'), 'correct horse')
  await user.click(screen.getByRole('button', { name: '登录' }))
}

describe('login page', () => {
  it('signs in with a password, drops the old cache and returns to the remembered page', async () => {
    let sent: unknown
    const login: MockHandler = ({ init }) => {
      sent = bodyOf(init)
      return { body: { status: 'signed_in', user: OWNER_ME } }
    }
    const { user, router, queryClient } = renderApp('/login?redirect=%2Fdevices', {
      me: null,
      api: { [LOGIN]: login },
    })
    queryClient.setQueryData(['users'], { items: ['stale'] })

    await signIn(user)

    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/devices')
    })
    expect(sent).toEqual({ username: 'owner', password: 'correct horse' })
    expect(queryClient.getQueryData(['users'])).toBeUndefined()
    expect(queryClient.getQueryData(ME_QUERY_KEY)).toEqual(OWNER_ME)
  })

  it('asks for a TOTP or recovery code before any session exists', async () => {
    let second: unknown
    const { user, router } = renderApp('/login', {
      me: null,
      api: {
        [LOGIN]: { body: { status: 'totp_required', challenge: 'chal-1' } },
        [TOTP]: ({ init }) => {
          second = bodyOf(init)
          return { body: { status: 'signed_in', user: OWNER_ME } }
        },
      },
    })

    await signIn(user)
    expect(await screen.findByRole('heading', { name: '两步验证' })).toBeInTheDocument()
    await user.type(screen.getByLabelText('验证码'), ' abcde-fghij ')
    await user.click(screen.getByRole('button', { name: '验证' }))

    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/')
    })
    expect(second).toEqual({ username: 'owner', challenge: 'chal-1', code: 'abcde-fghij' })
  })

  it('shows a wrong code in place and lets the user go back', async () => {
    const { user } = renderApp('/login', {
      me: null,
      api: {
        [LOGIN]: { body: { status: 'totp_required', challenge: 'chal-1' } },
        [TOTP]: { status: 422, body: errorEnvelope('invalid_code') },
      },
    })

    await signIn(user)
    await user.type(await screen.findByLabelText('验证码'), '000000')
    await user.click(screen.getByRole('button', { name: '验证' }))

    expect(await screen.findByText(/每个验证码只能使用一次/)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '返回' }))
    expect(await screen.findByRole('heading', { name: '登录 deskhaven' })).toBeInTheDocument()
  })

  it('translates wrong credentials and explains a plain-HTTP console', async () => {
    let attempt = 0
    const { user } = renderApp('/login', {
      me: null,
      api: {
        [LOGIN]: () => {
          attempt += 1
          return attempt === 1
            ? { status: 401, body: errorEnvelope('invalid_credentials') }
            : { status: 403, body: errorEnvelope('insecure_transport') }
        },
      },
    })

    await signIn(user)
    expect(await screen.findByText('用户名或密码错误。')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '登录' }))

    expect(await screen.findByText('当前连接不安全')).toBeInTheDocument()
    expect(screen.getByText(/ssh -L 21114/)).toBeInTheDocument()
  })

  it('sends a session with a pending setup step to that step', async () => {
    const restricted = { ...meFor('admin'), setup_required: 'password_change' as const }
    const { user, router } = renderApp('/login?redirect=%2Fdevices', {
      me: null,
      api: { [LOGIN]: { body: { status: 'signed_in', user: restricted } } },
    })

    await signIn(user)

    expect(await screen.findByRole('heading', { name: '设置新密码' })).toBeInTheDocument()
    expect(router.state.location.pathname).toBe('/account/password')
  })

  it('offers OIDC providers and leaves for the identity provider', async () => {
    const { user, fetchSpy } = renderApp('/login', {
      me: null,
      api: {
        'GET /admin/api/v1/auth/oidc/providers': { body: [{ name: 'corp' }] },
        'POST /admin/api/v1/auth/oidc/start': { body: { url: 'https://idp.example/auth?x=1' } },
      },
    })

    await user.click(await screen.findByRole('button', { name: '使用 corp 登录' }))

    await waitFor(() => {
      expect(assignLocation).toHaveBeenCalledWith('https://idp.example/auth?x=1')
    })
    expect(callsTo(fetchSpy, '/admin/api/v1/auth/oidc/start')).toBe(1)
  })

  it('points at the setup link while the server has no owner', async () => {
    renderApp('/login', {
      me: null,
      api: { 'GET /admin/api/v1/setup': { body: { required: true, public_url: null } } },
    })

    expect(await screen.findByText(/此服务器尚无所有者/)).toBeInTheDocument()
  })
})
