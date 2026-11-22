import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { ADMIN_ME } from '@/test/fixtures'
import { callsTo, errorEnvelope } from '@/test/mock-api'
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
  it('opens on the OIDC tab without mounting the LDAP form', async () => {
    renderApp('/identity', { api: API })

    expect(await screen.findByRole('region', { name: 'OIDC 提供方' })).toBeInTheDocument()
    expect(tab('OIDC')).toHaveAttribute('aria-selected', 'true')
    expect(screen.queryByRole('region', { name: 'LDAP 目录' })).not.toBeInTheDocument()
    expect(screen.queryByLabelText('名称')).not.toBeInTheDocument()
  })

  it('opens the tab named in the URL', async () => {
    renderApp('/identity?tab=ldap', { api: API })

    expect(await screen.findByRole('region', { name: 'LDAP 目录' })).toBeInTheDocument()
    expect(tab('LDAP')).toHaveAttribute('aria-selected', 'true')
    expect(screen.queryByRole('region', { name: 'OIDC 提供方' })).not.toBeInTheDocument()
  })

  it('falls back to the OIDC tab on an unknown tab', async () => {
    renderApp('/identity?tab=x', { api: API })

    expect(await screen.findByRole('region', { name: 'OIDC 提供方' })).toBeInTheDocument()
    expect(screen.queryByRole('region', { name: 'LDAP 目录' })).not.toBeInTheDocument()
  })

  it('puts the chosen tab in the URL, the default one as no parameter', async () => {
    const { user, router } = renderApp('/identity', { api: API })
    await screen.findByRole('region', { name: 'OIDC 提供方' })

    await user.click(tab('双因素认证'))
    await waitFor(() => {
      expect(router.state.location.search).toEqual({ tab: 'tfa' })
    })
    expect(await screen.findByRole('region', { name: '双因素认证' })).toBeInTheDocument()

    await user.click(tab('LDAP'))
    await waitFor(() => {
      expect(router.state.location.search).toEqual({ tab: 'ldap' })
    })

    await user.click(tab('OIDC'))
    await waitFor(() => {
      expect(router.state.location.search).toEqual({})
    })
  })

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

  it('switches tabs without asking when nothing changed', async () => {
    const { user, router } = renderApp('/identity?tab=ldap', { api: API })
    await screen.findByLabelText('名称')

    await user.click(tab('双因素认证'))

    await waitFor(() => {
      expect(router.state.location.search).toEqual({ tab: 'tfa' })
    })
    expect(screen.queryByRole('alertdialog', { name: LEAVE })).not.toBeInTheDocument()
  })

  it('shows the owner a summary on every tab', async () => {
    renderApp('/identity', { api: API })

    await waitFor(() => {
      expect(tab('OIDC')).toHaveAccessibleDescription('1 个启用')
    })
    await waitFor(() => {
      expect(tab('LDAP')).toHaveAccessibleDescription('已停用')
    })
    await waitFor(() => {
      expect(tab('双因素认证')).toHaveAccessibleDescription('管理员与所有者')
    })
  })

  it('summarizes a policy that enforces nothing in one short word', async () => {
    renderApp('/identity', {
      api: { ...API, [`GET ${SETTINGS_PATH}`]: { body: [tfaSetting({ value: 'off' })] } },
    })

    await waitFor(() => {
      expect(tab('双因素认证')).toHaveAccessibleDescription('不强制')
    })
  })

  it('summarizes no enabled provider and no directory', async () => {
    renderApp('/identity', {
      api: {
        ...API,
        [`GET ${PROVIDERS_PATH}`]: {
          body: { callback_url: null, items: [providerView(2, 'lab', { enabled: false })] },
        },
        [`GET ${DIRECTORIES_PATH}`]: { body: [] },
      },
    })

    await waitFor(() => {
      expect(tab('OIDC')).toHaveAccessibleDescription('未启用')
    })
    await waitFor(() => {
      expect(tab('LDAP')).toHaveAccessibleDescription('未配置')
    })
  })

  it('shows no summary while its list fails to load', async () => {
    renderApp('/identity?tab=ldap', {
      api: {
        ...API,
        [`GET ${DIRECTORIES_PATH}`]: { status: 400, body: errorEnvelope('bad_request') },
      },
    })

    await waitFor(() => {
      expect(tab('OIDC')).toHaveAccessibleDescription('1 个启用')
    })
    expect(await screen.findByRole('alert')).toBeInTheDocument()
    expect(tab('LDAP')).not.toHaveAttribute('aria-describedby')
  })

  it('gives admins no 2FA summary and never reads the settings, on any tab', async () => {
    const { user, fetchSpy } = renderApp('/identity', { me: ADMIN_ME, api: API })

    await waitFor(() => {
      expect(tab('LDAP')).toHaveAccessibleDescription('已停用')
    })
    expect(tab('OIDC')).toHaveAccessibleDescription('1 个启用')
    expect(tab('双因素认证')).not.toHaveAttribute('aria-describedby')

    await user.click(tab('LDAP'))
    await screen.findByRole('region', { name: 'LDAP 目录' })
    await user.click(tab('双因素认证'))
    expect(await screen.findByText('只有所有者可以修改此策略。')).toBeInTheDocument()
    expect(tab('双因素认证')).not.toHaveAttribute('aria-describedby')
    expect(callsTo(fetchSpy, SETTINGS_PATH)).toBe(0)
  })
})
