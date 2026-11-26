import { act, screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

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
})
