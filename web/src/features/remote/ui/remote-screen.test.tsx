import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { FakeSession } from '../connect/fake-session'
import type { CursorShape } from '../cursor/cursor'
import { RemoteScreen } from './remote-screen'
import { useClipboardSync } from './use-clipboard-sync'

const BOX = { width: 800, height: 600 }
const SHAPE: CursorShape = {
  id: '1',
  width: 12,
  height: 18,
  hotX: 2,
  hotY: 3,
  rgba: new Uint8ClampedArray(12 * 18 * 4),
}
const CSS_CURSOR = 'url("blob:cursor") 1 2, auto'

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

let overlayCtx = fakeContext()

beforeEach(() => {
  vi.stubGlobal('ResizeObserver', FixedResizeObserver)
  vi.stubGlobal('ImageData', FakeImageData)
  vi.stubGlobal('OffscreenCanvas', FakeOffscreenCanvas)
  vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:cursor')
  vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => undefined)
  overlayCtx = fakeContext()
  const getContext = function (this: HTMLCanvasElement) {
    return this.dataset.testid === 'remote-screen' ? overlayCtx : fakeContext()
  }
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(
    getContext as unknown as HTMLCanvasElement['getContext'],
  )
  HTMLElement.prototype.setPointerCapture = vi.fn()
})

function frame(): VideoFrame {
  return { displayWidth: 1280, displayHeight: 800, close: () => undefined } as VideoFrame
}

function shown(options: { showRemoteCursor?: boolean; viewOnly?: boolean } = {}) {
  const session = new FakeSession()
  session.activate()
  render(
    <RemoteScreen
      session={session}
      scale="fit"
      showRemoteCursor={options.showRemoteCursor ?? true}
      viewOnly={options.viewOnly ?? false}
      onLeave={() => undefined}
    />,
  )
  act(() => {
    session.queue.push(frame())
    session.cursor.set({ shape: SHAPE, position: null })
  })
  const overlay = screen.getByTestId('remote-screen')
  vi.spyOn(overlay, 'getBoundingClientRect').mockReturnValue(
    DOMRect.fromRect({ x: 0, y: 0, width: 800, height: 500 }),
  )
  return { session, overlay }
}

const localCursor = (overlay: HTMLElement) => overlay.style.getPropertyValue('--remote-cursor')

function peerMovesTo(session: FakeSession, x: number, y: number) {
  act(() => {
    session.cursor.set({ ...session.cursor.get(), position: { x, y } })
  })
}

function drawnAt(): [number, number] | null {
  const draws = overlayCtx.putImageData.mock.calls
  const last = draws.at(-1)
  const cleared = overlayCtx.clearRect.mock.calls.length
  return last && cleared < draws.length ? [last[1] as number, last[2] as number] : null
}

describe('remote screen cursor', () => {
  it('shows the shape as the local pointer, scaled to the screen', async () => {
    const { overlay } = shown()
    await waitFor(() => {
      expect(localCursor(overlay)).toBe(CSS_CURSOR)
    })
    expect(overlay).not.toHaveClass('cursor-none')
    fireEvent.pointerMove(overlay, { clientX: 100, clientY: 50 })
    expect(drawnAt()).toBeNull()
  })

  it('draws the peer cursor where the peer moved it, until the local pointer moves', async () => {
    const { session, overlay } = shown()
    await waitFor(() => {
      expect(localCursor(overlay)).toBe(CSS_CURSOR)
    })
    peerMovesTo(session, 640, 400)
    expect(drawnAt()).toEqual([638, 397])
    fireEvent.pointerMove(overlay, { clientX: 100, clientY: 50 })
    expect(drawnAt()).toBeNull()
  })

  it('does not draw the peer cursor when the setting is off', async () => {
    const { session, overlay } = shown({ showRemoteCursor: false })
    await waitFor(() => {
      expect(localCursor(overlay)).toBe(CSS_CURSOR)
    })
    peerMovesTo(session, 640, 400)
    expect(drawnAt()).toBeNull()
  })

  it.each([
    ['without input rights', { viewOnly: false, keyboard: false }],
    ['view only', { viewOnly: true, keyboard: true }],
  ])('%s: a plain local pointer, the peer cursor always drawn', (_, { viewOnly, keyboard }) => {
    const { session, overlay } = shown({ viewOnly, showRemoteCursor: false })
    act(() => {
      session.permissions.set({ keyboard, clipboard: true, restart: true })
    })
    expect(localCursor(overlay)).toBe('default')
    peerMovesTo(session, 640, 400)
    fireEvent.pointerMove(overlay, { clientX: 100, clientY: 50 })
    expect(drawnAt()).toEqual([638, 397])
  })

  it('shows nothing of its own when the peer embeds the cursor in the video', () => {
    const { session, overlay } = shown()
    act(() => {
      const peer = session.peer.get()
      if (peer) session.peer.set({ ...peer, cursorEmbedded: true })
    })
    expect(localCursor(overlay)).toBe('default')
    peerMovesTo(session, 640, 400)
    fireEvent.pointerMove(overlay, { clientX: 100, clientY: 50 })
    expect(drawnAt()).toBeNull()
  })

  it('draws the shape at the local pointer when it is too large for a CSS cursor', () => {
    const { session, overlay } = shown()
    const side = 220
    const big = {
      ...SHAPE,
      id: '2',
      width: side,
      height: side,
      rgba: new Uint8ClampedArray(side * side * 4),
    }
    act(() => {
      session.cursor.set({ shape: big, position: null })
    })
    expect(localCursor(overlay)).toBe('none')
    fireEvent.pointerMove(overlay, { clientX: 100, clientY: 50 })
    expect(drawnAt()).toEqual([158, 77])
  })
})

function Synced({ session }: { session: FakeSession }) {
  const sync = useClipboardSync({
    session,
    enabled: true,
    onEnabled: () => undefined,
    onSent: () => undefined,
    onCopied: () => undefined,
  })
  return (
    <RemoteScreen
      session={session}
      scale="fit"
      showRemoteCursor
      onLeave={() => undefined}
      onGesture={sync.onGesture}
      onPaste={sync.onPaste}
    />
  )
}

describe('remote screen paste with a device text waiting', () => {
  it('pastes the local text, and leaves the waiting text for another key', async () => {
    const writeText = vi
      .fn<(text: string) => Promise<void>>()
      .mockRejectedValueOnce(new DOMException('no gesture', 'NotAllowedError'))
      .mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', {
      value: { readText: vi.fn(), writeText },
      configurable: true,
    })
    vi.spyOn(document, 'hasFocus').mockReturnValue(true)
    const session = new FakeSession()
    session.activate()
    render(<Synced session={session} />)
    await act(async () => {
      session.remoteClipboard.set('from the device')
      await Promise.resolve()
    })
    expect(writeText).toHaveBeenCalledTimes(1)

    const keys = screen.getByRole('textbox')
    keys.focus()
    act(() => {
      keys.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'v', code: 'KeyV', ctrlKey: true, bubbles: true }),
      )
    })
    const paste = new Event('paste', { bubbles: true, cancelable: true })
    Object.defineProperty(paste, 'clipboardData', { value: { getData: () => 'typed here' } })
    act(() => {
      keys.dispatchEvent(paste)
    })
    expect(writeText).toHaveBeenCalledTimes(1)
    expect(session.pasted).toEqual(['typed here'])

    act(() => {
      keys.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', code: 'KeyA', bubbles: true }))
    })
    expect(writeText).toHaveBeenLastCalledWith('from the device')
    Reflect.deleteProperty(navigator, 'clipboard')
  })
})
