import { act, fireEvent, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { renderWithProviders } from '@/test/render-with-providers'

import { RemoteController } from '../../connect/controller'
import { fakeDeps } from '../../connect/fake-deps'
import { SETTINGS_KEY, type RemoteSettings } from '../../settings/settings'
import { RemoteDepsContext } from '../deps-context'
import { RemotePage } from '../remote-page'
import { openMenu } from './testing'

const SIZE_1920 = { width: 1920, height: 1080 }
const SIZE_1280 = { width: 1280, height: 800 }
const SIZE_1024 = { width: 1024, height: 768 }
const BOX = { width: 800, height: 600 }

class FixedResizeObserver {
  readonly #callback: ResizeObserverCallback
  constructor(callback: ResizeObserverCallback) {
    this.#callback = callback
  }
  observe(target: Element) {
    const entry = { target, contentRect: DOMRect.fromRect(BOX) } as ResizeObserverEntry
    this.#callback([entry], this)
  }
  readonly unobserve = vi.fn()
  readonly disconnect = vi.fn()
}

beforeEach(() => {
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null)
  HTMLElement.prototype.setPointerCapture = vi.fn()
})

async function activePage(stored?: Partial<RemoteSettings>, displays = 1) {
  const f = fakeDeps()
  if (stored) f.storage.setItem(SETTINGS_KEY, JSON.stringify({ v: 1, settings: stored }))
  const updates = vi.spyOn(RemoteController.prototype, 'updateSettings')
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
    session.activate(displays)
  })
  session.patches.length = 0
  return { ...view, f, session, updates }
}

function frame(): VideoFrame {
  return { displayWidth: 1280, displayHeight: 800, close: () => undefined } as VideoFrame
}

function screenArea() {
  const overlay = screen.getByTestId('remote-screen')
  const picture = overlay.parentElement
  const box = picture?.parentElement
  if (!picture || !box) throw new Error('no screen area')
  return { overlay, picture, box }
}

describe('display menu', () => {
  it('hands a picked quality to the controller, and the session gets it as a cap', async () => {
    const { user, f, session, updates } = await activePage()
    await openMenu(user, '显示')
    expect(screen.getByRole('menuitemradio', { name: /^均衡/ })).toHaveAttribute(
      'aria-checked',
      'true',
    )
    await user.click(screen.getByRole('menuitemradio', { name: /^最佳/ }))
    expect(updates).toHaveBeenCalledTimes(1)
    expect(updates).toHaveBeenCalledWith({ quality: 'best' })
    expect(session.patches).toEqual([{ quality: 'best' }])
    expect(f.sessions).toHaveLength(1)
    expect(screen.getByRole('menuitemradio', { name: /^最佳/ })).toHaveAttribute(
      'aria-checked',
      'true',
    )
  })

  it('starts a custom quality at the preset level and sets it when the slider is let go', async () => {
    const { user, updates } = await activePage({ quality: 'best' })
    await openMenu(user, '显示')
    expect(screen.queryByRole('slider', { name: '自定义画质' })).toBeNull()
    await user.click(screen.getByRole('menuitemradio', { name: '自定义' }))
    expect(updates).toHaveBeenLastCalledWith({ quality: { custom: 75 } })

    const slider = screen.getByRole('slider', { name: '自定义画质' })
    expect(slider).toHaveAttribute('aria-valuetext', '75（~1.6 Mbps）')
    fireEvent.change(slider, { target: { value: '6' } })
    fireEvent.change(slider, { target: { value: '10' } })
    expect(updates).toHaveBeenCalledTimes(1)
    fireEvent.pointerUp(slider)
    expect(updates).toHaveBeenCalledTimes(2)
    expect(updates).toHaveBeenLastCalledWith({ quality: { custom: 60 } })
    fireEvent.keyUp(slider, { key: 'ArrowRight' })
    expect(updates).toHaveBeenCalledTimes(2)
  })

  it('steps the slider by 5 up to 100, then by 25 up to 500', async () => {
    const { user, updates } = await activePage({ quality: { custom: 10 } })
    await openMenu(user, '显示')
    const slider = screen.getByRole('slider', { name: '自定义画质' })
    expect(slider).toHaveAttribute('min', '0')
    expect(slider).toHaveAttribute('max', '34')
    expect(slider).toHaveAttribute('step', '1')
    expect(slider).toHaveValue('0')
    for (const [position, level, rate] of [
      ['18', 100, '~2.1 Mbps'],
      ['19', 125, '~2.7 Mbps'],
      ['34', 500, '~10.7 Mbps'],
    ] as const) {
      fireEvent.change(slider, { target: { value: position } })
      expect(slider).toHaveAttribute('aria-valuetext', `${String(level)}（${rate}）`)
      expect(screen.getByText(rate)).toBeInTheDocument()
      fireEvent.keyUp(slider, { key: 'ArrowRight' })
      expect(updates).toHaveBeenLastCalledWith({ quality: { custom: level } })
    }
  })

  it('puts an off-grid stored level on the nearest step, and keeps it until moved', async () => {
    const { user, updates } = await activePage({ quality: { custom: 33 } })
    await openMenu(user, '显示')
    const slider = screen.getByRole('slider', { name: '自定义画质' })
    expect(slider).toHaveValue('5')
    expect(slider).toHaveAttribute('aria-valuetext', '33（~710 kbps）')
    fireEvent.blur(slider)
    expect(updates).not.toHaveBeenCalled()
  })

  it('marks each preset with its bitrate at the peer resolution, and calls it a ceiling', async () => {
    const { user } = await activePage()
    await openMenu(user, '显示')
    for (const name of ['低（~540 kbps）', '均衡（~720 kbps）', '最佳（~1.6 Mbps）', '自定义']) {
      expect(screen.getByRole('menuitemradio', { name })).toBeInTheDocument()
    }
    expect(screen.getByText('码率为上限，画面静止或简单时用不满。')).toBeInTheDocument()
  })

  it('follows a resolution change', async () => {
    const { user, session } = await activePage()
    act(() => {
      session.resize(1920, 1080)
    })
    await openMenu(user, '显示')
    expect(screen.getByRole('menuitemradio', { name: '最佳（~3.1 Mbps）' })).toBeInTheDocument()
  })

  it('shows the bitrate ratio while the peer resolution is unknown', async () => {
    const { user, session } = await activePage()
    act(() => {
      const peer = session.peer.get()
      if (peer) session.peer.set({ ...peer, display: null })
    })
    await openMenu(user, '显示')
    for (const name of ['低（0.5×）', '均衡（0.67×）', '最佳（1.5×）']) {
      expect(screen.getByRole('menuitemradio', { name })).toBeInTheDocument()
    }
  })

  it('leaves the numbers out for a codec whose bitrate we do not model', async () => {
    const { user, session } = await activePage({ quality: { custom: 200 } })
    act(() => {
      session.stats.set({ ...session.stats.get(), codec: 'avc1.64002A' })
    })
    await openMenu(user, '显示')
    expect(screen.getByRole('menuitemradio', { name: '最佳' })).toBeInTheDocument()
    expect(screen.getByRole('slider', { name: '自定义画质' })).toHaveAttribute(
      'aria-valuetext',
      '200',
    )
    expect(screen.queryByText(/Mbps|kbps/)).toBeNull()
    expect(screen.queryByText('码率为上限，画面静止或简单时用不满。')).toBeNull()
  })

  it('caps the frame rate at a picked step', async () => {
    const { user, session, updates } = await activePage()
    await openMenu(user, '显示')
    screen.getByRole('menuitem', { name: /帧率/ }).focus()
    await user.keyboard('{ArrowRight}')
    expect(await screen.findByRole('menuitemradio', { name: '30 fps' })).toHaveAttribute(
      'aria-checked',
      'true',
    )
    await user.keyboard('{ArrowDown}{ArrowDown}{Enter}')
    expect(updates).toHaveBeenCalledWith({ fps: 15 })
    expect(session.patches).toEqual([{ maxFps: 15 }])
  })

  it('switches between fit and original size, scrolls only at original size', async () => {
    vi.stubGlobal('ResizeObserver', FixedResizeObserver)
    const { user, session, updates } = await activePage()
    act(() => {
      session.queue.push(frame())
    })
    const { picture, box } = screenArea()
    await waitFor(() => {
      expect(picture.style.getPropertyValue('--screen-w')).toBe('800px')
    })
    expect(picture.style.getPropertyValue('--screen-h')).toBe('500px')
    expect(box).toHaveClass('overflow-hidden')

    await openMenu(user, '显示')
    await user.click(screen.getByRole('menuitemradio', { name: '原始大小' }))
    expect(updates).toHaveBeenCalledWith({ scale: 'original' })
    expect(picture.style.getPropertyValue('--screen-w')).toBe('1280px')
    expect(picture.style.getPropertyValue('--screen-h')).toBe('800px')
    expect(box).toHaveClass('overflow-auto')

    box.scrollLeft = 480
    box.scrollTop = 200
    await openMenu(user, '显示')
    await user.click(screen.getByRole('menuitemradio', { name: '适应窗口' }))
    expect(picture.style.getPropertyValue('--screen-w')).toBe('800px')
    expect(box.scrollLeft).toBe(0)
    expect(box.scrollTop).toBe(0)
  })

  it('maps the pointer onto the scrolled screen at original size', async () => {
    const { session } = await activePage({ scale: 'original' })
    const { overlay } = screenArea()
    vi.spyOn(overlay, 'getBoundingClientRect').mockReturnValue(
      DOMRect.fromRect({ x: -300, y: -200, width: 1280, height: 800 }),
    )
    fireEvent.pointerMove(overlay, { clientX: 100, clientY: 50 })
    expect(session.inputs).toEqual([
      expect.objectContaining({
        kind: 'pointer',
        action: 'move',
        point: { x: 400, y: 250, width: 1280, height: 800 },
      }),
    ])
  })

  it('offers a monitor switch only when the device has more than one', async () => {
    const { user } = await activePage()
    await openMenu(user, '显示')
    expect(screen.getByRole('menuitemradio', { name: '原始大小' })).toBeInTheDocument()
    expect(screen.queryByRole('menuitemradio', { name: /显示器/ })).toBeNull()
  })

  it('switches monitors', async () => {
    const { user, session } = await activePage(undefined, 2)
    await openMenu(user, '显示')
    expect(screen.getByRole('menuitemradio', { name: '显示器 1' })).toHaveAttribute(
      'aria-checked',
      'true',
    )
    await user.click(screen.getByRole('menuitemradio', { name: '显示器 2' }))
    expect(session.displays).toEqual([1])
  })

  it('offers the peer resolutions only with input rights and a choice', async () => {
    const { user, session } = await activePage()
    await openMenu(user, '显示')
    expect(screen.queryByRole('menuitem', { name: /分辨率/ })).toBeNull()
    await user.keyboard('{Escape}')

    act(() => {
      session.offerResolutions([SIZE_1280], SIZE_1920)
    })
    await openMenu(user, '显示')
    expect(screen.queryByRole('menuitem', { name: /分辨率/ })).toBeNull()
    await user.keyboard('{Escape}')

    act(() => {
      session.offerResolutions([SIZE_1920, SIZE_1280, SIZE_1024], SIZE_1920)
      session.permissions.set({ keyboard: false, clipboard: true, restart: true, audio: true })
    })
    await openMenu(user, '显示')
    expect(screen.queryByRole('menuitem', { name: /分辨率/ })).toBeNull()
    await user.keyboard('{Escape}')

    act(() => {
      session.permissions.set({ keyboard: true, clipboard: true, restart: true, audio: true })
    })
    await openMenu(user, '显示')
    expect(screen.getByRole('menuitem', { name: /分辨率/ })).toBeInTheDocument()
  })

  it('lists original first, marks the current size, and waits for the peer to switch', async () => {
    const { user, session } = await activePage()
    act(() => {
      session.offerResolutions([SIZE_1920, SIZE_1280, SIZE_1024], SIZE_1920)
    })
    await openMenu(user, '显示')
    screen.getByRole('menuitem', { name: /分辨率/ }).focus()
    await user.keyboard('{ArrowRight}')
    const items = await screen.findAllByRole('menuitemradio', { name: /×/ })
    expect(items.map((i) => i.textContent)).toEqual(['原始（1920×1080）', '1280×800', '1024×768'])
    expect(items.map((i) => i.getAttribute('aria-checked'))).toEqual(['false', 'true', 'false'])

    await user.click(screen.getByRole('menuitemradio', { name: '1024×768' }))
    expect(session.resolutions).toEqual([{ width: 1024, height: 768 }])
    expect(screen.queryByRole('menuitemradio', { name: /×/ })).toBeNull()

    await openMenu(user, '显示')
    screen.getByRole('menuitem', { name: /分辨率/ }).focus()
    await user.keyboard('{ArrowRight}')
    const unchanged = await screen.findAllByRole('menuitemradio', { name: /×/ })
    expect(unchanged.map((i) => i.getAttribute('aria-checked'))).toEqual(['false', 'true', 'false'])
    await user.keyboard('{Escape}{Escape}')

    act(() => {
      session.resize(1024, 768)
    })
    await openMenu(user, '显示')
    screen.getByRole('menuitem', { name: /分辨率/ }).focus()
    await user.keyboard('{ArrowRight}')
    const after = await screen.findAllByRole('menuitemradio', { name: /×/ })
    expect(after.map((i) => i.getAttribute('aria-checked'))).toEqual(['false', 'false', 'true'])
  })

  it('asks for the original resolution', async () => {
    const { user, session } = await activePage()
    act(() => {
      session.offerResolutions([SIZE_1920, SIZE_1280], SIZE_1920)
    })
    await openMenu(user, '显示')
    screen.getByRole('menuitem', { name: /分辨率/ }).focus()
    await user.keyboard('{ArrowRight}')
    await user.click(await screen.findByRole('menuitemradio', { name: '原始（1920×1080）' }))
    expect(session.resolutions).toEqual([{ width: 1920, height: 1080 }])
  })

  it('picks the connection, auto by default, and a change reconnects', async () => {
    const { user, f, updates } = await activePage()
    await openMenu(user, '显示')
    expect(screen.getByRole('group', { name: '连接方式' })).toBeInTheDocument()
    expect(screen.getByRole('menuitemradio', { name: '自动（WebRTC 优先）' })).toHaveAttribute(
      'aria-checked',
      'true',
    )
    await user.click(screen.getByRole('menuitemradio', { name: '仅中继' }))
    expect(updates).toHaveBeenCalledTimes(1)
    expect(updates).toHaveBeenCalledWith({ transport: 'relay' })
    await waitFor(() => {
      expect(f.sessions).toHaveLength(2)
    })
    act(() => {
      f.sessions[1]?.activate()
    })
    await openMenu(user, '显示')
    expect(screen.getByRole('menuitemradio', { name: '仅中继' })).toHaveAttribute(
      'aria-checked',
      'true',
    )
  })

  it('turns drawing the remote cursor off and on, without telling the peer', async () => {
    const { user, session, updates } = await activePage()
    await openMenu(user, '显示')
    const item = screen.getByRole('menuitemcheckbox', { name: '显示远端光标' })
    expect(item).toHaveAttribute('aria-checked', 'true')
    await user.click(item)
    expect(updates).toHaveBeenLastCalledWith({ showRemoteCursor: false })
    await openMenu(user, '显示')
    const off = screen.getByRole('menuitemcheckbox', { name: '显示远端光标' })
    expect(off).toHaveAttribute('aria-checked', 'false')
    await user.click(off)
    expect(updates).toHaveBeenLastCalledWith({ showRemoteCursor: true })
    expect(session.patches).toEqual([])
  })

  it('shows the saved settings again after a reconnect', async () => {
    const { user, f, session } = await activePage()
    await openMenu(user, '显示')
    await user.click(screen.getByRole('menuitemradio', { name: /^低/ }))
    await user.keyboard('{Escape}')
    act(() => {
      session.end({ by: 'peer', reason: 'Offline' })
    })
    await user.click(screen.getByRole('button', { name: '重新连接' }))
    await waitFor(() => {
      expect(f.sessions).toHaveLength(2)
    })
    act(() => {
      f.sessions[1]?.activate()
    })
    await openMenu(user, '显示')
    expect(screen.getByRole('menuitemradio', { name: /^低/ })).toHaveAttribute(
      'aria-checked',
      'true',
    )
  })
})
