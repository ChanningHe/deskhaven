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

async function findLdapSection() {
  return within(await screen.findByRole('region', { name: 'LDAP 目录' }))
}

describe('identity page: LDAP directory', () => {
  it('shows admins the directory read-only, without tests', async () => {
    renderApp('/identity?tab=ldap', { me: ADMIN_ME, api: api([directoryView()]) })

    const section = within(await screen.findByRole('region', { name: 'LDAP 目录' }))
    expect(await section.findByLabelText('URL')).toHaveValue('ldaps://ldap.example.org')
    expect(section.getByLabelText('URL')).toBeDisabled()
    expect(section.queryByRole('button', { name: '测试连接' })).not.toBeInTheDocument()
    expect(section.queryByRole('button', { name: '保存修改' })).not.toBeInTheDocument()
  })

  it('tests the saved directory by its id', async () => {
    let sent: Body | undefined
    const { user } = renderApp('/identity?tab=ldap', {
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

    await waitFor(() => {
      expect(sent).toEqual({ directory_id: 4 })
    })
  })

  it('tests unsaved changes as a form and asks for the password when the URL changes', async () => {
    let sent: Body | undefined
    const { user, fetchSpy } = renderApp('/identity?tab=ldap', {
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
    const { user } = renderApp('/identity?tab=ldap', {
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

  it('keeps a config-managed directory read-only and still offers the owner its tests', async () => {
    renderApp('/identity?tab=ldap', { api: api([directoryView({ managed_by: 'config' })]) })

    const section = await findLdapSection()
    expect(await section.findByLabelText('URL')).toBeDisabled()
    expect(section.queryByRole('button', { name: '保存修改' })).not.toBeInTheDocument()
    expect(screen.getByRole('region', { name: '测试目录' })).toBeInTheDocument()
  })
})
