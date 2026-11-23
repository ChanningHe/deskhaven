import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { FakeSession } from '../connect/fake-session'
import { useClipboardSync } from './use-clipboard-sync'

class FakeStatus extends EventTarget {
  state: PermissionState
  constructor(state: PermissionState) {
    super()
    this.state = state
  }
  change(state: PermissionState) {
    this.state = state
    this.dispatchEvent(new Event('change'))
  }
}

let local = ''
let focused = true
const readText = vi.fn(() => Promise.resolve(local))
const writeText = vi.fn((text: string) => {
  local = text
  return Promise.resolve()
})
const onSent = vi.fn()
const onCopied = vi.fn()
const onEnabled = vi.fn()
let pasting = 'pasted here'

function stubPermissions(query: (descriptor: { name: string }) => Promise<FakeStatus>) {
  Object.defineProperty(navigator, 'permissions', { value: { query }, configurable: true })
}

beforeEach(() => {
  local = 'before the session'
  focused = true
  pasting = 'pasted here'
  readText.mockClear()
  writeText.mockClear()
  onSent.mockClear()
  onCopied.mockClear()
  onEnabled.mockClear()
  Object.defineProperty(navigator, 'clipboard', {
    value: { readText, writeText },
    configurable: true,
  })
  vi.spyOn(document, 'hasFocus').mockImplementation(() => focused)
  setVisibility('visible')
})

afterEach(() => {
  Reflect.deleteProperty(navigator, 'permissions')
  Reflect.deleteProperty(navigator, 'clipboard')
  vi.useRealTimers()
})

interface Props {
  session: FakeSession
  enabled?: boolean
  viewOnly?: boolean
}

function Harness({ session, enabled = true, viewOnly = false }: Props) {
  const sync = useClipboardSync({ session, enabled, viewOnly, onEnabled, onSent, onCopied })
  return (
    <div
      data-testid="screen"
      onPointerEnter={sync.onPointerEnter}
      onPointerDownCapture={sync.onGesture}
      onKeyDownCapture={sync.onGesture}
    >
      <output>{sync.access ?? 'unknown'}</output>
      <button
        type="button"
        onClick={() => {
          sync.setEnabled(!enabled)
        }}
      >
        switch
      </button>
      <button
        type="button"
        onClick={() => {
          sync.onPaste(pasting)
        }}
      >
        paste
      </button>
    </div>
  )
}

/** Lets the clipboard and permission promises settle. */
async function settle() {
  await act(async () => {
    for (let i = 0; i < 5; i++) await Promise.resolve()
  })
}

async function mount(state: PermissionState | 'throws', props: Partial<Props> = {}) {
  const status = new FakeStatus(state === 'throws' ? 'prompt' : state)
  stubPermissions(({ name }) => {
    if (state === 'throws' || name !== 'clipboard-read') throw new TypeError('bad name')
    return Promise.resolve(status)
  })
  const session = props.session ?? new FakeSession()
  session.activate()
  const view = render(<Harness {...props} session={session} />)
  await settle()
  return { ...view, session, status }
}

async function focusWindow() {
  focused = true
  fireEvent.focus(window)
  await settle()
}

function blurWindow() {
  focused = false
  fireEvent.blur(window)
}

function setVisibility(state: DocumentVisibilityState) {
  Object.defineProperty(document, 'visibilityState', { value: state, configurable: true })
  fireEvent(document, new Event('visibilitychange'))
}

function peerCopies(session: FakeSession, text: string) {
  act(() => {
    session.remoteClipboard.set(text)
  })
}

describe('useClipboardSync: local to remote', () => {
  it('when granted, sends a new local text once on focus, never the baseline', async () => {
    const { session } = await mount('granted')
    expect(screen.getByRole('status')).toHaveTextContent('granted')
    expect(readText).toHaveBeenCalledTimes(1)
    await focusWindow()
    expect(session.pasted).toEqual([])

    local = 'copied here'
    await focusWindow()
    await focusWindow()
    expect(session.pasted).toEqual(['copied here'])
    expect(onSent).toHaveBeenCalledTimes(1)
  })

  it('reads when the page becomes visible and when the pointer enters the screen', async () => {
    const { session } = await mount('granted')
    local = 'one'
    setVisibility('visible')
    await settle()
    local = 'two'
    fireEvent.pointerEnter(screen.getByTestId('screen'))
    await settle()
    expect(session.pasted).toEqual(['one', 'two'])
  })

  it('does not read while access is only promptable, until it is granted', async () => {
    const { session, status } = await mount('prompt')
    await focusWindow()
    fireEvent.pointerEnter(screen.getByTestId('screen'))
    await settle()
    expect(readText).not.toHaveBeenCalled()

    act(() => {
      status.change('granted')
    })
    await settle()
    expect(readText).toHaveBeenCalledTimes(1)
    await focusWindow()
    expect(session.pasted).toEqual([])
  })

  it('takes a browser without the clipboard-read permission name as not reading itself', async () => {
    await mount('throws')
    expect(screen.getByRole('status')).toHaveTextContent('unsupported')
    await focusWindow()
    expect(readText).not.toHaveBeenCalled()
  })

  it('polls every second while focused and stops on blur', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] })
    await mount('granted')
    expect(readText).toHaveBeenCalledTimes(1)
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3000)
    })
    expect(readText).toHaveBeenCalledTimes(4)

    blurWindow()
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3000)
    })
    expect(readText).toHaveBeenCalledTimes(4)

    await focusWindow()
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000)
    })
    expect(readText).toHaveBeenCalledTimes(6)
  })

  it('takes a new baseline for a new session', async () => {
    const first = await mount('granted')
    local = 'copied between sessions'
    const next = new FakeSession()
    next.activate()
    first.rerender(<Harness session={next} />)
    await settle()
    await focusWindow()
    expect(first.session.pasted).toEqual([])
    expect(next.pasted).toEqual([])
  })
})

function press(name: string) {
  fireEvent.click(screen.getByRole('button', { name }))
}

describe('useClipboardSync: paste', () => {
  it.each<[string, PermissionState | 'throws']>([
    ['in a browser that cannot read the clipboard', 'throws'],
    ['before it may read the clipboard', 'prompt'],
    ['with the permission', 'granted'],
  ])('sends the pasted text, %s', async (_, access) => {
    const { session } = await mount(access)
    press('paste')
    expect(session.pasted).toEqual(['pasted here'])
    expect(onSent).toHaveBeenCalledTimes(1)
  })

  it('sends the pasted text once, whether the automatic read or the paste comes first', async () => {
    const { session } = await mount('granted')
    local = 'pasted here'
    await focusWindow()
    press('paste')
    await focusWindow()
    expect(session.pasted).toEqual(['pasted here'])

    pasting = 'next'
    press('paste')
    local = 'next'
    await focusWindow()
    expect(session.pasted).toEqual(['pasted here', 'next'])
  })

  it('sends a text copied before the session when it is pasted', async () => {
    const { session } = await mount('granted')
    pasting = 'before the session'
    press('paste')
    expect(session.pasted).toEqual(['before the session'])
  })

  it('sends nothing for an empty text', async () => {
    const { session } = await mount('granted')
    pasting = ''
    press('paste')
    expect(session.pasted).toEqual([])
  })

  it.each<[string, Partial<Props>]>([
    ['switched off', { enabled: false }],
    ['view only', { viewOnly: true }],
  ])('sends nothing when %s', async (_, props) => {
    const { session } = await mount('granted', props)
    press('paste')
    expect(session.pasted).toEqual([])
    expect(onSent).not.toHaveBeenCalled()
  })

  it('sends nothing when the device turned the clipboard off', async () => {
    const session = new FakeSession()
    session.permissions.set({ keyboard: true, clipboard: false, restart: true })
    await mount('granted', { session })
    press('paste')
    expect(session.pasted).toEqual([])
  })
})

describe('useClipboardSync: remote to local', () => {
  it('writes what the peer copied while the page has the focus', async () => {
    const { session } = await mount('granted')
    peerCopies(session, 'from the device')
    await settle()
    expect(writeText).toHaveBeenCalledWith('from the device')
    expect(onCopied).toHaveBeenCalledTimes(1)
    await focusWindow()
    expect(session.pasted).toEqual([])
  })

  it('does not send back a local read that started before the write', async () => {
    const { session } = await mount('granted')
    let finish: (text: string) => void = () => undefined
    readText.mockImplementationOnce(
      () =>
        new Promise<string>((resolve) => {
          finish = resolve
        }),
    )
    await focusWindow()
    peerCopies(session, 'from the device')
    await settle()
    expect(writeText).toHaveBeenCalledWith('from the device')
    finish('copied here before')
    await settle()
    await focusWindow()
    expect(session.pasted).toEqual([])
  })

  it('after a refused write, writes on the next pointer down inside that event', async () => {
    writeText.mockRejectedValueOnce(new DOMException('no gesture', 'NotAllowedError'))
    const { session } = await mount('throws')
    peerCopies(session, 'from the device')
    await settle()
    expect(writeText).toHaveBeenCalledTimes(1)
    expect(onCopied).not.toHaveBeenCalled()

    fireEvent.pointerDown(screen.getByTestId('screen'))
    expect(writeText).toHaveBeenCalledTimes(2)
    expect(writeText).toHaveBeenLastCalledWith('from the device')
    await settle()
    expect(onCopied).toHaveBeenCalledTimes(1)
    fireEvent.keyDown(screen.getByTestId('screen'), { key: 'a' })
    expect(writeText).toHaveBeenCalledTimes(2)
  })

  it.each([
    ['blur', blurWindow],
    [
      'the page going hidden',
      () => {
        setVisibility('hidden')
      },
    ],
  ])('drops a pending write on %s', async (_, leave) => {
    writeText.mockRejectedValueOnce(new DOMException('no gesture', 'NotAllowedError'))
    const { session } = await mount('granted')
    peerCopies(session, 'from the device')
    await settle()
    leave()
    focused = true
    fireEvent.pointerDown(screen.getByTestId('screen'))
    expect(writeText).toHaveBeenCalledTimes(1)
  })

  it('does not hold what the peer copied while the page is in the background', async () => {
    const { session } = await mount('granted')
    focused = false
    peerCopies(session, 'from the device')
    focused = true
    fireEvent.pointerDown(screen.getByTestId('screen'))
    await settle()
    expect(writeText).not.toHaveBeenCalled()
    expect(session.remoteClipboard.get()).toBe('from the device')
  })
})

describe('useClipboardSync: off', () => {
  it.each<[string, Partial<Props>]>([
    ['switched off', { enabled: false }],
    ['view only', { viewOnly: true }],
  ])('neither reads nor writes when %s', async (_, props) => {
    const { session } = await mount('granted', props)
    local = 'copied here'
    await focusWindow()
    peerCopies(session, 'from the device')
    fireEvent.pointerDown(screen.getByTestId('screen'))
    await settle()
    expect(readText).not.toHaveBeenCalled()
    expect(writeText).not.toHaveBeenCalled()
    expect(session.pasted).toEqual([])
  })

  it('neither reads nor writes when the device turned the clipboard off', async () => {
    const session = new FakeSession()
    session.permissions.set({ keyboard: true, clipboard: false, restart: true })
    await mount('granted', { session })
    peerCopies(session, 'from the device')
    await focusWindow()
    expect(readText).not.toHaveBeenCalled()
    expect(writeText).not.toHaveBeenCalled()
  })

  it('switching on reads once inside the click, and that read is only a baseline', async () => {
    const { session, status, rerender } = await mount('prompt', { enabled: false })
    fireEvent.click(screen.getByRole('button', { name: 'switch' }))
    expect(readText).toHaveBeenCalledTimes(1)
    expect(onEnabled).toHaveBeenCalledWith(true)
    rerender(<Harness session={session} enabled />)
    act(() => {
      status.change('granted')
    })
    await settle()
    await focusWindow()
    expect(session.pasted).toEqual([])
  })

  it('switching off does not read', async () => {
    await mount('prompt')
    fireEvent.click(screen.getByRole('button', { name: 'switch' }))
    expect(onEnabled).toHaveBeenCalledWith(false)
    expect(readText).not.toHaveBeenCalled()
  })
})
