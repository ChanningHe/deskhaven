import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { assignLocation } from '@/lib/browser-location'
import { USER_ME } from '@/test/fixtures'
import { callsTo, errorEnvelope } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import { account, ACCOUNT_PATH, API } from '../test-data'

vi.mock('@/lib/browser-location', () => ({ assignLocation: vi.fn() }))

const PROVIDERS = `${API}/auth/oidc/providers`
const LINK = `${API}/me/identities/link`

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

async function linkDialog(user: ReturnType<typeof renderApp>['user'], provider: string) {
  await user.click(await screen.findByRole('button', { name: `关联 ${provider}` }))
  return within(await screen.findByRole('dialog'))
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
    expect(screen.getByText('本人关联')).toBeInTheDocument()
    // Only providers without a linked identity are offered.
    expect(await screen.findByRole('button', { name: '关联 other' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '关联 corp' })).not.toBeInTheDocument()
  })

  it('offers no linking or unlinking to an account that cannot re-authenticate', async () => {
    renderApp('/me', {
      me: USER_ME,
      api: {
        [`GET ${ACCOUNT_PATH}`]: {
          body: account({ can_link_oidc: false, identities: [LINKED] }),
        },
        [`GET ${PROVIDERS}`]: { body: [{ name: 'other' }] },
      },
    })

    expect(await screen.findByText(/只有使用本地密码的账号/)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /关联 other/ })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /解除关联/ })).not.toBeInTheDocument()
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

    const dialog = await linkDialog(user, 'corp')
    await user.type(dialog.getByLabelText('当前密码'), 'secret pw')
    await user.type(dialog.getByLabelText('验证码或恢复码'), '123456')
    await user.click(dialog.getByRole('button', { name: '前往身份提供方' }))

    await waitFor(() => {
      expect(assignLocation).toHaveBeenCalledWith('https://idp.example/auth?x=1')
    })
    expect(sent).toEqual({ provider: 'corp', current_password: 'secret pw', code: '123456' })
    expect(callsTo(fetchSpy, LINK)).toBe(1)
  })

  it('asks for the password alone without TOTP and sends no code', async () => {
    let sent: unknown
    const { user } = renderApp('/me', {
      me: USER_ME,
      api: {
        [`GET ${ACCOUNT_PATH}`]: { body: account() },
        [`GET ${PROVIDERS}`]: { body: [{ name: 'corp' }] },
        [`POST ${LINK}`]: ({ init }) => {
          sent = body(init)
          return { body: { url: 'https://idp.example/auth' } }
        },
      },
    })

    const dialog = await linkDialog(user, 'corp')
    expect(dialog.queryByLabelText('验证码或恢复码')).not.toBeInTheDocument()
    await user.type(dialog.getByLabelText('当前密码'), 'secret pw')
    await user.click(dialog.getByRole('button', { name: '前往身份提供方' }))

    await waitFor(() => {
      expect(sent).toEqual({ provider: 'corp', current_password: 'secret pw', code: null })
    })
  })

  it('shows a wrong password and a refused link in the dialog', async () => {
    let calls = 0
    const { user } = renderApp('/me', {
      me: USER_ME,
      api: {
        [`GET ${ACCOUNT_PATH}`]: { body: account() },
        [`GET ${PROVIDERS}`]: { body: [{ name: 'corp' }] },
        [`POST ${LINK}`]: () => {
          calls += 1
          if (calls === 1) return { status: 422, body: errorEnvelope('wrong_password') }
          return {
            status: 403,
            body: {
              error: { code: 'oidc_failed', message: 'x', details: { reason: 'provider_linked' } },
            },
          }
        },
      },
    })

    const dialog = await linkDialog(user, 'corp')
    const submit = dialog.getByRole('button', { name: '前往身份提供方' })
    await user.type(dialog.getByLabelText('当前密码'), 'nope')
    await user.click(submit)
    expect(await dialog.findByText(/当前密码错误/)).toBeInTheDocument()
    await user.type(dialog.getByLabelText('当前密码'), 'secret pw')
    await user.click(submit)

    expect(
      await dialog.findByText('你的账号已关联了这个身份提供方的登录方式。'),
    ).toBeInTheDocument()
    expect(assignLocation).not.toHaveBeenCalledWith('https://idp.example/auth')
  })

  it('unlinks with the password, says an email link unverifies the address, and refreshes', async () => {
    let sent: unknown
    let accountCalls = 0
    const { user } = renderApp('/me', {
      me: USER_ME,
      api: {
        [`GET ${ACCOUNT_PATH}`]: () => {
          accountCalls += 1
          return { body: account({ identities: [{ ...LINKED, linked_via: 'email' }] }) }
        },
        [`GET ${PROVIDERS}`]: { body: [] },
        [`POST ${API}/me/identities/4/unlink`]: ({ init }) => {
          sent = body(init)
          return { status: 204 }
        },
      },
    })

    await user.click(await screen.findByRole('button', { name: '解除关联 Corp SSO' }))
    const dialog = within(await screen.findByRole('dialog'))
    expect(dialog.getByText(/你的邮箱将被标记为未验证/)).toBeInTheDocument()
    expect(dialog.queryByLabelText('验证码或恢复码')).not.toBeInTheDocument()
    await user.type(dialog.getByLabelText('当前密码'), 'secret pw')
    const before = accountCalls
    await user.click(dialog.getByRole('button', { name: '解除关联' }))

    await waitFor(() => {
      expect(sent).toEqual({ current_password: 'secret pw' })
    })
    await waitFor(() => {
      expect(accountCalls).toBeGreaterThan(before)
    })
  })
})
