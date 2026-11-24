import { screen, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { renderWithProviders } from '@/test/render-with-providers'

import { limitHints } from './limit-hints'
import { LimitsHint } from './limits-hint'

describe('limitHints', () => {
  it('sends shortcuts and Esc in full screen where the keyboard can be locked', () => {
    expect(limitHints({ keyboardLock: true, clipboard: 'granted' })).toEqual([
      'remote.limits.shortcutsFullscreen',
      'remote.limits.escape',
      'remote.limits.clipboardBoth',
    ])
  })

  it('points to the send keys menu, and says nothing of Esc, without Keyboard Lock', () => {
    expect(limitHints({ keyboardLock: false, clipboard: 'granted' })).toEqual([
      'remote.limits.shortcutsMenu',
      'remote.limits.clipboardBoth',
    ])
  })

  it.each([
    ['granted', 'remote.limits.clipboardBoth'],
    ['prompt', 'remote.limits.clipboardAsk'],
    [null, 'remote.limits.clipboardAsk'],
    ['denied', 'remote.limits.clipboardDenied'],
    ['unsupported', 'remote.limits.clipboardUnsupported'],
  ] as const)('reads clipboard access %s as %s', (clipboard, line) => {
    expect(limitHints({ keyboardLock: false, clipboard }).at(-1)).toBe(line)
  })
})

describe('LimitsHint', () => {
  function tip() {
    return screen.findByRole('list', { name: '使用限制' })
  }

  it('lists the limits on hover', async () => {
    const { user } = renderWithProviders(<LimitsHint keyboardLock={false} clipboard="denied" />)
    await user.hover(screen.getByRole('button', { name: '使用限制' }))
    const items = within(await tip()).getAllByRole('listitem')
    expect(items.map((i) => i.textContent)).toEqual([
      '此浏览器无法捕获 Win、Alt+Tab、Ctrl+W 等系统快捷键，请用“输入”菜单中的“发送按键”。',
      '浏览器禁止此页面读取剪贴板：只会同步设备上复制的文本，本机的文本请用“剪贴板”菜单中的“发送剪贴板”。',
    ])
  })

  it('lists the limits on keyboard focus', async () => {
    const { user } = renderWithProviders(<LimitsHint keyboardLock clipboard="granted" />)
    await user.tab()
    expect(screen.getByRole('button', { name: '使用限制' })).toHaveFocus()
    expect(within(await tip()).getAllByRole('listitem')).toHaveLength(3)
  })
})
