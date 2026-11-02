import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { ADMIN_ME } from '@/test/fixtures'
import { renderApp } from '@/test/render-app'

async function renderConsole(options: Parameters<typeof renderApp>[1] = {}) {
  const app = renderApp('/', options)
  await screen.findByRole('heading', { level: 1, name: '概览' })
  return app
}

const palette = () => screen.findByRole('dialog', { name: '命令面板' })

describe('command palette', () => {
  it('opens from the top bar button, which advertises the shortcut', async () => {
    const { user } = await renderConsole()

    const trigger = screen.getByRole('button', { name: '搜索…' })
    expect(trigger).toHaveAttribute('aria-keyshortcuts', 'Meta+K Control+K')
    await user.click(trigger)

    const dialog = await palette()
    expect(within(dialog).getByRole('combobox', { name: '搜索页面和操作' })).toHaveFocus()
  })

  it('opens with Ctrl+K, filters as you type and runs the highlighted item with Enter', async () => {
    const { user, router } = await renderConsole()

    await user.keyboard('{Control>}k{/Control}')
    const dialog = await palette()
    await user.keyboard('审计')

    expect(
      within(dialog)
        .getAllByRole('option')
        .map((option) => option.textContent),
    ).toEqual(['审计'])
    await user.keyboard('{Enter}')

    expect(await screen.findByRole('heading', { level: 1, name: '审计' })).toBeInTheDocument()
    expect(router.state.location.pathname).toBe('/audit')
    await waitFor(() => {
      expect(screen.queryByRole('dialog', { name: '命令面板' })).not.toBeInTheDocument()
    })
  })

  it('says so when nothing matches', async () => {
    const { user } = await renderConsole()

    await user.keyboard('{Meta>}k{/Meta}')
    const dialog = await palette()
    await user.keyboard('zzz-no-such-command')

    expect(within(dialog).queryAllByRole('option')).toHaveLength(0)
    expect(within(dialog).getByText('没有匹配的结果')).toBeVisible()
  })

  it('switches the theme from the preferences group', async () => {
    const { user } = await renderConsole()

    await user.click(screen.getByRole('button', { name: '搜索…' }))
    await user.click(within(await palette()).getByRole('option', { name: '主题: 深色' }))

    expect(document.documentElement).toHaveClass('dark')
  })

  it('offers only the pages the role may open', async () => {
    const { user } = await renderConsole({ me: ADMIN_ME })

    await user.click(screen.getByRole('button', { name: '搜索…' }))
    const options = within(await palette())
      .getAllByRole('option')
      .map((option) => option.textContent)

    expect(options).toContain('设备')
    expect(options).toContain('我的')
    expect(options).not.toContain('设置')
  })

  it('closes with Escape and returns focus to the trigger', async () => {
    const { user } = await renderConsole()
    const trigger = screen.getByRole('button', { name: '搜索…' })

    await user.click(trigger)
    await palette()
    await user.keyboard('{Escape}')

    await waitFor(() => {
      expect(screen.queryByRole('dialog', { name: '命令面板' })).not.toBeInTheDocument()
    })
    expect(trigger).toHaveFocus()
  })
})
