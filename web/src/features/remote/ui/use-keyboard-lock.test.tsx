import { act, fireEvent, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { renderWithProviders } from '@/test/render-with-providers'

import { fakeDeps } from '../connect/fake-deps'
import { RemoteDepsContext } from './deps-context'
import { RemotePage } from './remote-page'

const lock = vi.fn(() => Promise.resolve())
const unlock = vi.fn()

beforeEach(() => {
  lock.mockClear()
  unlock.mockClear()
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null)
  HTMLElement.prototype.setPointerCapture = vi.fn()
  document.documentElement.requestFullscreen = vi.fn(() => Promise.resolve())
  document.exitFullscreen = vi.fn(() => Promise.resolve())
})

afterEach(() => {
  Reflect.deleteProperty(navigator, 'keyboard')
  Object.defineProperty(document, 'fullscreenElement', { value: null, configurable: true })
})

function setFullscreen(on: boolean) {
  Object.defineProperty(document, 'fullscreenElement', {
    value: on ? document.documentElement : null,
    configurable: true,
  })
  fireEvent(document, new Event('fullscreenchange'))
}

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
  return { ...view, session, f }
}

describe('keyboard lock', () => {
  it('locks every key on entering full screen and unlocks on leaving it', async () => {
    Object.defineProperty(navigator, 'keyboard', { value: { lock, unlock }, configurable: true })
    await activePage()
    expect(lock).not.toHaveBeenCalled()
    setFullscreen(true)
    expect(lock).toHaveBeenCalledTimes(1)
    expect(lock).toHaveBeenCalledWith()
    expect(unlock).not.toHaveBeenCalled()
    setFullscreen(false)
    expect(unlock).toHaveBeenCalledTimes(1)
    expect(lock).toHaveBeenCalledTimes(1)
  })

  it('locks again, and stays full screen, after a reconnect', async () => {
    Object.defineProperty(navigator, 'keyboard', { value: { lock, unlock }, configurable: true })
    const { session, f } = await activePage()
    setFullscreen(true)
    act(() => {
      session.end({ by: 'link', reason: 'relay connection closed (code 1006)' })
    })
    await waitFor(
      () => {
        expect(f.sessions).toHaveLength(2)
      },
      { timeout: 3000 },
    )
    act(() => {
      f.sessions[1]?.activate()
    })
    expect(lock).toHaveBeenCalledTimes(2)
    expect(screen.getByRole('button', { name: '退出全屏' })).toBeInTheDocument()
  })

  it('unlocks when the page goes away while full screen', async () => {
    Object.defineProperty(navigator, 'keyboard', { value: { lock, unlock }, configurable: true })
    const { unmount } = await activePage()
    setFullscreen(true)
    unmount()
    expect(unlock).toHaveBeenCalledTimes(1)
  })

  it('goes on when the browser refuses the lock', async () => {
    const refused = vi.fn(() => Promise.reject(new Error('not focused')))
    Object.defineProperty(navigator, 'keyboard', {
      value: { lock: refused, unlock },
      configurable: true,
    })
    await activePage()
    setFullscreen(true)
    await Promise.resolve()
    expect(refused).toHaveBeenCalledTimes(1)
    expect(screen.getByRole('button', { name: '退出全屏' })).toBeInTheDocument()
  })
})
