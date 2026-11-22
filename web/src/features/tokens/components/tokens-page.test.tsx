import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { ADMIN_ME } from '@/test/fixtures'
import { callsTo, errorEnvelope, requestedUrl } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import { pageOf, TOKENS_PATH, tokenView } from '../test-data'
import { MS_PER_DAY } from '../token-rules'

const CI = tokenView(1, 'ci deploy', {
  scopes: ['deploy', 'assign'],
  expires_at: 1_900_000_000_000,
  last_used_at: 1_800_000_000_000,
})
const OWNERS = tokenView(2, 'owner token', {
  created_by: { id: 9, username: 'boss', role: 'owner' },
})
const OLD = tokenView(3, 'old token', { status: 'revoked', revoked_at: 1_750_000_000_000 })

function listRequests(spy: ReturnType<typeof renderApp>['fetchSpy']): URL[] {
  return spy.mock.calls
    .map((_, index) => requestedUrl(spy, index))
    .filter((url) => url.pathname === TOKENS_PATH)
}

describe('API tokens page', () => {
  it('lists name, prefix, scopes, creator, expiry, last use and status', async () => {
    renderApp('/tokens', { api: { [`GET ${TOKENS_PATH}`]: pageOf([CI, OLD]) } })

    const ci = (await screen.findByText('ci deploy')).closest('tr')
    expect(ci).not.toBeNull()
    const row = within(ci as HTMLElement)
    expect(row.getByText('dhk_abcd0001')).toBeInTheDocument()
    expect(row.getByText('deploy')).toBeInTheDocument()
    expect(row.getByText('assign')).toBeInTheDocument()
    expect(row.getByText('admin')).toBeInTheDocument()
    expect(row.getByText('有效')).toBeInTheDocument()
    const old = within(screen.getByText('old token').closest('tr') as HTMLElement)
    expect(old.getByText('已吊销')).toBeInTheDocument()
    expect(old.queryByRole('button', { name: /吊销令牌/ })).not.toBeInTheDocument()
  })

  it('keeps search and sort in the URL and in the list request', async () => {
    const { router, fetchSpy } = renderApp('/tokens?q=ci&sort=-name', {
      api: { [`GET ${TOKENS_PATH}`]: pageOf([CI]) },
    })

    await screen.findByText('ci deploy')

    const first = listRequests(fetchSpy)[0]
    expect(first?.searchParams.get('q')).toBe('ci')
    expect(first?.searchParams.get('sort')).toBe('-name')
    expect(router.state.location.search).toEqual({ q: 'ci', sort: '-name' })
  })

  it('creates a token with 90 days preselected and shows its secret exactly once', async () => {
    let posted: { name: string; scopes: string[]; expires_at: number | null } | undefined
    const { user: actor } = renderApp('/tokens', {
      api: {
        [`GET ${TOKENS_PATH}`]: pageOf([CI]),
        [`POST ${TOKENS_PATH}`]: ({ init }) => {
          posted = JSON.parse(init.body as string) as typeof posted
          return {
            status: 201,
            body: { token: tokenView(7, 'ci new'), secret: 'dhk_Secret-Value-123' },
          }
        },
      },
    })
    await actor.click(await screen.findByRole('button', { name: '新建令牌' }))
    const form = await screen.findByRole('dialog', { name: '新建 API 令牌' })
    expect(within(form).getByRole('combobox', { name: '有效期' })).toHaveValue('90')
    const submit = within(form).getByRole('button', { name: '创建令牌' })
    await actor.type(within(form).getByLabelText('名称'), 'ci new')
    expect(submit).toBeDisabled()
    await actor.click(within(form).getByRole('checkbox', { name: /^deploy/ }))
    expect(submit).toBeEnabled()
    const before = Date.now()
    await actor.click(submit)

    const reveal = await screen.findByRole('dialog', { name: '令牌「ci new」' })
    expect(within(reveal).getByRole('textbox')).toHaveValue('dhk_Secret-Value-123')
    expect(posted?.name).toBe('ci new')
    expect(posted?.scopes).toEqual(['deploy'])
    expect(posted?.expires_at).toBeGreaterThanOrEqual(before + 90 * MS_PER_DAY)
    expect(posted?.expires_at).toBeLessThanOrEqual(Date.now() + 90 * MS_PER_DAY)
    await actor.click(within(reveal).getByRole('checkbox'))
    await actor.click(within(reveal).getByRole('button', { name: '完成' }))
    await waitFor(() => {
      expect(screen.queryByDisplayValue('dhk_Secret-Value-123')).not.toBeInTheDocument()
    })
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
  })

  it('warns when "never expires" is chosen and sends no expiry', async () => {
    let posted: { expires_at: number | null } | undefined
    const { user: actor } = renderApp('/tokens', {
      api: {
        [`GET ${TOKENS_PATH}`]: pageOf([]),
        [`POST ${TOKENS_PATH}`]: ({ init }) => {
          posted = JSON.parse(init.body as string) as typeof posted
          return { status: 201, body: { token: tokenView(8, 'forever'), secret: 'dhk_x' } }
        },
      },
    })
    await actor.click(await screen.findByRole('button', { name: '新建令牌' }))
    const form = await screen.findByRole('dialog', { name: '新建 API 令牌' })
    expect(within(form).queryByText(/永不过期的令牌/)).not.toBeInTheDocument()

    await actor.selectOptions(within(form).getByRole('combobox', { name: '有效期' }), 'never')

    expect(within(form).getByText(/永不过期的令牌/)).toBeInTheDocument()
    await actor.type(within(form).getByLabelText('名称'), 'forever')
    await actor.click(within(form).getByRole('checkbox', { name: /^assign/ }))
    await actor.click(within(form).getByRole('button', { name: '创建令牌' }))
    await screen.findByRole('dialog', { name: '令牌「forever」' })
    expect(posted?.expires_at).toBeNull()
  })

  it('shows the translated error and keeps the form when creation is refused', async () => {
    const { user: actor } = renderApp('/tokens', {
      api: {
        [`GET ${TOKENS_PATH}`]: pageOf([]),
        [`POST ${TOKENS_PATH}`]: { status: 403, body: errorEnvelope('forbidden') },
      },
    })
    await actor.click(await screen.findByRole('button', { name: '新建令牌' }))
    const form = await screen.findByRole('dialog', { name: '新建 API 令牌' })
    await actor.type(within(form).getByLabelText('名称'), 'x')
    await actor.click(within(form).getByRole('checkbox', { name: /^deploy/ }))
    await actor.click(within(form).getByRole('button', { name: '创建令牌' }))

    expect(await within(form).findByRole('alert')).toBeInTheDocument()
    expect(screen.queryByRole('dialog', { name: /令牌「/ })).not.toBeInTheDocument()
  })

  it('asks before revoking, then revokes and refreshes the list', async () => {
    let revoked = false
    const { user: actor, fetchSpy } = renderApp('/tokens', {
      api: {
        [`GET ${TOKENS_PATH}`]: () => ({
          body: {
            items: [revoked ? { ...CI, status: 'revoked', revoked_at: 1 } : CI],
            total: 1,
            page: 1,
            page_size: 20,
          },
        }),
        [`POST ${TOKENS_PATH}/1/revoke`]: () => {
          revoked = true
          return { body: { ...CI, status: 'revoked', revoked_at: 1 } }
        },
      },
    })
    await actor.click(await screen.findByRole('button', { name: '吊销令牌 ci deploy' }))
    const confirm = await screen.findByRole('alertdialog', { name: '吊销此令牌？' })
    expect(callsTo(fetchSpy, `${TOKENS_PATH}/1/revoke`)).toBe(0)
    await actor.click(within(confirm).getByRole('button', { name: '吊销令牌' }))

    await waitFor(() => {
      expect(callsTo(fetchSpy, `${TOKENS_PATH}/1/revoke`)).toBe(1)
    })
    expect(await screen.findByText('已吊销')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '吊销令牌 ci deploy' })).not.toBeInTheDocument()
  })

  it('disables revoking an owner token for an admin and says why', async () => {
    renderApp('/tokens', {
      me: ADMIN_ME,
      api: { [`GET ${TOKENS_PATH}`]: pageOf([OWNERS]) },
    })

    const revoke = await screen.findByRole('button', { name: '吊销令牌 owner token' })

    expect(revoke).toBeDisabled()
    expect(revoke).toHaveAttribute('title', '所有者的令牌和系统令牌只能由所有者吊销。')
  })
})
