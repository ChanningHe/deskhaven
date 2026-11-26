import { screen, waitFor, within } from '@testing-library/react'
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
  ] as const)('reads clipboard access %s as %s', (clipboard, line) => {
    expect(limitHints({ keyboardLock: false, clipboard }).at(-1)).toBe(line)
  })

  it('says nothing of the clipboard where the browser cannot report its access', () => {
    expect(limitHints({ keyboardLock: false, clipboard: 'unsupported' })).toEqual([
      'remote.limits.shortcutsMenu',
    ])
  })
})

describe('LimitsHint', () => {
  const LOCKLESS_DENIED = [
    '此浏览器无法捕获 Win、Alt+Tab、Ctrl+W 等系统快捷键，请用“输入”菜单中的“发送按键”。',
    '浏览器禁止此页面读取剪贴板：设备上复制的文本仍会同步，本机文本在远程画面按 Ctrl+V 发送。',
  ]

  function trigger() {
    return screen.getByRole('button', { name: '使用限制' })
  }

  function lines(dialog: HTMLElement) {
    return within(dialog)
      .getAllByRole('listitem')
      .map((i) => i.textContent)
  }

  it('opens on a click, names and describes itself, and closes on Esc', async () => {
    const { user } = renderWithProviders(<LimitsHint keyboardLock={false} clipboard="denied" />)
    expect(screen.queryByRole('dialog')).toBeNull()
    await user.click(trigger())
    const dialog = await screen.findByRole('dialog', { name: '使用限制' })
    expect(lines(dialog)).toEqual(LOCKLESS_DENIED)
    expect(dialog).toHaveAccessibleDescription(LOCKLESS_DENIED.join(' '))
    await user.keyboard('{Escape}')
    await waitFor(() => {
      expect(screen.queryByRole('dialog')).toBeNull()
    })
    expect(trigger()).toHaveFocus()
  })

  it('closes on a second click and on a click outside', async () => {
    const { user } = renderWithProviders(
      <>
        <LimitsHint keyboardLock clipboard="granted" />
        <p>outside</p>
      </>,
    )
    await user.click(trigger())
    expect(await screen.findByRole('dialog')).toBeInTheDocument()
    await user.click(trigger())
    await waitFor(() => {
      expect(screen.queryByRole('dialog')).toBeNull()
    })
    await user.click(trigger())
    expect(await screen.findByRole('dialog')).toBeInTheDocument()
    await user.click(screen.getByText('outside'))
    await waitFor(() => {
      expect(screen.queryByRole('dialog')).toBeNull()
    })
  })

  it('opens from the keyboard', async () => {
    const { user } = renderWithProviders(<LimitsHint keyboardLock clipboard="granted" />)
    await user.tab()
    expect(trigger()).toHaveFocus()
    await user.keyboard('{Enter}')
    expect(lines(await screen.findByRole('dialog'))).toHaveLength(3)
  })
})
