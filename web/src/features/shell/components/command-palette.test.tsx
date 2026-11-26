import { screen, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { ADMIN_ME } from '@/test/fixtures'
import { renderApp } from '@/test/render-app'

import { isPaletteShortcut } from '../command-shortcut'

async function renderConsole(options: Parameters<typeof renderApp>[1] = {}) {
  const app = renderApp('/', options)
  await screen.findByRole('heading', { level: 1, name: '概览' })
  return app
}

const palette = () => screen.findByRole('dialog', { name: '命令面板' })

describe('command palette', () => {
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

  it('matches the physical K key, whatever the keyboard layout', () => {
    const press = (init: KeyboardEventInit) => isPaletteShortcut(new KeyboardEvent('keydown', init))

    expect(press({ key: 'k', code: 'KeyK', ctrlKey: true })).toBe(true)
    expect(press({ key: 'л', code: 'KeyK', metaKey: true })).toBe(true)
    expect(press({ key: 'k', code: 'KeyK', ctrlKey: true, altKey: true })).toBe(false)
    expect(press({ key: 'k', code: 'KeyK' })).toBe(false)
    expect(press({ key: 'k', code: 'KeyJ', ctrlKey: true })).toBe(false)
  })
})
