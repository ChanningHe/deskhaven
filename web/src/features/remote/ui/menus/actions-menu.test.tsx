import { act, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { renderWithProviders } from '@/test/render-with-providers'

import { fakeDeps } from '../../connect/fake-deps'
import { ALL_ALLOWED } from '../../session/permissions'
import { RemoteDepsContext } from '../deps-context'
import { RemotePage } from '../remote-page'
import { openMenu } from './testing'

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
  return { ...view, f, session }
}

describe('actions menu: restart', () => {
  it('asks first, and sends nothing before the user confirms', async () => {
    const { user, session } = await activePage()
    await openMenu(user, '操作')
    await user.click(screen.getByRole('menuitem', { name: '重启远端' }))
    const dialog = await screen.findByRole('alertdialog')
    expect(session.restarts).toBe(0)

    await user.click(within(dialog).getByRole('button', { name: '重启' }))
    expect(session.restarts).toBe(1)
    await waitFor(() => {
      expect(screen.queryByRole('alertdialog')).toBeNull()
    })
  })

  it('is disabled while the device does not allow it, and enabled again with it', async () => {
    const { user, session } = await activePage()
    act(() => {
      session.permissions.set({ ...ALL_ALLOWED, restart: false })
    })
    await openMenu(user, '操作')
    const item = screen.getByRole('menuitem', { name: '重启远端' })
    expect(item).toHaveAttribute('aria-disabled', 'true')
    await user.click(item)
    expect(screen.queryByRole('alertdialog')).toBeNull()
    await user.keyboard('{Escape}')

    act(() => {
      session.permissions.set(ALL_ALLOWED)
    })
    await openMenu(user, '操作')
    expect(screen.getByRole('menuitem', { name: '重启远端' })).not.toHaveAttribute(
      'aria-disabled',
      'true',
    )
  })
})
