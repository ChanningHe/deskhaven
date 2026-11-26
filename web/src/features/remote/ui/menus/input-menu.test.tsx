import { act, screen, waitFor } from '@testing-library/react'
import type { UserEvent } from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { renderWithProviders } from '@/test/render-with-providers'

import { fakeDeps } from '../../connect/fake-deps'
import type { SessionAction } from '../../session/messages'
import { RemoteDepsContext } from '../deps-context'
import { RemotePage } from '../remote-page'
import { openMenu } from './testing'

async function openSendKeys(user: UserEvent) {
  screen.getByRole('menuitem', { name: '发送按键' }).focus()
  await user.keyboard('{ArrowRight}')
}

beforeEach(() => {
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null)
  HTMLElement.prototype.setPointerCapture = vi.fn()
})

async function activePage() {
  const f = fakeDeps()
  const view = renderWithProviders(
    <RemoteDepsContext.Provider value={f.deps}>
      <RemotePage deviceId="123456789" />
    </RemoteDepsContext.Provider>,
  )
  await waitFor(() => {
    expect(f.sessions).toHaveLength(1)
  })
  const [session] = f.sessions
  if (!session) throw new Error('no session started')
  act(() => {
    session.activate()
  })
  return { ...view, session }
}

const ITEMS: [string, SessionAction][] = [
  ['Ctrl+Alt+Del', 'ctrl_alt_del'],
  ['锁屏', 'lock_screen'],
  ['Win 键', 'win'],
  ['Alt+Tab', 'alt_tab'],
  ['PrintScreen', 'print_screen'],
]

describe('send keys menu', () => {
  it.each(ITEMS)('sends %s', async (name, kind) => {
    const { user, session } = await activePage()
    await openMenu(user, '输入')
    await openSendKeys(user)
    await user.click(await screen.findByRole('menuitem', { name }))
    expect(session.actions).toEqual([kind])
  })

  it('is disabled while the device has the keyboard off, and enabled again with it', async () => {
    const { user, session } = await activePage()
    act(() => {
      session.permissions.set({ keyboard: false, clipboard: true, restart: true, audio: true })
    })
    await openMenu(user, '输入')
    expect(screen.getByRole('menuitem', { name: '发送按键' })).toHaveAttribute(
      'aria-disabled',
      'true',
    )
    await user.keyboard('{Escape}')
    act(() => {
      session.permissions.set({ keyboard: true, clipboard: true, restart: true, audio: true })
    })
    await openMenu(user, '输入')
    expect(screen.getByRole('menuitem', { name: '发送按键' })).not.toHaveAttribute(
      'aria-disabled',
      'true',
    )
  })
})
