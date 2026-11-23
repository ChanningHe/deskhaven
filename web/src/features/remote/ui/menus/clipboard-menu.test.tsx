import { act, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { renderWithProviders } from '@/test/render-with-providers'

import { RemoteController } from '../../connect/controller'
import { fakeDeps } from '../../connect/fake-deps'
import { SETTINGS_KEY, type RemoteSettings } from '../../settings/settings'
import { RemoteDepsContext } from '../deps-context'
import { RemotePage } from '../remote-page'
import { openMenu } from './testing'

let local = 'before the session'
const readText = vi.fn(() => Promise.resolve(local))
const writeText = vi.fn((text: string) => {
  local = text
  return Promise.resolve()
})

beforeEach(() => {
  local = 'before the session'
  readText.mockClear()
  writeText.mockClear()
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null)
  HTMLElement.prototype.setPointerCapture = vi.fn()
  vi.spyOn(document, 'hasFocus').mockReturnValue(true)
})

afterEach(() => {
  Reflect.deleteProperty(navigator, 'permissions')
  Reflect.deleteProperty(navigator, 'clipboard')
})

async function activePage(access: PermissionState, stored?: Partial<RemoteSettings>) {
  const f = fakeDeps()
  if (stored) f.storage.setItem(SETTINGS_KEY, JSON.stringify({ v: 1, settings: stored }))
  const updates = vi.spyOn(RemoteController.prototype, 'updateSettings')
  const view = renderWithProviders(
    <RemoteDepsContext.Provider value={f.deps}>
      <RemotePage deviceId="123456789" />
    </RemoteDepsContext.Provider>,
  )
  // After userEvent.setup, which installs a clipboard of its own.
  Object.defineProperty(navigator, 'clipboard', {
    value: { readText, writeText },
    configurable: true,
  })
  const status = Object.assign(new EventTarget(), { state: access })
  Object.defineProperty(navigator, 'permissions', {
    value: { query: () => Promise.resolve(status) },
    configurable: true,
  })
  await waitFor(() => {
    expect(f.sessions).toHaveLength(1)
  })
  const [session] = f.sessions
  if (!session) throw new Error('no session started')
  act(() => {
    session.activate()
  })
  return { ...view, session, updates }
}

const syncSwitch = () => screen.getByRole('menuitemcheckbox', { name: '自动同步剪贴板' })

describe('clipboard menu: automatic sync', () => {
  it('turning it on reads the clipboard in that click, only as a baseline', async () => {
    const { user, session, updates } = await activePage('prompt', { clipboardSync: false })
    await openMenu(user, '剪贴板')
    expect(syncSwitch()).toHaveAttribute('aria-checked', 'false')
    await user.click(syncSwitch())
    expect(readText).toHaveBeenCalledTimes(1)
    expect(updates).toHaveBeenLastCalledWith({ clipboardSync: true })
    expect(session.pasted).toEqual([])

    await openMenu(user, '剪贴板')
    await user.click(syncSwitch())
    expect(updates).toHaveBeenLastCalledWith({ clipboardSync: false })
    expect(readText).toHaveBeenCalledTimes(1)
  })

  it('offers to ask for access while it is on and access is only promptable', async () => {
    const { user } = await activePage('prompt')
    await openMenu(user, '剪贴板')
    expect(syncSwitch()).toHaveAttribute('aria-checked', 'true')
    await user.click(screen.getByRole('menuitem', { name: '允许读取剪贴板' }))
    expect(readText).toHaveBeenCalledTimes(1)
  })

  it('says when the browser denies reading, and can still be turned off', async () => {
    const { user, updates } = await activePage('denied')
    await openMenu(user, '剪贴板')
    expect(syncSwitch()).toHaveAccessibleDescription(/浏览器禁止此页面读取剪贴板/)
    expect(screen.queryByRole('menuitem', { name: '允许读取剪贴板' })).toBeNull()
    await user.click(syncSwitch())
    expect(updates).toHaveBeenLastCalledWith({ clipboardSync: false })
  })

  it('tells once what it sent and what it copied from the device', async () => {
    const { session } = await activePage('granted')
    await waitFor(() => {
      expect(readText).toHaveBeenCalled()
    })
    local = 'copied here'
    act(() => {
      window.dispatchEvent(new FocusEvent('focus'))
    })
    expect(await screen.findByText('已发送到设备剪贴板。')).toBeInTheDocument()
    expect(session.pasted).toEqual(['copied here'])

    act(() => {
      session.remoteClipboard.set('from the device')
    })
    expect(await screen.findByText('已复制设备剪贴板的内容。')).toBeInTheDocument()
    expect(writeText).toHaveBeenCalledWith('from the device')
  })
})
