import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type { MockResponse } from '@/test/mock-api'
import { callsTo } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import { pageOf, userDetail, userView, USERS_PATH } from '../test-data'

const BOB = userView(5, 'bob')
const IMPORT = `${USERS_PATH}/import`

describe('user details', () => {
  it('disables unlock while it runs', async () => {
    let answer: (response: MockResponse) => void = () => undefined
    const { user } = renderApp('/users?user=5', {
      api: {
        [`GET ${USERS_PATH}`]: pageOf([BOB]),
        [`GET ${USERS_PATH}/5`]: { body: userDetail(BOB) },
        [`POST ${USERS_PATH}/5/unlock`]: () =>
          new Promise<MockResponse>((resolve) => {
            answer = resolve
          }),
      },
    })
    const drawer = await screen.findByRole('dialog', { name: 'bob' })
    const unlock = within(drawer).getByRole('button', { name: '解除登录锁定' })
    await user.click(unlock)
    await waitFor(() => {
      expect(unlock).toBeDisabled()
    })
    answer({ body: { cleared: 2 } })
    await waitFor(() => {
      expect(unlock).toBeEnabled()
    })
  })

  it('treats an email of blanks as unchanged', async () => {
    const { user } = renderApp('/users?user=5', {
      api: {
        [`GET ${USERS_PATH}`]: pageOf([BOB]),
        [`GET ${USERS_PATH}/5`]: { body: userDetail(BOB) },
      },
    })
    const drawer = await screen.findByRole('dialog', { name: 'bob' })
    await user.type(within(drawer).getByLabelText('邮箱'), '   ')
    expect(within(drawer).getByRole('button', { name: '保存修改' })).toBeDisabled()
  })

  it('measures the CSV limit in UTF-8 bytes and sends nothing over it', async () => {
    const { user, fetchSpy } = renderApp('/users', {
      api: { [`GET ${USERS_PATH}`]: pageOf([BOB]) },
    })
    await user.click(await screen.findByRole('button', { name: '导入 CSV' }))
    const dialog = await screen.findByRole('dialog', { name: '导入用户' })
    // 22 000 characters, 66 000 bytes: under a character limit, over the byte limit.
    const csv = `username,note\nann,${'用'.repeat(22_000)}`
    await user.click(within(dialog).getByLabelText('CSV 文本'))
    await user.paste(csv)

    expect(await within(dialog).findByRole('alert')).toBeInTheDocument()
    const submit = within(dialog).getByRole('button', { name: '导入' })
    expect(submit).toBeDisabled()
    expect(callsTo(fetchSpy, IMPORT)).toBe(0)
  })
})
