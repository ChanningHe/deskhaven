import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { ADMIN_ME } from '@/test/fixtures'
import { callsTo } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import {
  pageOf as list,
  userDetail as detail,
  userView as user,
  USERS_PATH as LIST_PATH,
} from '../test-data'

const BOB = user(5, 'bob')
const BOSS = user(6, 'boss', { role: 'owner' })

describe('users page', () => {
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
      expect(screen.queryByDisplayValue('Tmp-Secret-123')).not.toBeInTheDocument()
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
    expect(within(drawer).queryByRole('button', { name: '重置密码' })).not.toBeInTheDocument()
    expect(within(drawer).queryByRole('button', { name: '删除' })).not.toBeInTheDocument()
    expect(within(drawer).getByLabelText('显示名称')).toBeDisabled()

    await actor.keyboard('{Escape}')
    await actor.click(await screen.findByRole('button', { name: '新建用户' }))
    const form = await screen.findByRole('dialog', { name: '新建用户' })
    const roles = within(within(form).getByLabelText('角色')).getAllByRole('option')
    expect(roles.map((option) => (option as HTMLOptionElement).value)).toEqual(['user', 'admin'])
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
})
