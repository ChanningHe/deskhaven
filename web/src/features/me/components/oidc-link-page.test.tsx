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

    expect(await screen.findByText('sub-1')).toBeInTheDocument()
    expect(screen.getByText('alice@corp.example')).toBeInTheDocument()
    expect(screen.getByText('alice.corp')).toBeInTheDocument()
    expect(router.state.location.search).toEqual({})
    await user.click(screen.getByRole('button', { name: '关联' }))

    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/me')
    })
    expect(verified).toEqual({ state: 'st-1', code: 'co-1', error: null })
    expect(confirmed).toEqual({ state: 'st-1' })
  })

  it('asks again while another request is verifying the same link', async () => {
    let calls = 0
    renderApp(PAGE, {
      me: USER_ME,
      api: {
        [`POST ${VERIFY}`]: () => {
          calls += 1
          if (calls === 1) {
            return {
              status: 409,
              body: { error: { code: 'conflict', message: 'x', details: { reason: 'verifying' } } },
            }
          }
          return { body: SHOWN }
        },
      },
    })

    expect(await screen.findByText('sub-1', {}, { timeout: 3000 })).toBeInTheDocument()
    expect(calls).toBe(2)
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
    expect(callsTo(fetchSpy, COMPLETE)).toBe(1)
  })

  it('fails without a state, and asks nothing', async () => {
    const { fetchSpy } = renderApp('/me/oidc-link', { me: USER_ME, api: {} })

    expect(await screen.findByRole('alert')).toBeInTheDocument()
    expect(callsTo(fetchSpy, VERIFY)).toBe(0)
  })

  it('shows a refusal at the identity provider directly, without verifying', async () => {
    const { fetchSpy } = renderApp('/me/oidc-link?state=st-1&error=access_denied', {
      me: USER_ME,
      api: {},
    })

    expect(await screen.findByRole('alert')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '关联' })).not.toBeInTheDocument()
    expect(callsTo(fetchSpy, VERIFY)).toBe(0)
  })

  it('offers no second confirmation after a refusal at confirmation', async () => {
    const { user } = renderApp(PAGE, {
      me: USER_ME,
      api: {
        [`POST ${VERIFY}`]: { body: SHOWN },
        [`POST ${COMPLETE}`]: refused('identity_taken'),
      },
    })

    await user.click(await screen.findByRole('button', { name: '关联' }))

    expect(await screen.findByRole('alert')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '关联' })).toBeDisabled()
  })

  it('falls back to the generic message for other errors', async () => {
    renderApp(PAGE, {
      me: USER_ME,
      api: { [`POST ${VERIFY}`]: { status: 500, body: errorEnvelope('internal') } },
    })

    expect(await screen.findByRole('alert')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: '返回我的账号' })).toBeInTheDocument()
  })
})
