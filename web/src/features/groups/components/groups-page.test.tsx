import { act, screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { getGetGroupQueryKey } from '@/api/generated/groups/groups'
import type { GroupView, UserView } from '@/api/generated/model'
import { callsTo, requestedUrl } from '@/test/mock-api'
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
  it('pages through the members and shows their total', async () => {
    const big = group(7, 'big', { member_count: 120 })
    const everyone = Array.from({ length: 120 }, (_, i) =>
      member(100 + i, `m${String(i).padStart(3, '0')}`),
    )
    const { user, fetchSpy } = renderApp('/groups?group=7', {
      api: {
        [`GET ${GROUPS}`]: page([big]),
        [`GET ${GROUPS}/7`]: { body: big },
        [`GET ${GROUPS}/7/members`]: ({ url }) => {
          const at = Number(url.searchParams.get('page') ?? 1)
          const size = Number(url.searchParams.get('page_size'))
          const items = everyone.slice((at - 1) * size, at * size)
          return { body: { items, total: everyone.length, page: at, page_size: size } }
        },
      },
    })
    const drawer = await screen.findByRole('dialog', { name: 'big' })
    expect(await within(drawer).findByText('m000')).toBeInTheDocument()
    expect(within(drawer).getByText('共 120 名')).toBeInTheDocument()
    expect(within(drawer).getByText('第 1 / 3 页')).toBeInTheDocument()

    await user.click(within(drawer).getByRole('button', { name: '下一页' }))
    expect(await within(drawer).findByText('m050')).toBeInTheDocument()
    expect(within(drawer).queryByText('m000')).not.toBeInTheDocument()
    const pages = fetchSpy.mock.calls
      .map((_, index) => requestedUrl(fetchSpy, index))
      .filter((url) => url.pathname === `${GROUPS}/7/members`)
      .map((url) => url.searchParams.get('page'))
    expect(pages).toContain('2')
  })

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

  it('saves only the edited field when another admin changed the other one meanwhile', async () => {
    let served = group(7, 'ops', { note: 'night shift' })
    const bodies: unknown[] = []
    const { user, queryClient } = renderApp('/groups?group=7', {
      api: {
        [`GET ${GROUPS}`]: page([served]),
        [`GET ${GROUPS}/7`]: () => ({ body: served }),
        [`GET ${GROUPS}/7/members`]: page([]),
        [`PATCH ${GROUPS}/7`]: ({ init }) => {
          bodies.push(JSON.parse(init.body as string))
          served = { ...served, name: 'ops-2' }
          return { body: served }
        },
      },
    })
    const drawer = await screen.findByRole('dialog', { name: 'ops' })
    const name = within(drawer).getByLabelText('名称')
    await user.clear(name)
    await user.type(name, 'ops-2')

    served = group(7, 'ops', { note: 'moved to days' })
    await act(async () => {
      await queryClient.refetchQueries({ queryKey: getGetGroupQueryKey(7) })
    })
    await waitFor(() => {
      expect(within(drawer).getByLabelText('备注')).toHaveValue('moved to days')
    })
    expect(name).toHaveValue('ops-2')

    await user.click(within(drawer).getByRole('button', { name: '保存修改' }))
    await waitFor(() => {
      expect(bodies).toEqual([{ name: 'ops-2' }])
    })
  })

  it('adds a member found by search and asks before removing one', async () => {
    const ops = group(7, 'ops', { member_count: 1 })
    let added: unknown
    const { user, fetchSpy } = renderApp('/groups?group=7', {
      api: {
        [`GET ${GROUPS}`]: page([ops]),
        [`GET ${GROUPS}/7`]: { body: ops },
        [`GET ${GROUPS}/7/members`]: page([member(5, 'bob')]),
        // The server leaves the members out (`not_group`).
        'GET /admin/api/v1/users': page([member(6, 'carol'), member(8, 'dave')]),
        [`POST ${GROUPS}/7/members`]: ({ init }) => {
          added = JSON.parse(init.body as string)
          return { status: 204 }
        },
        [`DELETE ${GROUPS}/7/members/5`]: { status: 204 },
      },
    })
    const drawer = await screen.findByRole('dialog', { name: 'ops' })
    const search = within(drawer).getByRole('combobox', { name: '查找要添加的用户' })
    await user.type(search, 'da')
    const listbox = await within(drawer).findByRole('listbox', { name: '组外用户' })
    expect(within(drawer).getByRole('status')).toHaveTextContent('组外有 2 个匹配用户')
    await waitFor(() => {
      expect(search).toHaveAttribute('aria-busy', 'false')
    })
    // Focus lists the first accounts; then debounced: one request for the whole term. The
    // server leaves the members out.
    const requests = fetchSpy.mock.calls
      .map((_, index) => requestedUrl(fetchSpy, index))
      .filter((url) => url.pathname === '/admin/api/v1/users')
    expect(requests.map((url) => url.searchParams.get('q'))).toEqual(['', 'da'])
    expect(requests.every((url) => url.searchParams.get('not_group') === '7')).toBe(true)

    // Keyboard: the first suggestion is highlighted; ArrowDown moves on, Enter adds it.
    expect(within(listbox).getByRole('option', { name: /carol/ })).toHaveAttribute(
      'aria-selected',
      'true',
    )
    await user.keyboard('{ArrowDown}{Enter}')
    await waitFor(() => {
      expect(added).toEqual({ user_id: 8 })
    })
    expect(search).toHaveValue('')

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
    expect(within(drawer).queryByRole('combobox')).not.toBeInTheDocument()
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
