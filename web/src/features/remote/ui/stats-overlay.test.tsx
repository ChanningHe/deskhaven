import { act, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { renderWithProviders } from '@/test/render-with-providers'

import { fakeDeps } from '../connect/fake-deps'
import { NO_STATS } from '../session/stats'
import { RemoteDepsContext } from './deps-context'
import { openMenu } from './menus/testing'
import { RemotePage } from './remote-page'

beforeEach(() => {
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null)
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

async function showStats(user: Awaited<ReturnType<typeof activePage>>['user']) {
  await openMenu(user, '显示')
  await user.click(screen.getByRole('menuitemcheckbox', { name: '统计信息' }))
}

describe('stats overlay', () => {
  it('is off until the display menu turns it on, and off again', async () => {
    const { user } = await activePage()
    const item = () => screen.getByRole('menuitemcheckbox', { name: '统计信息' })
    await openMenu(user, '显示')
    expect(item()).toHaveAttribute('aria-checked', 'false')
    expect(screen.queryByLabelText('连接统计')).toBeNull()

    await user.click(item())
    expect(screen.getByLabelText('连接统计')).toBeInTheDocument()
    await openMenu(user, '显示')
    expect(item()).toHaveAttribute('aria-checked', 'true')

    await user.click(item())
    expect(screen.queryByLabelText('连接统计')).toBeNull()
  })

  it("shows the session's figures and the controller's transport, raw values readable", async () => {
    const { user, session } = await activePage()
    await showStats(user)
    act(() => {
      session.stats.set({
        ...NO_STATS,
        fps: 29.5,
        kbps: 1840,
        delayMs: 83,
        keyRequests: 2,
        width: 1280,
        height: 800,
        codec: 'vp09.00.10.08',
      })
    })
    const overlay = screen.getByLabelText('连接统计')
    const stat = (name: string) => overlay.querySelector(`[data-stat="${name}"]`)
    expect(stat('fps')).toHaveAttribute('data-value', '29.5')
    expect(stat('kbps')).toHaveTextContent('1840 kbps')
    expect(stat('delayMs')).toHaveTextContent('83 ms')
    expect(stat('decodeMs')).toHaveAttribute('data-value', '')
    expect(stat('keyRequests')).toHaveAttribute('data-value', '2')
    expect(stat('size')).toHaveTextContent('1280×800')
    expect(stat('codec')).toHaveTextContent('vp09.00.10.08')
    expect(within(overlay).getByText('中继')).toBe(stat('transport'))
    expect(stat('quality')).toHaveAttribute('data-value', '')
  })

  it.each([
    ['balanced', 'balanced', '均衡'],
    [{ custom: 18 }, '18', '自定义 18'],
  ] as const)('shows the quality the session asks for: %o', async (quality, value, text) => {
    const { user, session } = await activePage()
    await showStats(user)
    act(() => {
      session.stats.set({ ...NO_STATS, quality })
    })
    const stat = screen.getByLabelText('连接统计').querySelector('[data-stat="quality"]')
    expect(stat).toHaveAttribute('data-value', value)
    expect(stat).toHaveTextContent(text)
  })
})
