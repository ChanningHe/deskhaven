import { act, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { renderWithProviders } from '@/test/render-with-providers'

import { fakeDeps } from '../../connect/fake-deps'
import { OptionMessage_BoolOption } from '../../proto/gen/message_pb.js'
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

const header = () => within(screen.getByRole('banner'))

describe('actions menu', () => {
  it('offers view only, lock after the session, and restart', async () => {
    const { user } = await activePage()
    await openMenu(user, '操作')
    expect(screen.getByRole('menuitemcheckbox', { name: '仅查看' })).toBeInTheDocument()
    expect(screen.getByRole('menuitemcheckbox', { name: '会话结束后锁定' })).toBeInTheDocument()
    expect(screen.getByRole('menuitem', { name: '重启远端' })).toBeInTheDocument()
  })

  it('starts with both switches off and nothing marked on the bar', async () => {
    const { user } = await activePage()
    expect(header().queryByText('仅查看')).toBeNull()
    await openMenu(user, '操作')
    for (const name of ['仅查看', '会话结束后锁定']) {
      expect(screen.getByRole('menuitemcheckbox', { name })).toHaveAttribute(
        'aria-checked',
        'false',
      )
    }
  })

  it('view only: tells the controller, marks the bar, and shows checked next time', async () => {
    const { user, session } = await activePage()
    session.patches.length = 0
    await openMenu(user, '操作')
    await user.click(screen.getByRole('menuitemcheckbox', { name: '仅查看' }))
    expect(session.patches).toMatchObject([{ viewOnly: true }])
    expect(header().getByText('仅查看')).toBeInTheDocument()

    await openMenu(user, '操作')
    expect(screen.getByRole('menuitemcheckbox', { name: '仅查看' })).toHaveAttribute(
      'aria-checked',
      'true',
    )
    await user.click(screen.getByRole('menuitemcheckbox', { name: '仅查看' }))
    expect(session.patches).toMatchObject([{ viewOnly: true }, { viewOnly: false }])
    expect(header().queryByText('仅查看')).toBeNull()
  })

  it('lock after the session: tells the controller the lock field alone', async () => {
    const { user, session } = await activePage()
    session.patches.length = 0
    await openMenu(user, '操作')
    await user.click(screen.getByRole('menuitemcheckbox', { name: '会话结束后锁定' }))
    expect(session.patches).toHaveLength(1)
    expect(session.patches[0]).not.toHaveProperty('viewOnly')
    expect(session.patches[0]?.peer).toMatchObject({
      lockAfterSessionEnd: OptionMessage_BoolOption.Yes,
    })
  })

  it('has no lock to switch while viewing only: none is sent then', async () => {
    const { user } = await activePage()
    await openMenu(user, '操作')
    await user.click(screen.getByRole('menuitemcheckbox', { name: '仅查看' }))
    await openMenu(user, '操作')
    expect(screen.getByRole('menuitemcheckbox', { name: '会话结束后锁定' })).toHaveAttribute(
      'aria-disabled',
      'true',
    )
  })
})

describe('actions menu: restart', () => {
  it('asks first, and sends nothing before the user confirms', async () => {
    const { user, session } = await activePage()
    await openMenu(user, '操作')
    await user.click(screen.getByRole('menuitem', { name: '重启远端' }))
    const dialog = await screen.findByRole('alertdialog')
    expect(within(dialog).getByText('重启远端设备？')).toBeInTheDocument()
    expect(session.restarts).toBe(0)

    await user.click(within(dialog).getByRole('button', { name: '重启' }))
    expect(session.restarts).toBe(1)
    await waitFor(() => {
      expect(screen.queryByRole('alertdialog')).toBeNull()
    })
  })

  it('sends nothing when the user cancels', async () => {
    const { user, session } = await activePage()
    await openMenu(user, '操作')
    await user.click(screen.getByRole('menuitem', { name: '重启远端' }))
    await user.click(
      within(await screen.findByRole('alertdialog')).getByRole('button', { name: '取消' }),
    )
    await waitFor(() => {
      expect(screen.queryByRole('alertdialog')).toBeNull()
    })
    expect(session.restarts).toBe(0)
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

  it('says why the session ended when the device went down for it', async () => {
    const { session } = await activePage()
    act(() => {
      session.end({ by: 'peer', reason: 'the device is restarting' })
    })
    expect(await screen.findByRole('alert')).toHaveTextContent('远端正在重启')
  })
})
