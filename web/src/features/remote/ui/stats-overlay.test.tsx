import { act, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { renderWithProviders } from '@/test/render-with-providers'

import { fakeDeps } from '../connect/fake-deps'
import { NO_STATS } from '../session/stats'
import { RemoteDepsContext } from './deps-context'
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

describe('stats overlay', () => {
  it('is off until the toolbar switch turns it on, and off again', async () => {
    const { user } = await activePage()
    const toggle = screen.getByRole('button', { name: '统计信息' })
    expect(toggle).toHaveAttribute('aria-pressed', 'false')
    expect(screen.queryByLabelText('连接统计')).toBeNull()

    await user.click(toggle)
    expect(toggle).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByLabelText('连接统计')).toBeInTheDocument()

    await user.click(toggle)
    expect(screen.queryByLabelText('连接统计')).toBeNull()
  })

  it("shows the session's figures and the controller's transport, raw values readable", async () => {
    const { user, session } = await activePage()
    await user.click(screen.getByRole('button', { name: '统计信息' }))
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
  })
})
