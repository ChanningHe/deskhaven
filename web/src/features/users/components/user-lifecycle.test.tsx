// Drawer history, dialog reset and the lifetime of show-once temporary passwords.

import { act, screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { callsTo, errorEnvelope } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import { cachedSecrets, pageOf, userDetail, userView, USERS_PATH } from '../test-data'

const BOB = userView(5, 'bob')
const SECRET = 'Tmp-Secret-123'

describe('user drawer and dialogs', () => {
  it('keeps a reset password on screen when Back closes the drawer, then forgets it', async () => {
    const { router, user, queryClient } = renderApp('/users', {
      api: {
        [`GET ${USERS_PATH}`]: pageOf([BOB]),
        [`GET ${USERS_PATH}/5`]: { body: userDetail(BOB) },
        [`POST ${USERS_PATH}/5/reset-password`]: { body: { temporary_password: SECRET } },
      },
    })
    await user.click(await screen.findByRole('button', { name: 'bob' }))
    const drawer = await screen.findByRole('dialog', { name: 'bob' })
    await user.click(within(drawer).getByRole('button', { name: '重置密码' }))
    const confirm = await screen.findByRole('alertdialog', { name: '重置密码？' })
    await user.click(within(confirm).getByRole('button', { name: '重置密码' }))
    const reveal = await screen.findByRole('dialog', { name: 'bob 的新临时密码' })

    act(() => {
      router.history.back()
    })
    await waitFor(() => {
      expect(router.state.location.search).toEqual({})
    })
    expect(screen.queryByRole('dialog', { name: 'bob' })).not.toBeInTheDocument()
    expect(within(reveal).getByDisplayValue(SECRET)).toBeInTheDocument()

    await user.click(within(reveal).getByRole('checkbox'))
    await user.click(within(reveal).getByRole('button', { name: '完成' }))
    await waitFor(() => {
      expect(screen.queryByDisplayValue(SECRET)).not.toBeInTheDocument()
    })
    await waitFor(() => {
      expect(cachedSecrets(queryClient)).toEqual([])
    })
  })

  it('drops a temporary password from the mutation cache when the page is left', async () => {
    const { router, user, queryClient } = renderApp('/users', {
      api: {
        [`GET ${USERS_PATH}`]: pageOf([BOB]),
        [`POST ${USERS_PATH}`]: {
          status: 201,
          body: { user: userView(9, 'carol'), temporary_password: SECRET },
        },
        'GET /admin/api/v1/groups': pageOf([]),
      },
    })
    await user.click(await screen.findByRole('button', { name: '新建用户' }))
    const form = await screen.findByRole('dialog', { name: '新建用户' })
    await user.type(within(form).getByLabelText('用户名'), 'carol')
    await user.click(within(form).getByRole('button', { name: '创建用户' }))
    await screen.findByDisplayValue(SECRET)
    expect(cachedSecrets(queryClient)).toHaveLength(1)

    await act(async () => {
      await router.navigate({ to: '/groups' })
    })
    await waitFor(() => {
      expect(cachedSecrets(queryClient)).toEqual([])
    })
  })

  it('closes the drawer without a history entry: Back does not reopen it', async () => {
    const { router, user } = renderApp('/users', {
      api: {
        [`GET ${USERS_PATH}`]: pageOf([BOB]),
        [`GET ${USERS_PATH}/5`]: { body: userDetail(BOB) },
      },
    })
    await user.click(await screen.findByRole('button', { name: 'bob' }))
    await screen.findByRole('dialog', { name: 'bob' })
    await user.keyboard('{Escape}')
    await waitFor(() => {
      expect(router.state.location.search).toEqual({})
    })

    act(() => {
      router.history.back()
    })
    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/users')
    })
    expect(router.state.location.search).toEqual({})
    expect(screen.queryByRole('dialog', { name: 'bob' })).not.toBeInTheDocument()
  })

  it('does not reopen a deleted account on Back', async () => {
    const { router, user, fetchSpy } = renderApp('/users', {
      api: {
        [`GET ${USERS_PATH}`]: pageOf([BOB]),
        [`GET ${USERS_PATH}/5`]: { body: userDetail(BOB) },
        [`DELETE ${USERS_PATH}/5`]: { status: 204 },
      },
    })
    await user.click(await screen.findByRole('button', { name: 'bob' }))
    const drawer = await screen.findByRole('dialog', { name: 'bob' })
    await user.click(within(drawer).getByRole('button', { name: '删除' }))
    const remove = await screen.findByRole('alertdialog', { name: '删除此账户？' })
    await user.type(within(remove).getByRole('textbox'), 'bob')
    await user.click(within(remove).getByRole('button', { name: '删除账户' }))
    await waitFor(() => {
      expect(router.state.location.search).toEqual({})
    })
    const detailCalls = callsTo(fetchSpy, `${USERS_PATH}/5`)

    act(() => {
      router.history.back()
    })
    await waitFor(() => {
      expect(router.state.location.search).toEqual({})
    })
    expect(screen.queryByRole('dialog', { name: 'bob' })).not.toBeInTheDocument()
    expect(callsTo(fetchSpy, `${USERS_PATH}/5`)).toBe(detailCalls)
  })

  it('starts the create dialog empty again after a cancel', async () => {
    const { user } = renderApp('/users', {
      api: {
        [`GET ${USERS_PATH}`]: pageOf([BOB]),
        [`POST ${USERS_PATH}`]: { status: 409, body: errorEnvelope('conflict') },
      },
    })
    await user.click(await screen.findByRole('button', { name: '新建用户' }))
    let form = await screen.findByRole('dialog', { name: '新建用户' })
    await user.type(within(form).getByLabelText('用户名'), 'bob')
    await user.click(within(form).getByRole('button', { name: '创建用户' }))
    expect(await within(form).findByRole('alert')).toBeInTheDocument()

    await user.keyboard('{Escape}')
    await waitFor(() => {
      expect(screen.queryByRole('dialog', { name: '新建用户' })).not.toBeInTheDocument()
    })
    await user.click(screen.getByRole('button', { name: '新建用户' }))
    form = await screen.findByRole('dialog', { name: '新建用户' })
    expect(within(form).getByLabelText('用户名')).toHaveValue('')
    expect(within(form).queryByRole('alert')).not.toBeInTheDocument()
  })
})
