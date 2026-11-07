import { screen, waitFor } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { USER_ME } from '@/test/fixtures'
import type { MockResponse } from '@/test/mock-api'
import { callsTo, errorEnvelope } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import { account, ACCOUNT_PATH, API } from '../test-data'

const VERIFY = `${API}/me/identities/link/verify`
const COMPLETE = `${API}/me/identities/link/complete`
const PAGE = '/me/oidc-link?state=st-1&code=co-1'

const SHOWN = {
  provider: 'Corp SSO',
  username: 'alice',
  subject: 'sub-1',
  email: 'alice@corp.example',
  preferred_username: 'alice.corp',
}

const LINKED = {
  id: 4,
  provider: 'corp',
  provider_display_name: 'Corp SSO',
  subject: 'sub-1',
  email: 'alice@corp.example',
  linked_via: 'explicit' as const,
  created_at: 1_800_000_000_000,
}

function refused(reason: string): MockResponse {
  return {
    status: 403,
    body: { error: { code: 'oidc_failed', message: 'x', details: { reason } } },
  }
}

function body(init: RequestInit): unknown {
  return JSON.parse(typeof init.body === 'string' ? init.body : 'null')
}

const REASONS: Record<string, string> = {
  denied: '已在身份提供方取消或拒绝登录。',
  unavailable: '身份提供方暂时不可用，请稍后再试。',
  invalid_token: '无法验证身份提供方的应答。',
  identity_taken: '这个身份已关联到其他账号。',
  provider_linked: '你的账号已关联了这个身份提供方的登录方式。',
  directory_account: '目录（LDAP）账号不能关联登录方式。',
  account_inactive: '你的账号已不可用。',
  expired: '此链接已过期，或在其他浏览器或账号中打开。请从“我的”重新开始。',
}

describe('Link confirmation page', () => {
  it('names the IdP identity and the account, then links on confirmation', async () => {
    let verified: unknown
    let confirmed: unknown
    const { user, router } = renderApp(PAGE, {
      me: USER_ME,
      api: {
        [`POST ${VERIFY}`]: ({ init }) => {
          verified = body(init)
          return { body: SHOWN }
        },
        [`POST ${COMPLETE}`]: ({ init }) => {
          confirmed = body(init)
          return { body: LINKED }
        },
        [`GET ${ACCOUNT_PATH}`]: { body: account({ identities: [LINKED] }) },
      },
    })

    expect(await screen.findByText(/将你的 Corp SSO 身份关联到账号 alice/)).toBeInTheDocument()
    expect(screen.getByText('alice@corp.example')).toBeInTheDocument()
    expect(screen.getByText('sub-1')).toBeInTheDocument()
    expect(screen.getByText('alice.corp')).toBeInTheDocument()
    expect(router.state.location.search).toEqual({})
    expect(screen.queryByRole('link', { name: '返回我的账号' })).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '关联' }))

    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/me')
    })
    expect(verified).toEqual({ state: 'st-1', code: 'co-1', error: null })
    expect(confirmed).toEqual({ state: 'st-1' })
  })

  it('shows a loading state while verifying', async () => {
    renderApp(PAGE, {
      me: USER_ME,
      api: { [`POST ${VERIFY}`]: () => new Promise<MockResponse>(() => undefined) },
    })

    expect(await screen.findByText(/正在验证/)).toHaveAttribute('role', 'status')
  })

  it('confirms once on a double click', async () => {
    const { user, fetchSpy } = renderApp(PAGE, {
      me: USER_ME,
      api: {
        [`POST ${VERIFY}`]: { body: SHOWN },
        [`POST ${COMPLETE}`]: () => new Promise<MockResponse>(() => undefined),
      },
    })

    const confirm = await screen.findByRole('button', { name: '关联' })
    await user.dblClick(confirm)

    await waitFor(() => {
      expect(confirm).toBeDisabled()
    })
    expect(confirm).toHaveAttribute('aria-busy', 'true')
    expect(callsTo(fetchSpy, COMPLETE)).toBe(1)
  })

  it('says the link is gone without a state, and asks nothing', async () => {
    const { fetchSpy } = renderApp('/me/oidc-link', { me: USER_ME, api: {} })

    expect(await screen.findByText(REASONS.expired ?? '')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: '返回我的账号' })).toBeInTheDocument()
    expect(callsTo(fetchSpy, VERIFY)).toBe(0)
  })

  it('shows a refusal at the identity provider directly, without verifying', async () => {
    const { fetchSpy } = renderApp('/me/oidc-link?state=st-1&error=access_denied', {
      me: USER_ME,
      api: {},
    })

    expect(await screen.findByText(REASONS.denied ?? '')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: '返回我的账号' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '关联' })).not.toBeInTheDocument()
    expect(callsTo(fetchSpy, VERIFY)).toBe(0)
  })

  it.each(Object.keys(REASONS))('explains a %s refusal at verification', async (reason) => {
    renderApp(PAGE, { me: USER_ME, api: { [`POST ${VERIFY}`]: refused(reason) } })

    expect(await screen.findByText(REASONS[reason] ?? '')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: '返回我的账号' })).toBeInTheDocument()
  })

  it.each(['identity_taken', 'provider_linked', 'account_inactive'])(
    'explains a %s refusal at confirmation',
    async (reason) => {
      const { user } = renderApp(PAGE, {
        me: USER_ME,
        api: { [`POST ${VERIFY}`]: { body: SHOWN }, [`POST ${COMPLETE}`]: refused(reason) },
      })

      await user.click(await screen.findByRole('button', { name: '关联' }))

      expect(await screen.findByText(REASONS[reason] ?? '')).toBeInTheDocument()
      expect(screen.getByRole('button', { name: '关联' })).toBeDisabled()
    },
  )

  it('falls back to the generic message for other errors', async () => {
    renderApp(PAGE, {
      me: USER_ME,
      api: { [`POST ${VERIFY}`]: { status: 500, body: errorEnvelope('internal') } },
    })

    expect(await screen.findByRole('alert')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: '返回我的账号' })).toBeInTheDocument()
  })
})
