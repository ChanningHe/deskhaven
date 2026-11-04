import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { ADMIN_ME } from '@/test/fixtures'
import { callsTo } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import {
  CONNECTION_TEST_PATH,
  DIRECTORIES_PATH,
  directoryView,
  PROVIDERS_PATH,
  SETTINGS_PATH,
  tfaSetting,
  USER_DIAGNOSTIC,
  USER_TEST_PATH,
} from '../test-data'

type Body = Record<string, unknown>

function api(directories: unknown[], extra: Record<string, unknown> = {}) {
  return {
    [`GET ${PROVIDERS_PATH}`]: { body: { callback_url: null, items: [] } },
    [`GET ${DIRECTORIES_PATH}`]: { body: directories },
    [`GET ${SETTINGS_PATH}`]: { body: [tfaSetting()] },
    ...extra,
  }
}

function ldapSection() {
  return within(screen.getByRole('region', { name: 'LDAP 目录' }))
}

describe('identity page: LDAP directory', () => {
  it('shows admins the directory read-only, without tests', async () => {
    renderApp('/identity', { me: ADMIN_ME, api: api([directoryView()]) })

    const section = within(await screen.findByRole('region', { name: 'LDAP 目录' }))
    expect(await section.findByLabelText('URL')).toHaveValue('ldaps://ldap.example.org')
    expect(section.getByLabelText('URL')).toBeDisabled()
    expect(section.getByText(/只有所有者可以修改目录/)).toBeInTheDocument()
    expect(section.queryByRole('button', { name: '测试连接' })).not.toBeInTheDocument()
    expect(section.queryByRole('button', { name: '保存修改' })).not.toBeInTheDocument()
  })

  it('tests the saved directory and shows every step', async () => {
    let sent: Body | undefined
    const { user } = renderApp('/identity', {
      api: api([directoryView()], {
        [`POST ${CONNECTION_TEST_PATH}`]: ({ init }: { init: RequestInit }) => {
          sent = JSON.parse(init.body as string) as Body
          return {
            body: {
              passed: false,
              steps: [
                { step: 'connect', status: 'failed', message: 'connection refused' },
                { step: 'service_bind', status: 'skipped', message: 'skipped' },
              ],
              user: null,
            },
          }
        },
      }),
    })
    await user.click(await screen.findByRole('button', { name: '测试连接' }))

    const result = within(await screen.findByRole('region', { name: '测试结果' }))
    expect(sent).toEqual({ directory_id: 4 })
    expect(result.getByText('有步骤失败。')).toBeInTheDocument()
    const connect = within(result.getByText('连接').closest('li') as HTMLElement)
    expect(connect.getByText('失败')).toBeInTheDocument()
    expect(connect.getByText('connection refused')).toBeInTheDocument()
    const bind = within(result.getByText('服务账号 bind').closest('li') as HTMLElement)
    expect(bind.getByText('跳过')).toBeInTheDocument()
  })

  it('tests a user with a password and shows the entry found', async () => {
    let sent: Body | undefined
    const { user } = renderApp('/identity', {
      api: api([directoryView()], {
        [`POST ${USER_TEST_PATH}`]: ({ init }: { init: RequestInit }) => {
          sent = JSON.parse(init.body as string) as Body
          return { body: USER_DIAGNOSTIC }
        },
      }),
    })
    await screen.findByRole('button', { name: '测试连接' })
    const section = ldapSection()
    expect(section.getByText(/计入该账户的登录锁定/)).toBeInTheDocument()
    await user.type(section.getByLabelText('用户名'), 'alice')
    await user.type(section.getByLabelText('密码（可选）'), 'pw-1')
    await user.click(section.getByRole('button', { name: '测试用户' }))

    const result = within(await screen.findByRole('region', { name: '测试结果' }))
    expect(sent).toEqual({ directory_id: 4, username: 'alice', password: 'pw-1' })
    const bind = within(result.getByText('用户 bind').closest('li') as HTMLElement)
    expect(bind.getByText('the directory refused the password')).toBeInTheDocument()
    expect(result.getByText('uid=alice,ou=people')).toBeInTheDocument()
    expect(result.getByText('未配置管理员分组')).toBeInTheDocument()
  })

  it('tests unsaved changes as a form and asks for the password when the URL changes', async () => {
    let sent: Body | undefined
    const { user, fetchSpy } = renderApp('/identity', {
      api: api([directoryView()], {
        [`POST ${CONNECTION_TEST_PATH}`]: ({ init }: { init: RequestInit }) => {
          sent = JSON.parse(init.body as string) as Body
          return { body: { passed: true, steps: [], user: null } }
        },
      }),
    })
    const url = await screen.findByLabelText('URL')
    expect(screen.getByLabelText('Bind 密码')).toHaveValue('')
    await user.clear(url)
    await user.type(url, 'ldaps://other.example.org')

    expect(screen.getByText('连接已修改：请重新输入 Bind 密码。')).toBeInTheDocument()
    expect(ldapSection().getByRole('button', { name: '保存修改' })).toBeDisabled()
    await user.click(screen.getByRole('button', { name: '测试连接' }))
    await waitFor(() => {
      expect(sent).toBeDefined()
    })
    expect(sent?.directory_id).toBeUndefined()
    expect(sent?.directory).toMatchObject({ url: 'ldaps://other.example.org', bind_password: null })
    expect(callsTo(fetchSpy, `${DIRECTORIES_PATH}/4`)).toBe(0)
  })

  it('saves only the edited fields with the re-entered password', async () => {
    let patched: Body | undefined
    const { user } = renderApp('/identity', {
      api: api([directoryView()], {
        [`PATCH ${DIRECTORIES_PATH}/4`]: ({ init }: { init: RequestInit }) => {
          patched = JSON.parse(init.body as string) as Body
          return { body: directoryView({ url: 'ldaps://other.example.org' }) }
        },
      }),
    })
    const url = await screen.findByLabelText('URL')
    await user.clear(url)
    await user.type(url, 'ldaps://other.example.org')
    await user.type(screen.getByLabelText('Bind 密码'), 'new-pw')
    await user.click(ldapSection().getByRole('button', { name: '保存修改' }))

    await waitFor(() => {
      expect(patched).toEqual({ url: 'ldaps://other.example.org', bind_password: 'new-pw' })
    })
  })

  it('warns in red when certificate verification is turned off', async () => {
    const { user } = renderApp('/identity', { api: api([]) })
    const verify = await screen.findByRole('checkbox', { name: '校验服务器证书' })
    expect(screen.queryByText(/证书校验已关闭/)).not.toBeInTheDocument()

    await user.click(verify)

    expect(screen.getByText(/证书校验已关闭/)).toBeInTheDocument()
    expect(ldapSection().getByRole('button', { name: '保存目录' })).toBeEnabled()
  })
})
