import { act, screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type { GroupView, UserView } from '@/api/generated/model'
import { callsTo } from '@/test/mock-api'
import type { MockHandler } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

const GROUPS = '/admin/api/v1/groups'

function group(id: number, name: string, patch: Partial<GroupView> = {}): GroupView {
  return {
    id,
    name,
    note: '',
    managed_by: 'ui',
    member_count: 0,
    created_at: 1_700_000_000_000,
    updated_at: 1_700_000_000_000,
    ...patch,
  }
}

function member(id: number, username: string): UserView {
  return {
    id,
    username,
    display_name: username,
    email: null,
    email_verified: false,
    auth_source: 'local',
    role: 'user',
    status: 'active',
    must_change_password: false,
    totp_enabled: false,
    note: '',
    last_login_at: null,
    created_at: 1_700_000_000_000,
    updated_at: 1_700_000_000_000,
  }
}

function page(items: unknown[]): MockHandler {
  return () => ({ body: { items, total: items.length, page: 1, page_size: 50 } })
}

describe('groups page', () => {
  it('creates a group and opens it', async () => {
    let posted: unknown
    const ops = group(7, 'ops')
    const { router, user } = renderApp('/groups', {
      api: {
        [`GET ${GROUPS}`]: page([group(1, 'dev')]),
        [`POST ${GROUPS}`]: ({ init }) => {
          posted = JSON.parse(init.body as string)
          return { status: 201, body: ops }
        },
        [`GET ${GROUPS}/7`]: { body: ops },
        [`GET ${GROUPS}/7/members`]: page([]),
      },
    })
    await user.click(await screen.findByRole('button', { name: '新建用户组' }))
    const dialog = await screen.findByRole('dialog', { name: '新建用户组' })
    await user.type(within(dialog).getByLabelText('名称'), 'ops')
    await user.click(within(dialog).getByRole('button', { name: '创建用户组' }))

    expect(await screen.findByRole('dialog', { name: 'ops' })).toBeInTheDocument()
    expect(posted).toEqual({ name: 'ops', note: '' })
    expect(router.state.location.search).toEqual({ group: 7 })
  })

  it('adds a member found by search and asks before removing one', async () => {
    const ops = group(7, 'ops', { member_count: 1 })
    let added: unknown
    const { user, fetchSpy } = renderApp('/groups?group=7', {
      api: {
        [`GET ${GROUPS}`]: page([ops]),
        [`GET ${GROUPS}/7`]: { body: ops },
        [`GET ${GROUPS}/7/members`]: page([member(5, 'bob')]),
        'GET /admin/api/v1/users': page([member(5, 'bob'), member(6, 'carol')]),
        [`POST ${GROUPS}/7/members`]: ({ init }) => {
          added = JSON.parse(init.body as string)
          return { status: 204 }
        },
        [`DELETE ${GROUPS}/7/members/5`]: { status: 204 },
      },
    })
    const drawer = await screen.findByRole('dialog', { name: 'ops' })
    await user.type(within(drawer).getByRole('textbox', { name: '查找要添加的用户' }), 'a')
    await user.click(await within(drawer).findByRole('button', { name: '添加 carol' }))
    expect(within(drawer).queryByRole('button', { name: '添加 bob' })).not.toBeInTheDocument()
    await waitFor(() => {
      expect(added).toEqual({ user_id: 6 })
    })

    await user.click(within(drawer).getByRole('button', { name: '移除 bob' }))
    const confirm = await screen.findByRole('alertdialog', { name: '从用户组移除？' })
    expect(callsTo(fetchSpy, `${GROUPS}/7/members/5`)).toBe(0)
    await user.click(within(confirm).getByRole('button', { name: '移除' }))
    await waitFor(() => {
      expect(callsTo(fetchSpy, `${GROUPS}/7/members/5`)).toBe(1)
    })
  })

  it('shows groups from the configuration file read-only', async () => {
    const managed = group(8, 'from-config', { managed_by: 'config', member_count: 1 })
    renderApp('/groups?group=8', {
      api: {
        [`GET ${GROUPS}`]: page([managed]),
        [`GET ${GROUPS}/8`]: { body: managed },
        [`GET ${GROUPS}/8/members`]: page([member(5, 'bob')]),
      },
    })
    const drawer = await screen.findByRole('dialog', { name: 'from-config' })
    expect(within(drawer).getByText('由配置文件定义，在此处只读。')).toBeInTheDocument()
    expect(await within(drawer).findByText('bob')).toBeInTheDocument()
    expect(within(drawer).queryByRole('button', { name: '删除用户组' })).not.toBeInTheDocument()
    expect(within(drawer).queryByRole('button', { name: '移除 bob' })).not.toBeInTheDocument()
    expect(within(drawer).queryByRole('textbox')).not.toBeInTheDocument()
  })

  it.each([
    ['clicking the button', false],
    ['pressing Enter in the name field', true],
  ])('deletes after the group name is typed, by %s, and saves nothing', async (_, withEnter) => {
    const ops = group(7, 'ops')
    const { router, user, fetchSpy } = renderApp('/groups', {
      api: {
        [`GET ${GROUPS}`]: page([ops]),
        [`GET ${GROUPS}/7`]: { body: ops },
        [`GET ${GROUPS}/7/members`]: page([]),
        [`DELETE ${GROUPS}/7`]: { status: 204 },
        [`PATCH ${GROUPS}/7`]: { body: ops },
      },
    })
    await user.click(await screen.findByRole('button', { name: 'ops' }))
    const drawer = await screen.findByRole('dialog', { name: 'ops' })
    await user.click(within(drawer).getByRole('button', { name: '删除用户组' }))
    const confirm = await screen.findByRole('alertdialog', { name: '删除此用户组？' })
    const button = within(confirm).getByRole('button', { name: '删除用户组' })
    expect(button).toBeDisabled()
    await user.type(within(confirm).getByRole('textbox'), withEnter ? 'ops{Enter}' : 'ops')
    if (!withEnter) await user.click(button)
    await waitFor(() => {
      expect(router.state.location.search).toEqual({})
    })
    const methods = fetchSpy.mock.calls.map(([, init]) => init?.method ?? 'GET')
    expect(methods.filter((method) => method === 'DELETE')).toHaveLength(1)
    expect(methods).not.toContain('PATCH')

    // The drawer was closed without a history entry: Back does not reopen the deleted group.
    act(() => {
      router.history.back()
    })
    await waitFor(() => {
      expect(router.state.location.search).toEqual({})
    })
    expect(screen.queryByRole('dialog', { name: 'ops' })).not.toBeInTheDocument()
  })

  it('starts the create dialog empty again after a cancel', async () => {
    const { user } = renderApp('/groups', {
      api: { [`GET ${GROUPS}`]: page([group(1, 'dev')]) },
    })
    await user.click(await screen.findByRole('button', { name: '新建用户组' }))
    let dialog = await screen.findByRole('dialog', { name: '新建用户组' })
    await user.type(within(dialog).getByLabelText('名称'), 'ops')
    await user.keyboard('{Escape}')
    await waitFor(() => {
      expect(screen.queryByRole('dialog', { name: '新建用户组' })).not.toBeInTheDocument()
    })
    await user.click(screen.getByRole('button', { name: '新建用户组' }))
    dialog = await screen.findByRole('dialog', { name: '新建用户组' })
    expect(within(dialog).getByLabelText('名称')).toHaveValue('')
  })
})
