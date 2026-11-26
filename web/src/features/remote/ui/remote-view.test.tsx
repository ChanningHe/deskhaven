import { act, fireEvent, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { renderWithProviders } from '@/test/render-with-providers'

import { fakeDeps } from '../connect/fake-deps'
import type { FakeSession } from '../connect/fake-session'
import type { CursorShape } from '../cursor/cursor'
import { RemoteDepsContext } from './deps-context'
import { openMenu } from './menus/testing'
import { RemotePage } from './remote-page'

const SHAPE: CursorShape = {
  id: '1',
  width: 12,
  height: 18,
  hotX: 2,
  hotY: 3,
  rgba: new Uint8ClampedArray(12 * 18 * 4),
}

class FixedResizeObserver {
  readonly #callback: ResizeObserverCallback
  constructor(callback: ResizeObserverCallback) {
    this.#callback = callback
  }
  observe(target: Element) {
    const contentRect = DOMRect.fromRect({ width: 800, height: 600 })
    this.#callback([{ target, contentRect } as ResizeObserverEntry], this)
  }
  readonly unobserve = vi.fn()
  readonly disconnect = vi.fn()
}

class FakeImageData {
  readonly data: Uint8ClampedArray
  readonly width: number
  readonly height: number
  constructor(data: Uint8ClampedArray, width: number, height: number) {
    this.data = data
    this.width = width
    this.height = height
  }
}

const fakeContext = () => ({
  drawImage: vi.fn(),
  putImageData: vi.fn(),
  clearRect: vi.fn(),
  imageSmoothingQuality: 'low',
})

class FakeOffscreenCanvas {
  getContext() {
    return fakeContext()
  }
  convertToBlob() {
    return Promise.resolve(new Blob())
  }
}

let local = ''
const readText = vi.fn(() => Promise.resolve(local))

beforeEach(() => {
  vi.stubGlobal('ResizeObserver', FixedResizeObserver)
  vi.stubGlobal('ImageData', FakeImageData)
  vi.stubGlobal('OffscreenCanvas', FakeOffscreenCanvas)
  vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:cursor')
  vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => undefined)
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation((() =>
    fakeContext()) as unknown as HTMLCanvasElement['getContext'])
  HTMLElement.prototype.setPointerCapture = vi.fn()
  vi.spyOn(document, 'hasFocus').mockReturnValue(true)
  local = 'before the session'
  readText.mockClear()
})

afterEach(() => {
  Reflect.deleteProperty(navigator, 'permissions')
  Reflect.deleteProperty(navigator, 'clipboard')
})

function frame(): VideoFrame {
  return { displayWidth: 1280, displayHeight: 800, close: () => undefined } as VideoFrame
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
  Object.defineProperty(navigator, 'clipboard', {
    value: { readText, writeText: () => Promise.resolve() },
    configurable: true,
  })
  Object.defineProperty(navigator, 'permissions', {
    value: { query: () => Promise.resolve(Object.assign(new EventTarget(), { state: 'granted' })) },
    configurable: true,
  })
  act(() => {
    session.activate()
    session.queue.push(frame())
    session.cursor.set({ shape: SHAPE, position: null })
  })
  return { ...view, session }
}

const overlay = () => screen.getByTestId('remote-screen')

async function viewOnly(user: Awaited<ReturnType<typeof activePage>>['user']) {
  await openMenu(user, '操作')
  await user.click(screen.getByRole('menuitemcheckbox', { name: '仅查看' }))
}

async function settle() {
  await act(async () => {
    for (let i = 0; i < 5; i++) await Promise.resolve()
  })
}

async function pointerEnters(session: FakeSession, text: string) {
  local = text
  const screenBox = overlay().parentElement
  if (!screenBox) throw new Error('the screen has no box')
  fireEvent.pointerEnter(screenBox)
  await settle()
  return session.pasted
}

describe('remote view: sending the clipboard by hand', () => {
  it('does not send a text pasted into the box again either', async () => {
    const { user, session } = await activePage()
    await settle()
    readText.mockRejectedValueOnce(new DOMException('denied', 'NotAllowedError'))
    await openMenu(user, '剪贴板')
    await user.click(screen.getByRole('menuitem', { name: '发送剪贴板' }))
    await user.type(await screen.findByLabelText('要发送的文本'), 'typed')
    await user.click(screen.getByRole('button', { name: '发送' }))
    expect(session.pasted).toEqual(['typed'])
    expect(await pointerEnters(session, 'typed')).toEqual(['typed'])
  })
})

describe('remote view: view only', () => {
  it('stops the clipboard sync both ways, and starts it again', async () => {
    const { user, session } = await activePage()
    await settle()
    expect(await pointerEnters(session, 'copied before')).toEqual(['copied before'])

    await viewOnly(user)
    expect(await pointerEnters(session, 'copied while viewing')).toEqual(['copied before'])

    await openMenu(user, '操作')
    await user.click(screen.getByRole('menuitemcheckbox', { name: '仅查看' }))
    await settle()
    expect(await pointerEnters(session, 'copied after')).toContain('copied after')
  })

  it('greys out the keyboard items of the Input menu', async () => {
    const { user } = await activePage()
    await viewOnly(user)
    await openMenu(user, '输入')
    expect(screen.getByRole('menuitem', { name: '发送按键' })).toHaveAttribute(
      'aria-disabled',
      'true',
    )
    for (const mode of screen.getAllByRole('menuitemradio')) {
      expect(mode).toHaveAttribute('aria-disabled', 'true')
    }
  })

  it('offers no resolution to change while viewing only', async () => {
    const { user, session } = await activePage()
    act(() => {
      session.offerResolutions(
        [
          { width: 1920, height: 1080 },
          { width: 1280, height: 800 },
        ],
        { width: 1920, height: 1080 },
      )
    })
    await openMenu(user, '显示')
    expect(screen.getByRole('menuitem', { name: /分辨率/ })).toBeInTheDocument()
    await user.keyboard('{Escape}')
    await viewOnly(user)
    await openMenu(user, '显示')
    expect(screen.queryByRole('menuitem', { name: /分辨率/ })).toBeNull()
  })

  it('greys out sending the clipboard', async () => {
    const { user } = await activePage()
    await viewOnly(user)
    await openMenu(user, '剪贴板')
    expect(screen.getByRole('menuitem', { name: '发送剪贴板' })).toHaveAttribute(
      'aria-disabled',
      'true',
    )
  })
})
