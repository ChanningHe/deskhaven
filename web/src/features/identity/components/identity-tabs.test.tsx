import { createBrowserHistory, RouterProvider } from '@tanstack/react-router'
import { render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it } from 'vitest'

import { createApp } from '@/app/create-app'
import { AppProviders } from '@/app/providers'
import { ADMIN_ME, OWNER_ME } from '@/test/fixtures'
import { callsTo, mockApi } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import {
  DIRECTORIES_PATH,
  directoryView,
  PROVIDERS_PATH,
  providerView,
  SETTINGS_PATH,
  tfaSetting,
} from '../test-data'

const API = {
  [`GET ${PROVIDERS_PATH}`]: {
    body: {
      callback_url: null,
      items: [providerView(1, 'corp'), providerView(2, 'lab', { enabled: false })],
    },
  },
  [`GET ${DIRECTORIES_PATH}`]: { body: [directoryView({ enabled: false })] },
  [`GET ${SETTINGS_PATH}`]: { body: [tfaSetting({ value: 'admins' })] },
}

const LEAVE = '放弃未保存的修改？'

function tab(name: string) {
  return screen.getByRole('tab', { name })
}

describe('identity page: tabs', () => {
  it('asks before switching tabs with an unsaved LDAP edit', async () => {
    const { user, router } = renderApp('/identity?tab=ldap', { api: API })
    await user.type(await screen.findByLabelText('名称'), '-eu')

    await user.click(tab('双因素认证'))
    let dialog = await screen.findByRole('alertdialog', { name: LEAVE })
    await user.click(within(dialog).getByRole('button', { name: '取消' }))

    await waitFor(() => {
      expect(screen.queryByRole('alertdialog', { name: LEAVE })).not.toBeInTheDocument()
    })
    expect(router.state.location.search).toEqual({ tab: 'ldap' })
    expect(tab('LDAP')).toHaveAttribute('aria-selected', 'true')
    expect(screen.getByLabelText('名称')).toHaveValue('corp-eu')

    await user.click(tab('双因素认证'))
    dialog = await screen.findByRole('alertdialog', { name: LEAVE })
    await user.click(within(dialog).getByRole('button', { name: '放弃修改' }))

    await waitFor(() => {
      expect(router.state.location.search).toEqual({ tab: 'tfa' })
    })
    expect(await screen.findByRole('region', { name: '双因素认证' })).toBeInTheDocument()
    expect(screen.queryByLabelText('名称')).not.toBeInTheDocument()
  })

  it('never reads the owner-only settings for an admin, on any tab', async () => {
    const { user, fetchSpy } = renderApp('/identity', { me: ADMIN_ME, api: API })

    await user.click(await screen.findByRole('tab', { name: 'LDAP' }))
    await screen.findByRole('region', { name: 'LDAP 目录' })
    await user.click(tab('双因素认证'))
    await screen.findByText('只有所有者可以修改此策略。')
    expect(callsTo(fetchSpy, SETTINGS_PATH)).toBe(0)
  })
})

/**
 * The browser's back and forward arrive as popstate, which only the browser history turns
 * into a blocked navigation (the memory history of `renderApp` never asks the blockers).
 */
describe('identity page: browser back and forward', () => {
  let destroy: (() => void) | undefined

  afterEach(() => {
    destroy?.()
    destroy = undefined
  })

  function renderInBrowser(path: string) {
    mockApi({ 'GET /admin/api/v1/me': { body: OWNER_ME }, ...API })
    window.history.replaceState(null, '', path)
    const history = createBrowserHistory()
    destroy = history.destroy
    const { queryClient, router } = createApp(history)
    render(
      <AppProviders queryClient={queryClient}>
        <RouterProvider router={router} />
      </AppProviders>,
    )
    return { router, user: userEvent.setup() }
  }

  it('asks before going back from an unsaved LDAP edit, and cancelling keeps it', async () => {
    const { user, router } = renderInBrowser('/identity')
    await screen.findByRole('region', { name: 'OIDC 提供方' })
    await user.click(tab('LDAP'))
    await user.type(await screen.findByLabelText('名称'), '-eu')

    router.history.back()
    const dialog = await screen.findByRole('alertdialog', { name: LEAVE })
    await user.click(within(dialog).getByRole('button', { name: '取消' }))

    await waitFor(() => {
      expect(screen.queryByRole('alertdialog', { name: LEAVE })).not.toBeInTheDocument()
    })
    expect(router.state.location.search).toEqual({ tab: 'ldap' })
    expect(window.location.search).toBe('?tab=ldap')
    expect(tab('LDAP')).toHaveAttribute('aria-selected', 'true')
    expect(screen.getByLabelText('名称')).toHaveValue('corp-eu')
  })
})
