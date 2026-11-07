import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { assignLocation } from '@/lib/browser-location'
import { USER_ME } from '@/test/fixtures'
import { callsTo } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import { account, ACCOUNT_PATH, API } from '../test-data'

vi.mock('@/lib/browser-location', () => ({ assignLocation: vi.fn() }))

const PROVIDERS = `${API}/auth/oidc/providers`
const LINK = `${API}/me/identities/link`
const PREVIEW = `${API}/me/identities/link/preview`
const COMPLETE = `${API}/me/identities/link/complete`

const LINKED = {
  id: 4,
  provider: 'corp',
  provider_display_name: 'Corp SSO',
  subject: 'sub-1',
  email: 'alice@corp.example',
  linked_via: 'explicit' as const,
  created_at: 1_800_000_000_000,
}

function body(init: RequestInit): unknown {
  return JSON.parse(typeof init.body === 'string' ? init.body : 'null')
}

describe('My account: linked sign-ins', () => {
  it('lists the linked identities with the standing warning', async () => {
    renderApp('/me', {
      me: USER_ME,
      api: {
        [`GET ${ACCOUNT_PATH}`]: { body: account({ identities: [LINKED] }) },
        [`GET ${PROVIDERS}`]: { body: [{ name: 'corp' }, { name: 'other' }] },
      },
    })

    expect(await screen.findByText('Corp SSO')).toBeInTheDocument()
    expect(screen.getByText('alice@corp.example')).toBeInTheDocument()
    expect(screen.getByText(/不再需要本账号的两步验证码/)).toBeInTheDocument()
    // Only providers without a linked identity are offered.
    expect(await screen.findByRole('button', { name: '关联 other' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '关联 corp' })).not.toBeInTheDocument()
  })

  it('re-authenticates with password and code, then leaves for the identity provider', async () => {
    let sent: unknown
    const { user, fetchSpy } = renderApp('/me', {
      me: USER_ME,
      api: {
        [`GET ${ACCOUNT_PATH}`]: { body: account({ totp_enabled: true }) },
        [`GET ${PROVIDERS}`]: { body: [{ name: 'corp' }] },
        [`POST ${LINK}`]: ({ init }) => {
          sent = body(init)
          return { body: { url: 'https://idp.example/auth?x=1' } }
        },
      },
    })

    await user.click(await screen.findByRole('button', { name: '关联 corp' }))
    const dialog = within(await screen.findByRole('dialog'))
    await user.type(dialog.getByLabelText('当前密码'), 'secret pw')
    await user.type(dialog.getByLabelText('验证码或恢复码'), '123456')
    await user.click(dialog.getByRole('button', { name: '前往身份提供方' }))

    await waitFor(() => {
      expect(assignLocation).toHaveBeenCalledWith('https://idp.example/auth?x=1')
    })
    expect(sent).toEqual({ provider: 'corp', current_password: 'secret pw', code: '123456' })
    expect(callsTo(fetchSpy, LINK)).toBe(1)
  })

  it('unlinks with the password', async () => {
    let sent: unknown
    const { user } = renderApp('/me', {
      me: USER_ME,
      api: {
        [`GET ${ACCOUNT_PATH}`]: { body: account({ identities: [LINKED] }) },
        [`GET ${PROVIDERS}`]: { body: [] },
        [`POST ${API}/me/identities/4/unlink`]: ({ init }) => {
          sent = body(init)
          return { status: 204 }
        },
      },
    })

    await user.click(await screen.findByRole('button', { name: '解除关联' }))
    const dialog = within(await screen.findByRole('dialog'))
    await user.type(dialog.getByLabelText('当前密码'), 'secret pw')
    expect(dialog.queryByLabelText('验证码或恢复码')).not.toBeInTheDocument()
    await user.click(dialog.getByRole('button', { name: '解除关联' }))

    await waitFor(() => {
      expect(sent).toEqual({ current_password: 'secret pw' })
    })
  })
})

describe('My account: unlinking an email link', () => {
  it('says the email will be marked unverified', async () => {
    const { user } = renderApp('/me', {
      me: USER_ME,
      api: {
        [`GET ${ACCOUNT_PATH}`]: {
          body: account({ identities: [{ ...LINKED, linked_via: 'email' }] }),
        },
        [`GET ${PROVIDERS}`]: { body: [] },
      },
    })

    await user.click(await screen.findByRole('button', { name: '解除关联' }))

    const dialog = within(await screen.findByRole('dialog'))
    expect(dialog.getByText(/你的邮箱将被标记为未验证/)).toBeInTheDocument()
  })
})

describe('Link confirmation page', () => {
  it('names the provider and the account, then links on confirmation', async () => {
    let sent: unknown
    const { user, router } = renderApp('/me/oidc-link?state=st-1&code=co-1', {
      me: USER_ME,
      api: {
        [`GET ${PREVIEW}`]: { body: { provider: 'Corp SSO', username: 'alice' } },
        [`POST ${COMPLETE}`]: ({ init }) => {
          sent = body(init)
          return { body: LINKED }
        },
        [`GET ${ACCOUNT_PATH}`]: { body: account({ identities: [LINKED] }) },
      },
    })

    expect(await screen.findByText(/将你的 Corp SSO 身份关联到账号 alice/)).toBeInTheDocument()
    expect(router.state.location.search).toEqual({})
    await user.click(screen.getByRole('button', { name: '关联' }))

    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/me')
    })
    expect(sent).toEqual({ state: 'st-1', code: 'co-1', error: null })
  })

  it('explains a refused link', async () => {
    const { user } = renderApp('/me/oidc-link?state=st-1&code=co-1', {
      me: USER_ME,
      api: {
        [`GET ${PREVIEW}`]: { body: { provider: 'Corp SSO', username: 'alice' } },
        [`POST ${COMPLETE}`]: {
          status: 403,
          body: {
            error: { code: 'oidc_failed', message: 'x', details: { reason: 'identity_taken' } },
          },
        },
      },
    })

    await user.click(await screen.findByRole('button', { name: '关联' }))

    expect(await screen.findByText('这个身份已关联到其他账号。')).toBeInTheDocument()
  })
})
