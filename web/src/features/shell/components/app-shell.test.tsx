import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { LANGUAGE_STORAGE_KEY } from '@/i18n'
import { THEME_STORAGE_KEY } from '@/lib/theme/theme'
import { renderApp } from '@/test/render-app'

import { SIDEBAR_COLLAPSED_STORAGE_KEY } from '../use-sidebar-collapsed'

async function renderShell(path = '/') {
  const app = renderApp(path)
  await screen.findByRole('navigation', { name: '主导航' })
  return app
}

describe('app shell', () => {
  it('renders landmarks and every P0 nav item', async () => {
    await renderShell()

    expect(screen.getByRole('banner')).toBeInTheDocument()
    expect(screen.getByRole('main')).toBeInTheDocument()
    const nav = screen.getByRole('navigation', { name: '主导航' })
    const labels = within(nav)
      .getAllByRole('link')
      .map((link) => link.textContent)
    expect(labels).toEqual([
      '概览',
      '设备',
      '用户',
      '分组',
      '地址簿',
      '策略',
      '审计',
      '身份认证',
      '客户端配置',
      'API Token',
      '设置',
    ])
  })

  it('navigates to a page that shows its header and empty state', async () => {
    const { user } = await renderShell()

    await user.click(screen.getByRole('link', { name: '设备' }))

    expect(await screen.findByRole('heading', { level: 1, name: '设备' })).toBeInTheDocument()
    expect(screen.getByRole('heading', { name: '暂无设备' })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: '设备' })).toHaveAttribute('aria-current', 'page')
  })

  it('collapses the sidebar and persists the state', async () => {
    const { user, unmount } = await renderShell()

    const toggle = screen.getByRole('button', { name: '收起侧栏' })
    expect(toggle).toHaveAttribute('aria-expanded', 'true')

    await user.click(toggle)

    const expand = screen.getByRole('button', { name: '展开侧栏' })
    expect(expand).toHaveAttribute('aria-expanded', 'false')
    expect(document.getElementById('app-sidebar')).toHaveAttribute('data-collapsed', 'true')
    // Icon-only links keep an accessible name.
    expect(screen.getByRole('link', { name: '设备' })).toBeInTheDocument()
    expect(window.localStorage.getItem(SIDEBAR_COLLAPSED_STORAGE_KEY)).toBe('true')

    unmount()
    await renderShell()
    expect(screen.getByRole('button', { name: '展开侧栏' })).toBeInTheDocument()
  })

  it('opens the mobile drawer and closes it after navigating', async () => {
    const { user } = await renderShell()

    await user.click(screen.getByRole('button', { name: '打开导航' }))
    const drawer = await screen.findByRole('dialog')
    await user.click(within(drawer).getByRole('link', { name: '审计' }))

    expect(await screen.findByRole('heading', { level: 1, name: '审计' })).toBeInTheDocument()
    await waitFor(() => {
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    })
  })

  it('switches language and re-renders nav labels', async () => {
    const { user } = await renderShell()

    await user.click(screen.getByRole('button', { name: '语言' }))
    await user.click(await screen.findByRole('menuitemradio', { name: 'English' }))

    expect(await screen.findByRole('link', { name: 'Devices' })).toBeInTheDocument()
    expect(screen.queryByRole('link', { name: '设备' })).not.toBeInTheDocument()
    expect(window.localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe('en')
    expect(document.documentElement.lang).toBe('en')
  })

  it('applies and persists the selected theme', async () => {
    const { user } = await renderShell()

    await user.click(screen.getByRole('button', { name: '主题' }))
    await user.click(await screen.findByRole('menuitemradio', { name: '深色' }))

    expect(document.documentElement).toHaveClass('dark')
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe('dark')
    await waitFor(() => {
      expect(screen.queryByRole('menu')).not.toBeInTheDocument()
    })

    await user.click(screen.getByRole('button', { name: '主题' }))
    await user.click(await screen.findByRole('menuitemradio', { name: '跟随系统' }))

    expect(document.documentElement).not.toHaveClass('dark')
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe('system')
  })

  it('restores a persisted theme on load', async () => {
    window.localStorage.setItem(THEME_STORAGE_KEY, 'dark')

    await renderShell()

    expect(document.documentElement).toHaveClass('dark')
  })
})

describe('routes outside the shell', () => {
  it('renders /login without the console navigation', async () => {
    renderApp('/login')

    expect(await screen.findByRole('heading', { name: '登录 deskhaven' })).toBeInTheDocument()
    expect(screen.queryByRole('navigation')).not.toBeInTheDocument()
  })

  it('renders a not-found page for unknown paths', async () => {
    renderApp('/no-such-page')

    expect(await screen.findByRole('heading', { name: '页面不存在' })).toBeInTheDocument()
  })
})
