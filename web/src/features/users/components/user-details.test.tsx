// Smaller behaviours: busy buttons, stable confirm texts, the email field, CSV limits.

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
    expect(within(drawer).getByText('已解除登录锁定。')).toBeInTheDocument()
  })

  it('says when the drawer shows only the most recently used sessions', async () => {
    const session = {
      id: 1,
      kind: 'console' as const,
      created_at: 1_790_000_000_000,
      last_used_at: 1_800_000_000_000,
      expires_at: 1_900_000_000_000,
    }
    renderApp('/users?user=5', {
      api: {
        [`GET ${USERS_PATH}`]: pageOf([BOB]),
        [`GET ${USERS_PATH}/5`]: {
          body: { ...userDetail(BOB), sessions: [session], sessions_total: 300 },
        },
      },
    })
    const drawer = await screen.findByRole('dialog', { name: 'bob' })
    expect(within(drawer).getByText('仅显示最近使用的 1 / 300 个会话。')).toBeInTheDocument()
  })

  it('keeps the confirm texts while the dialog closes', async () => {
    const { user } = renderApp('/users?user=5', {
      api: {
        [`GET ${USERS_PATH}`]: pageOf([BOB]),
        [`GET ${USERS_PATH}/5`]: { body: userDetail(BOB) },
      },
    })
    const drawer = await screen.findByRole('dialog', { name: 'bob' })
    await user.click(within(drawer).getByRole('button', { name: '禁用' }))
    const confirm = await screen.findByRole('alertdialog', { name: '禁用此账户？' })
    await user.click(within(confirm).getByRole('button', { name: '取消' }))
    // Whatever is still on screen of the closing dialog names the same action.
    for (const dialog of screen.queryAllByRole('alertdialog', { hidden: true })) {
      expect(dialog).toHaveTextContent('禁用此账户？')
    }
    await waitFor(() => {
      expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
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

    expect(await within(dialog).findByRole('alert')).toHaveTextContent(
      'CSV 大小为 65 KiB，单次最多导入 64 KiB。',
    )
    const submit = within(dialog).getByRole('button', { name: '导入' })
    expect(submit).toBeDisabled()
    expect(callsTo(fetchSpy, IMPORT)).toBe(0)
  })

  it('reports a file that cannot be read', async () => {
    const { user } = renderApp('/users', {
      api: { [`GET ${USERS_PATH}`]: pageOf([BOB]) },
    })
    await user.click(await screen.findByRole('button', { name: '导入 CSV' }))
    const dialog = await screen.findByRole('dialog', { name: '导入用户' })
    const broken = new File(['username\nann\n'], 'users.csv', { type: 'text/csv' })
    Object.defineProperty(broken, 'text', {
      value: () => Promise.reject(new Error('unreadable')),
    })
    await user.upload(within(dialog).getByLabelText('CSV 文件'), broken)

    expect(await within(dialog).findByRole('alert')).toHaveTextContent('无法读取该文件')
    expect(within(dialog).getByRole('button', { name: '导入' })).toBeDisabled()
  })
})
