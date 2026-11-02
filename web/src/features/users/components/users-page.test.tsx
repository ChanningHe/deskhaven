import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type { UserDetail, UserView } from '@/api/generated/model'
import { ADMIN_ME } from '@/test/fixtures'
import { callsTo, errorEnvelope, requestedUrl } from '@/test/mock-api'
import type { MockHandler } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

function user(id: number, username: string, patch: Partial<UserView> = {}): UserView {
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
    ...patch,
  }
}

const LIST_PATH = '/admin/api/v1/users'
const BOB = user(5, 'bob')
const BOSS = user(6, 'boss', { role: 'owner' })

function list(items: UserView[], total = items.length): MockHandler {
  return ({ url }) => ({
    body: {
      items,
      total,
      page: Number(url.searchParams.get('page') ?? 1),
      page_size: 20,
    },
  })
}

function detail(target: UserView): UserDetail {
  return { user: target, groups: [{ id: 3, name: 'ops' }], sessions: [] }
}

function listRequests(spy: ReturnType<typeof renderApp>['fetchSpy']): URL[] {
  return spy.mock.calls
    .map((_, index) => requestedUrl(spy, index))
    .filter((url) => url.pathname === LIST_PATH)
}

describe('users page', () => {
  it('keeps filters, sort and page in the URL and in the list request', async () => {
    const {
      router,
      user: actor,
      fetchSpy,
    } = renderApp('/users?role=admin&page=2&sort=-username', {
      api: { [`GET ${LIST_PATH}`]: list([BOB], 45) },
    })

    expect(await screen.findByRole('button', { name: 'bob' })).toBeInTheDocument()
    const first = listRequests(fetchSpy)[0]
    expect(first?.searchParams.get('role')).toBe('admin')
    expect(first?.searchParams.get('page')).toBe('2')
    expect(first?.searchParams.get('sort')).toBe('-username')

    await actor.selectOptions(screen.getByRole('combobox', { name: '全部状态' }), 'disabled')

    await waitFor(() => {
      expect(router.state.location.search).toEqual({
        role: 'admin',
        sort: '-username',
        status: 'disabled',
      })
    })
    await waitFor(() => {
      expect(listRequests(fetchSpy).at(-1)?.searchParams.get('status')).toBe('disabled')
    })
    expect(listRequests(fetchSpy).at(-1)?.searchParams.get('page')).toBe('1')
  })

  it('creates a user and shows the generated password exactly once', async () => {
    let posted: unknown
    const { user: actor } = renderApp('/users', {
      api: {
        [`GET ${LIST_PATH}`]: list([BOB]),
        [`POST ${LIST_PATH}`]: ({ init }) => {
          posted = JSON.parse(init.body as string)
          return {
            status: 201,
            body: { user: user(9, 'carol'), temporary_password: 'Tmp-Secret-123' },
          }
        },
      },
    })
    await actor.click(await screen.findByRole('button', { name: '新建用户' }))
    const form = await screen.findByRole('dialog', { name: '新建用户' })
    await actor.type(within(form).getByLabelText('用户名'), 'carol')
    await actor.click(within(form).getByRole('button', { name: '创建用户' }))

    const reveal = await screen.findByRole('dialog', { name: 'carol 的临时密码' })
    expect(posted).toEqual({ username: 'carol', display_name: '', role: 'user' })
    expect(within(reveal).getByRole('textbox')).toHaveValue('Tmp-Secret-123')
    await actor.click(within(reveal).getByRole('checkbox'))
    await actor.click(within(reveal).getByRole('button', { name: '完成' }))

    await waitFor(() => {
      expect(screen.queryByText('Tmp-Secret-123')).not.toBeInTheDocument()
    })
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
  })

  it('asks for the username before deleting and confirms before disabling', async () => {
    const {
      router,
      user: actor,
      fetchSpy,
    } = renderApp('/users?user=5', {
      api: {
        [`GET ${LIST_PATH}`]: list([BOB]),
        [`GET ${LIST_PATH}/5`]: { body: detail(BOB) },
        [`POST ${LIST_PATH}/5/disable`]: { body: { ...BOB, status: 'disabled' } },
        [`DELETE ${LIST_PATH}/5`]: { status: 204 },
      },
    })
    const drawer = await screen.findByRole('dialog', { name: 'bob' })
    expect(within(drawer).getByText('ops')).toBeInTheDocument()

    await actor.click(within(drawer).getByRole('button', { name: '禁用' }))
    const disable = await screen.findByRole('alertdialog', { name: '禁用此账户？' })
    expect(callsTo(fetchSpy, `${LIST_PATH}/5/disable`)).toBe(0)
    await actor.click(within(disable).getByRole('button', { name: '禁用' }))
    await waitFor(() => {
      expect(callsTo(fetchSpy, `${LIST_PATH}/5/disable`)).toBe(1)
    })

    await actor.click(within(drawer).getByRole('button', { name: '删除' }))
    const remove = await screen.findByRole('alertdialog', { name: '删除此账户？' })
    const confirm = within(remove).getByRole('button', { name: '删除账户' })
    expect(confirm).toBeDisabled()
    await actor.type(within(remove).getByRole('textbox'), 'bob')
    await actor.click(confirm)

    await waitFor(() => {
      expect(router.state.location.search).toEqual({})
    })
    const deletes = fetchSpy.mock.calls.filter(([, init]) => init?.method === 'DELETE')
    expect(deletes).toHaveLength(1)
  })

  it('hides owner management and the owner role from admins', async () => {
    const { user: actor } = renderApp('/users?user=6', {
      me: ADMIN_ME,
      api: {
        [`GET ${LIST_PATH}`]: list([BOSS]),
        [`GET ${LIST_PATH}/6`]: { body: detail(BOSS) },
      },
    })
    const drawer = await screen.findByRole('dialog', { name: 'boss' })
    expect(within(drawer).getByText('所有者账户只能由所有者修改。')).toBeInTheDocument()
    expect(within(drawer).queryByRole('button', { name: '重置密码' })).not.toBeInTheDocument()
    expect(within(drawer).queryByRole('button', { name: '删除' })).not.toBeInTheDocument()
    expect(within(drawer).getByLabelText('显示名称')).toBeDisabled()

    await actor.keyboard('{Escape}')
    await actor.click(await screen.findByRole('button', { name: '新建用户' }))
    const form = await screen.findByRole('dialog', { name: '新建用户' })
    const roles = within(within(form).getByLabelText('角色')).getAllByRole('option')
    expect(roles.map((option) => option.textContent)).toEqual(['用户', '管理员'])
  })

  it('never offers to delete your own account or reset your own 2FA here', async () => {
    const self = user(1, 'owner', { role: 'owner', totp_enabled: true })
    renderApp('/users?user=1', {
      api: {
        [`GET ${LIST_PATH}`]: list([self]),
        [`GET ${LIST_PATH}/1`]: { body: detail(self) },
      },
    })
    const drawer = await screen.findByRole('dialog', { name: 'owner' })
    expect(within(drawer).getByRole('button', { name: '禁用' })).toBeInTheDocument()
    expect(within(drawer).queryByRole('button', { name: '删除' })).not.toBeInTheDocument()
    expect(within(drawer).queryByRole('button', { name: '重置 2FA' })).not.toBeInTheDocument()
  })

  it('lists every rejected import row and imports nothing', async () => {
    const rejection = {
      error: {
        code: 'validation_failed',
        message: 'import rejected',
        details: {
          rows: [
            { line: 3, field: 'username', code: 'taken' },
            { line: 4, field: null, code: 'columns' },
          ],
        },
      },
    }
    const { user: actor } = renderApp('/users', {
      api: {
        [`GET ${LIST_PATH}`]: list([BOB]),
        [`POST ${LIST_PATH}/import`]: { status: 422, body: rejection },
      },
    })
    await actor.click(await screen.findByRole('button', { name: '导入 CSV' }))
    const dialog = await screen.findByRole('dialog', { name: '导入用户' })
    await actor.type(within(dialog).getByLabelText('CSV 文本'), 'username{enter}bob')
    await actor.click(within(dialog).getByRole('button', { name: '导入' }))

    const alert = await within(dialog).findByRole('alert')
    expect(alert).toHaveTextContent('第 3 行: username：已被其他账户或前面的行使用')
    expect(alert).toHaveTextContent('第 4 行: 列数不正确')
  })

  it('shows the server error of a refused change', async () => {
    const { user: actor } = renderApp('/users?user=5', {
      api: {
        [`GET ${LIST_PATH}`]: list([BOB]),
        [`GET ${LIST_PATH}/5`]: { body: detail(BOB) },
        [`POST ${LIST_PATH}/5/unlock`]: {
          status: 403,
          body: errorEnvelope('owner_required'),
        },
      },
    })
    const drawer = await screen.findByRole('dialog', { name: 'bob' })
    await actor.click(within(drawer).getByRole('button', { name: '解除登录锁定' }))
    expect(await within(drawer).findByRole('alert')).toHaveTextContent(
      '只有所有者可以管理所有者账户或授予所有者角色。',
    )
  })
})
