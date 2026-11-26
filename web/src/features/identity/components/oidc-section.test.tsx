import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { ADMIN_ME } from '@/test/fixtures'
import { renderApp } from '@/test/render-app'

import {
  CALLBACK_URL,
  DIRECTORIES_PATH,
  PROVIDERS_PATH,
  providerView,
  SETTINGS_PATH,
  tfaSetting,
} from '../test-data'

const CORP = providerView(1, 'corp', { admin_group: 'rd-admins' })
const MANAGED = providerView(2, 'managed', { managed_by: 'config', has_client_secret: false })

function api(extra: Record<string, unknown> = {}) {
  return {
    [`GET ${PROVIDERS_PATH}`]: { body: { callback_url: CALLBACK_URL, items: [CORP, MANAGED] } },
    [`GET ${DIRECTORIES_PATH}`]: { body: [] },
    [`GET ${SETTINGS_PATH}`]: { body: [tfaSetting()] },
    ...extra,
  }
}

describe('identity page: OIDC providers', () => {
  it('offers no delete for a config-managed provider', async () => {
    renderApp('/identity', { api: api() })

    const table = await screen.findByRole('table', { name: 'OIDC 提供方' })
    expect(within(table).getByRole('button', { name: '删除 corp' })).toBeInTheDocument()
    expect(within(table).queryByRole('button', { name: '删除 managed' })).not.toBeInTheDocument()
  })

  it('creates a provider with its secret', async () => {
    let posted: Record<string, unknown> | undefined
    const { user } = renderApp('/identity', {
      api: api({
        [`POST ${PROVIDERS_PATH}`]: ({ init }: { init: RequestInit }) => {
          posted = JSON.parse(init.body as string) as Record<string, unknown>
          return { status: 201, body: providerView(9, 'okta') }
        },
      }),
    })
    await user.click(await screen.findByRole('button', { name: '新建提供方' }))
    const dialog = await screen.findByRole('dialog', { name: '新建 OIDC 提供方' })
    const form = within(dialog)

    await user.type(form.getByLabelText('名称'), 'okta')
    await user.type(form.getByLabelText('Issuer 地址'), 'https://okta.example.com')
    await user.type(form.getByLabelText('Client ID'), 'rd')
    await user.type(form.getByLabelText('Client secret'), 's3cret')
    await user.click(form.getByRole('button', { name: '创建提供方' }))

    await waitFor(() => {
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    })
    expect(posted).toMatchObject({
      name: 'okta',
      issuer: 'https://okta.example.com',
      client_id: 'rd',
      client_secret: 's3cret',
      scopes: 'openid profile email',
      enabled: true,
    })
  })

  it('edits only the changed fields and never sends a secret that was not typed', async () => {
    let patched: Record<string, unknown> | undefined
    const { user } = renderApp('/identity', {
      api: api({
        [`PATCH ${PROVIDERS_PATH}/1`]: ({ init }: { init: RequestInit }) => {
          patched = JSON.parse(init.body as string) as Record<string, unknown>
          return { body: { ...CORP, display_name: 'Corp' } }
        },
      }),
    })
    await user.click(await screen.findByRole('button', { name: '编辑 corp' }))
    const form = within(await screen.findByRole('dialog', { name: 'OIDC 提供方 corp' }))
    expect(form.getByLabelText('Issuer 地址')).toBeDisabled()
    expect(form.getByLabelText('Client secret')).toHaveValue('')

    const name = form.getByLabelText('显示名称')
    await user.clear(name)
    await user.type(name, 'Corp')
    await user.click(form.getByRole('button', { name: '保存修改' }))

    await waitFor(() => {
      expect(patched).toEqual({ display_name: 'Corp' })
    })
  })

  it('shows a config-managed provider read-only', async () => {
    const { user } = renderApp('/identity', { api: api() })
    await user.click(await screen.findByRole('button', { name: '编辑 managed' }))
    const form = within(await screen.findByRole('dialog', { name: 'OIDC 提供方 managed' }))

    expect(form.getByLabelText('显示名称')).toBeDisabled()
    expect(form.queryByRole('button', { name: '保存修改' })).not.toBeInTheDocument()
  })

  it('shows admins the providers read-only: no create, delete or save', async () => {
    const { user } = renderApp('/identity', { me: ADMIN_ME, api: api() })

    const table = await screen.findByRole('table', { name: 'OIDC 提供方' })
    const section = within(screen.getByRole('region', { name: 'OIDC 提供方' }))
    expect(section.queryByRole('button', { name: '新建提供方' })).not.toBeInTheDocument()
    expect(within(table).queryByRole('button', { name: '删除 corp' })).not.toBeInTheDocument()

    await user.click(within(table).getByRole('button', { name: '查看 corp' }))
    const form = within(await screen.findByRole('dialog', { name: 'OIDC 提供方 corp' }))
    expect(form.getByLabelText('显示名称')).toBeDisabled()
    expect(form.queryByRole('button', { name: '保存修改' })).not.toBeInTheDocument()
  })
})
