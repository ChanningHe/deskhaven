import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { renderApp } from '@/test/render-app'

import {
  DIRECTORIES_PATH,
  directoryView,
  PROVIDERS_PATH,
  SETTINGS_PATH,
  tfaSetting,
} from '../test-data'

const API = {
  [`GET ${PROVIDERS_PATH}`]: { body: { callback_url: null, items: [] } },
  [`GET ${DIRECTORIES_PATH}`]: { body: [directoryView()] },
  [`GET ${SETTINGS_PATH}`]: { body: [tfaSetting()] },
}

const LEAVE = '放弃未保存的修改？'

describe('identity page: unsaved changes', () => {
  it('asks before leaving with an unsaved LDAP edit and keeps it on cancel', async () => {
    const { user, router } = renderApp('/identity', { api: API })
    const name = await screen.findByLabelText('名称')
    await user.type(name, '-eu')

    void router.navigate({ to: '/groups' })

    const dialog = await screen.findByRole('alertdialog', { name: LEAVE })
    await user.click(within(dialog).getByRole('button', { name: '取消' }))
    expect(router.state.location.pathname).toBe('/identity')
    expect(screen.getByLabelText('名称')).toHaveValue('corp-eu')
  })

  it('asks before leaving with an unsaved 2FA choice and keeps it on cancel', async () => {
    const { user, router } = renderApp('/identity', { api: API })
    const select = await screen.findByRole('combobox', { name: '强制双因素认证的范围' })
    await user.selectOptions(select, 'all')

    void router.navigate({ to: '/groups' })

    const dialog = await screen.findByRole('alertdialog', { name: LEAVE })
    await user.click(within(dialog).getByRole('button', { name: '取消' }))
    expect(router.state.location.pathname).toBe('/identity')
    expect(screen.getByRole('combobox', { name: '强制双因素认证的范围' })).toHaveValue('all')
  })

  it('leaves without asking when nothing changed', async () => {
    const { router } = renderApp('/identity', { api: API })
    await screen.findByLabelText('名称')
    await screen.findByRole('combobox', { name: '强制双因素认证的范围' })

    void router.navigate({ to: '/groups' })

    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/groups')
    })
    expect(screen.queryByRole('alertdialog', { name: LEAVE })).not.toBeInTheDocument()
  })

  it('asks once when both the LDAP form and the 2FA policy have unsaved edits', async () => {
    const { user, router } = renderApp('/identity', { api: API })
    await user.type(await screen.findByLabelText('名称'), '-eu')
    await user.selectOptions(
      await screen.findByRole('combobox', { name: '强制双因素认证的范围' }),
      'all',
    )

    void router.navigate({ to: '/groups' })

    const dialog = await screen.findByRole('alertdialog', { name: LEAVE })
    await user.click(within(dialog).getByRole('button', { name: '放弃修改' }))
    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/groups')
    })
  })
})
