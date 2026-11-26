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
      <RemotePage deviceId="123456789" autoConnect />
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

function screenArea() {
  const overlay = screen.getByTestId('remote-screen')
  const picture = overlay.parentElement
  const box = picture?.parentElement
  if (!picture || !box) throw new Error('no screen area')
  return { overlay, picture, box }
}

describe('display menu', () => {
  it('starts a custom quality at the preset level and sets it when the slider is let go', async () => {
    const { user, updates } = await activePage({ quality: 'best' })
    await openMenu(user, '显示')
    expect(screen.queryByRole('slider', { name: '自定义画质' })).toBeNull()
    await user.click(screen.getByRole('menuitemradio', { name: '自定义' }))
    expect(updates).toHaveBeenLastCalledWith({ quality: { custom: 75 } })

    const slider = screen.getByRole('slider', { name: '自定义画质' })
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
    for (const [position, level] of [
      ['18', 100],
      ['19', 125],
      ['34', 500],
    ] as const) {
      fireEvent.change(slider, { target: { value: position } })
      fireEvent.keyUp(slider, { key: 'ArrowRight' })
      expect(updates).toHaveBeenLastCalledWith({ quality: { custom: level } })
    }
  })

  it('puts an off-grid stored level on the nearest step, and keeps it until moved', async () => {
    const { user, updates } = await activePage({ quality: { custom: 33 } })
    await openMenu(user, '显示')
    const slider = screen.getByRole('slider', { name: '自定义画质' })
    expect(slider).toHaveValue('5')
    fireEvent.blur(slider)
    expect(updates).not.toHaveBeenCalled()
  })

  async function openCodecs(user: Awaited<ReturnType<typeof activePage>>['user']) {
    await openMenu(user, '显示')
    screen.getByRole('menuitem', { name: /^编码/ }).focus()
    await user.keyboard('{ArrowRight}')
    return screen.findByRole('menuitemradio', { name: /^VP9/ })
  }

  it('greys out a codec either side cannot handle, and it cannot be picked', async () => {
    const { user, session, updates } = await activePage()
    act(() => {
      session.codecs.set({
        ...session.codecs.get(),
        support: { vp9: 'ok', av1: 'browser', vp8: 'peer', h264: 'peer', h265: 'peer' },
      })
    })
    await openCodecs(user)
    const av1 = screen.getByRole('menuitemradio', { name: /^AV1/ })
    const vp8 = screen.getByRole('menuitemradio', { name: /^VP8/ })
    expect(av1).toHaveAttribute('aria-disabled', 'true')
    expect(vp8).toHaveAttribute('aria-disabled', 'true')
    await user.click(vp8)
    expect(updates).not.toHaveBeenCalled()
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

  it('marks the current size and waits for the peer to switch', async () => {
    const { user, session } = await activePage()
    act(() => {
      session.offerResolutions([SIZE_1920, SIZE_1280, SIZE_1024], SIZE_1920)
    })
    await openMenu(user, '显示')
    screen.getByRole('menuitem', { name: /分辨率/ }).focus()
    await user.keyboard('{ArrowRight}')
    const items = await screen.findAllByRole('menuitemradio', { name: /×/ })
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
})
